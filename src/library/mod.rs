//! 라이브러리 도메인 모델과 폴더 검증/초기화/스캔.
//!
//! 레이아웃:
//! ```text
//! <LibraryRoot>/
//! ├─ .writer/library.toml          # 라이브러리 마커
//! ├─ Notes/<Group>/.writer.toml    # 그룹 메타(아이콘·시트 순서)
//! │  └─ <Sheet>.md
//! └─ Projects/<Project>/.writer.toml
//!    ├─ <Sheet>.md
//!    ├─ <Group>/…                  # 원고 그룹(하위 그룹 가능, 빌드 대상)
//!    └─ Extras/<Group>/…           # 추가자료(빌드에서 빠짐)
//!       └─ Attachments/…         # 첨부파일: cover.<확장자>, <ULID>.<확장자>
//! ```

mod book;
mod document;
mod files;
mod layout;
mod manuscript;
mod meta;
mod preview;
mod style;

pub use book::{
    BookMeta, COVER_EXTENSIONS, COVER_STEM, attach_to_sheet, read_book, set_cover, write_book,
};
pub use document::{count_words, read_sheet, write_sheet};
pub use files::{FileItem, FileKind, GroupKind, format_size, move_file};
pub use layout::{FolderKind, initialize, inspect};
pub use manuscript::{ManuscriptSheet, manuscript_sheets};
pub use meta::{
    create_group, create_project, create_sheet, glue_sheets, move_sheet, move_to_trash, set_sort,
    set_sort_descending, unglue_sheets, update_group, update_project,
};
pub use preview::{Preview, read_preview};
pub use style::{
    Align, FONT_WEIGHTS, Numbering, PREFIX_NUMBER, Placement, ResolvedStyle, StyleSpec,
    StyleTarget, Styles, VerticalAlign, import_styles, read_styles, write_styles,
};

use std::io;
use std::path::{Path, PathBuf};

use layout::{ensure_layout, write_default_meta};
use meta::{GroupMeta, read_group_meta};

/// 라이브러리 표식 디렉터리/파일.
pub const MARKER_DIR: &str = ".writer";
pub const MARKER_FILE: &str = "library.toml";

/// 그룹 폴더 안의 메타 파일 이름.
pub const GROUP_META_FILE: &str = ".writer.toml";

/// 고정 섹션 순서.
pub const SECTIONS: [SectionKind; 2] = [SectionKind::Notes, SectionKind::Projects];

/// 메모 아래 기본 그룹 이름.
pub const DEFAULT_INBOX: &str = "Inbox";

/// 프로젝트 안의 추가자료 폴더. 원고(빌드 대상)와 따로 두며, 그룹 이름으로 쓸 수 없다.
pub const EXTRAS_DIR: &str = "Extras";

/// 새 프로젝트의 기본 추가자료 그룹.
pub const TEMPLATES_GROUP: &str = "Templates";

/// 첨부파일(표지 등) 추가자료 그룹. 처음 첨부할 때 만든다.
pub const ATTACHMENTS_GROUP: &str = "Attachments";

/// 라이브러리 최상위 섹션.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SectionKind {
    /// 메모(그룹 가능, 시트는 최근 수정순).
    Notes,
    /// 프로젝트(그룹, 시트는 `.writer.toml` 순서 → 최근 수정순).
    Projects,
}

impl SectionKind {
    /// 물리 디렉터리 이름.
    pub fn dir(self) -> &'static str {
        match self {
            SectionKind::Notes => "Notes",
            SectionKind::Projects => "Projects",
        }
    }

    /// 화면 표시 이름.
    pub fn title(self) -> &'static str {
        match self {
            SectionKind::Notes => "메모",
            SectionKind::Projects => "프로젝트",
        }
    }
}

/// 그룹의 시트 정렬 방법. 그룹 메타(`sort`)에 저장한다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortMode {
    /// 끌어서 정한 순서(메타 `sheets`).
    Manual,
    /// 제목(본문 첫 줄) 가나다순.
    Title,
    /// 만든 시각, 최근 것이 위.
    Created,
    /// 수정 시각(파일 메타데이터), 최근 것이 위.
    Modified,
}

impl SortMode {
    /// 메뉴에 보이는 순서.
    pub const ALL: [SortMode; 4] = [
        SortMode::Manual,
        SortMode::Title,
        SortMode::Modified,
        SortMode::Created,
    ];

    /// 방향을 따로 정하지 않았을 때 내림차순인지. 날짜는 최신이 위, 제목은 ㄱ부터.
    pub fn default_descending(self) -> bool {
        matches!(self, SortMode::Created | SortMode::Modified)
    }

    /// 방향 메뉴 이름: (오름차순, 내림차순). 수동은 방향이 없다.
    pub fn direction_titles(self) -> Option<(&'static str, &'static str)> {
        match self {
            SortMode::Manual => None,
            SortMode::Title => Some(("ㄱ부터 ㅎ까지", "ㅎ부터 ㄱ까지")),
            SortMode::Created | SortMode::Modified => Some(("오래된 항목부터", "최신 항목부터")),
        }
    }

    /// 메타 파일에 쓰는 이름.
    pub fn key(self) -> &'static str {
        match self {
            SortMode::Manual => "manual",
            SortMode::Title => "title",
            SortMode::Created => "created",
            SortMode::Modified => "modified",
        }
    }

    /// 메타 이름을 읽는다. 모르는 이름이면 `None`(기본 정렬).
    pub fn from_key(key: &str) -> Option<SortMode> {
        SortMode::ALL.into_iter().find(|mode| mode.key() == key)
    }

    /// 화면 표시 이름.
    pub fn title(self) -> &'static str {
        match self {
            SortMode::Manual => "수동",
            SortMode::Title => "제목순으로",
            SortMode::Created => "생성일순으로",
            SortMode::Modified => "수정일순으로",
        }
    }
}

/// 열린 라이브러리.
#[derive(Clone, Debug)]
pub struct Library {
    pub root: PathBuf,
    pub sections: Vec<Section>,
}

/// 섹션과 그 항목(시트/그룹).
#[derive(Clone, Debug)]
pub struct Section {
    pub kind: SectionKind,
    pub entries: Vec<Entry>,
}

/// 섹션의 항목.
#[derive(Clone, Debug)]
pub enum Entry {
    Sheet(Sheet),
    Group(Group),
}

/// 문서(시트). 스캔은 파일 메타데이터만 읽는다. 본문 미리보기는 [`read_preview`]로 따로 읽는다.
#[derive(Clone, Debug)]
pub struct Sheet {
    /// 시트 파일 경로.
    pub path: PathBuf,
    /// 정렬용 수정 시각(나노초). 최근일수록 큼.
    pub modified: u128,
    /// 만든 시각(나노초). 파일명이 ULID면 거기서, 아니면(밖에서 가져온 파일) 파일 메타데이터에서 얻는다.
    pub created: u128,
    /// 앞 시트에 접합됐는지(그룹 메타 `glued`). 접합한 시트는 프리뷰·내보내기에서 앞 시트와 하나로 이어진다.
    /// 그룹의 첫 시트에는 의미가 없다.
    pub glued: bool,
}

/// 그룹(폴더). 프로젝트도 `Projects/` 바로 아래의 그룹이다.
#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    /// 그룹 디렉터리 경로.
    pub path: PathBuf,
    /// 그룹 메타(`.writer.toml`)에 지정된 아이콘 이름(없으면 기본).
    pub icon: Option<String>,
    /// 그룹 메타에 지정된 색 이름(없으면 기본 아이콘 색).
    pub color: Option<String>,
    /// 그룹 메타에 지정된 정렬 방법(없으면 섹션 기본).
    pub sort: Option<SortMode>,
    /// 그룹 메타에 지정된 정렬 방향(없으면 정렬 방법의 기본 방향).
    pub descending: Option<bool>,
    /// 그룹이 담는 것(메타 `kind`). 파일 그룹은 시트 대신 `files`를 읽는다.
    pub kind: GroupKind,
    /// 시트. 수동 순서(메타 `sheets` → 나머지는 최근 수정순)로 읽는다. 다른 정렬은 화면에서 한다.
    pub sheets: Vec<Sheet>,
    /// 파일 그룹의 파일(메타 `files` 순서 → 이름순).
    pub files: Vec<FileItem>,
    /// 하위 그룹(메타 `groups` 순서 → 이름순).
    pub groups: Vec<Group>,
    /// 추가자료 그룹(프로젝트만, `<Project>/Extras/<Group>`). 원고 빌드에서 빠진다.
    pub extras: Vec<Group>,
}

impl Group {
    /// 이 그룹과 모든 하위·추가자료 그룹(깊이 우선).
    pub fn descendants(&self) -> Vec<&Group> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a Group>) {
        out.push(self);
        for child in self.groups.iter().chain(&self.extras) {
            child.collect(out);
        }
    }

    /// 이 그룹이나 그 아래에서 경로로 그룹을 찾는다.
    pub fn find_mut(&mut self, path: &Path) -> Option<&mut Group> {
        if self.path == path {
            return Some(self);
        }
        self.groups
            .iter_mut()
            .chain(&mut self.extras)
            .find_map(|child| child.find_mut(path))
    }
}

impl Section {
    /// 섹션의 모든 그룹(하위·추가자료 포함).
    pub fn groups(&self) -> Vec<&Group> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Group(group) => Some(group.descendants()),
                Entry::Sheet(_) => None,
            })
            .flatten()
            .collect()
    }

    /// 섹션의 모든 시트(그룹 안 포함).
    pub fn sheets(&self) -> Vec<&Sheet> {
        let loose = self.entries.iter().filter_map(|entry| match entry {
            Entry::Sheet(sheet) => Some(sheet),
            Entry::Group(_) => None,
        });
        loose
            .chain(self.groups().into_iter().flat_map(|group| &group.sheets))
            .collect()
    }
}

impl Library {
    /// 섹션을 찾는다.
    pub fn section(&self, kind: SectionKind) -> Option<&Section> {
        self.sections.iter().find(|section| section.kind == kind)
    }

    /// 모든 섹션에서 경로로 그룹을 찾는다(하위·추가자료 포함).
    pub fn find_group(&self, path: &Path) -> Option<&Group> {
        self.sections
            .iter()
            .flat_map(Section::groups)
            .find(|group| group.path == path)
    }

    pub fn find_group_mut(&mut self, path: &Path) -> Option<&mut Group> {
        self.sections
            .iter_mut()
            .flat_map(|section| &mut section.entries)
            .find_map(|entry| match entry {
                Entry::Group(group) => group.find_mut(path),
                Entry::Sheet(_) => None,
            })
    }

    /// 경로(시트·그룹)가 들어 있는 프로젝트 폴더(`Projects/<Project>`). 프로젝트 밖이면 `None`.
    pub fn project_of(&self, path: &Path) -> Option<PathBuf> {
        let projects = self.root.join(SectionKind::Projects.dir());
        let name = path.strip_prefix(&projects).ok()?.components().next()?;
        Some(projects.join(name))
    }

    /// `Projects/` 바로 아래의 프로젝트를 찾는다.
    pub fn find_project(&self, path: &Path) -> Option<&Group> {
        self.section(SectionKind::Projects)?
            .entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Group(group) if group.path == path => Some(group),
                _ => None,
            })
    }
}

/// 레이아웃과 기본 메타를 보장한 뒤 라이브러리를 스캔한다.
pub fn open(root: &Path) -> io::Result<Library> {
    ensure_layout(root)?;
    write_default_meta(root)?;
    scan(root)
}

/// 라이브러리를 스캔해 모델로 읽는다.
pub fn scan(root: &Path) -> io::Result<Library> {
    let mut sections = Vec::with_capacity(SECTIONS.len());
    for kind in SECTIONS {
        sections.push(Section {
            kind,
            entries: scan_section(&root.join(kind.dir()), kind)?,
        });
    }
    Ok(Library {
        root: root.to_path_buf(),
        sections,
    })
}

fn scan_section(dir: &Path, kind: SectionKind) -> io::Result<Vec<Entry>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut sheets = Vec::new();
    let mut groups = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if is_group_dir(&path) {
            groups.push(scan_group(&path, kind == SectionKind::Projects)?);
        } else if path.is_file() && is_markdown(&path) {
            sheets.push(sheet_from_path(&path));
        }
    }

    // 그룹을 먼저(이름순), 그 뒤 시트(최근 수정순).
    groups.sort_by_key(|group| group.name.to_lowercase());
    sort_recent_first(&mut sheets);
    let mut entries: Vec<Entry> = groups.into_iter().map(Entry::Group).collect();
    entries.extend(sheets.into_iter().map(Entry::Sheet));
    Ok(entries)
}

/// 그룹 폴더를 하위 그룹까지 읽는다. 프로젝트(`project`)면 안의 `Extras/`를 추가자료로 읽는다.
fn scan_group(dir: &Path, project: bool) -> io::Result<Group> {
    let meta = read_group_meta(dir);
    let kind = GroupKind::from_key(meta.kind.as_deref());

    let mut sheets = Vec::new();
    let mut groups = Vec::new();
    let mut extras = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if project && path.file_name().is_some_and(|name| name == EXTRAS_DIR) && path.is_dir() {
            extras = scan_subgroups(&path, &read_group_meta(&path))?;
        } else if is_group_dir(&path) {
            groups.push(scan_group(&path, false)?);
        } else if kind == GroupKind::Sheets && path.is_file() && is_markdown(&path) {
            sheets.push(sheet_from_path(&path));
        }
    }

    // 메타에 나열된 파일을 그 순서대로 앞에 두고, 나머지는 최근 수정순으로 뒤에 붙인다.
    sort_recent_first(&mut sheets);
    order_by_names(&mut sheets, &meta.sheets, sheet_file_name);
    for sheet in &mut sheets {
        sheet.glued = meta.glued.contains(&sheet_file_name(sheet));
    }
    sort_groups(&mut groups, &meta);
    let files = match kind {
        GroupKind::Files => files::scan_files(dir, &meta.files)?,
        GroupKind::Sheets => Vec::new(),
    };

    Ok(Group {
        name: meta.title.unwrap_or_else(|| file_name(dir)),
        path: dir.to_path_buf(),
        icon: meta.icon,
        color: meta.color,
        sort: meta.sort.as_deref().and_then(SortMode::from_key),
        descending: meta.descending,
        kind,
        sheets,
        files,
        groups,
        extras,
    })
}

/// `dir` 바로 아래의 그룹 폴더들을 읽는다(시트는 무시). 순서는 `meta`의 `groups`를 따른다.
fn scan_subgroups(dir: &Path, meta: &GroupMeta) -> io::Result<Vec<Group>> {
    let mut groups = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if is_group_dir(&path) {
            groups.push(scan_group(&path, false)?);
        }
    }
    sort_groups(&mut groups, meta);
    Ok(groups)
}

/// 하위 그룹 정렬: 메타 `groups`(폴더 이름)에 나열된 순서 → 나머지는 이름순.
fn sort_groups(groups: &mut Vec<Group>, meta: &GroupMeta) {
    groups.sort_by_key(|group| group.name.to_lowercase());
    order_by_names(groups, &meta.groups, |group| file_name(&group.path));
}

/// `names`에 나열된 항목을 그 순서대로 앞으로 옮긴다. 나머지는 원래 순서로 뒤에 둔다.
fn order_by_names<T>(items: &mut Vec<T>, names: &[String], key: impl Fn(&T) -> String) {
    if names.is_empty() {
        return;
    }
    let mut ordered = Vec::with_capacity(items.len());
    for name in names {
        if let Some(index) = items.iter().position(|item| key(item) == *name) {
            ordered.push(items.remove(index));
        }
    }
    ordered.append(items);
    *items = ordered;
}

/// 그룹으로 읽을 폴더인지. 숨김 폴더와 첨부 폴더(`*.assets`)는 뺀다.
fn is_group_dir(path: &Path) -> bool {
    let name = file_name(path);
    path.is_dir() && !name.starts_with('.') && !name.ends_with(".assets")
}

/// 기본 정렬: 최근 수정순. 같으면 파일명 역순(ULID 파일명은 최근 생성이 앞).
fn sort_recent_first(sheets: &mut [Sheet]) {
    sheets.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| sheet_file_name(b).cmp(&sheet_file_name(a)))
    });
}

/// 시트의 파일명(확장자 포함).
fn sheet_file_name(sheet: &Sheet) -> String {
    sheet
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sheet_from_path(path: &Path) -> Sheet {
    let modified = modified_nanos(path);
    Sheet {
        path: path.to_path_buf(),
        modified,
        created: created_nanos(path).unwrap_or(modified),
        glued: false,
    }
}

/// 시트를 만든 시각(나노초).
///
/// 파일명이 ULID면 그 시각을 쓴다. 복사·동기화·git을 거쳐도 파일명은 남기 때문이다.
/// 아니면 파일 시스템의 생성 시각을 쓰고, 그것도 없으면(일부 파일 시스템) `None`.
fn created_nanos(path: &Path) -> Option<u128> {
    let from_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.parse::<ulid::Ulid>().ok())
        .map(|id| u128::from(id.timestamp_ms()) * 1_000_000);
    from_name.or_else(|| {
        std::fs::metadata(path)
            .and_then(|meta| meta.created())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|delta| delta.as_nanos())
    })
}

/// 파일 수정 시각(나노초). 읽지 못하면 0.
pub fn modified_nanos(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|delta| delta.as_nanos())
        .unwrap_or(0)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

#[cfg(test)]
mod test_util {
    use super::*;

    pub fn stem(sheet: &Sheet) -> String {
        sheet
            .path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    pub fn temp_dir(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "writer-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::test_util::{stem, temp_dir};

    #[test]
    fn scan_sorts_sheets_recent_first() {
        let root = temp_dir("scan");
        initialize(&root).unwrap();

        // 메모 2개: b가 더 최근
        std::fs::write(root.join("Notes").join("a.md"), "a").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(root.join("Notes").join("b.md"), "b").unwrap();

        // 프로젝트 그룹(메타 순서 없음): 나중에 쓴 01이 앞
        let project = root.join("Projects").join("Novel");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("02.md"), "02").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(project.join("01.md"), "01").unwrap();

        let library = scan(&root).unwrap();

        let notes = library
            .sections
            .iter()
            .find(|section| section.kind == SectionKind::Notes)
            .unwrap();
        let note_titles: Vec<_> = notes
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Sheet(sheet) => Some(stem(sheet)),
                _ => None,
            })
            .collect();
        assert_eq!(note_titles, vec!["b", "a"]);

        let projects = library
            .sections
            .iter()
            .find(|section| section.kind == SectionKind::Projects)
            .unwrap();
        let novel = projects
            .entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Group(group) if group.name == "Novel" => Some(group),
                _ => None,
            })
            .expect("Novel 그룹이 있어야 한다");
        let titles: Vec<_> = novel.sheets.iter().map(stem).collect();
        assert_eq!(titles, vec!["01", "02"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn created_comes_from_ulid_name_or_file_metadata() {
        let root = temp_dir("created");
        let id = ulid::Ulid::from_parts(1_700_000_000_000, 7);
        let named = root.join(format!("{id}.md"));
        let imported = root.join("가져온 글.md");
        std::fs::write(&named, "a").unwrap();
        std::fs::write(&imported, "b").unwrap();

        // ULID 파일명이면 파일 시각과 상관없이 ID의 시각
        assert_eq!(
            sheet_from_path(&named).created,
            1_700_000_000_000 * 1_000_000
        );
        // 아니면 파일 메타데이터(생성 시각, 없으면 수정 시각)
        let sheet = sheet_from_path(&imported);
        assert!(sheet.created > 1_700_000_000_000 * 1_000_000);
        assert!(sheet.created <= sheet.modified);

        std::fs::remove_dir_all(&root).ok();
    }
}
