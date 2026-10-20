//! 라이브러리 도메인 모델과 폴더 검증/초기화/스캔.
//!
//! 레이아웃:
//! ```text
//! <LibraryRoot>/
//! ├─ .writer/library.toml          # 라이브러리 마커
//! ├─ Notes/<Group>/.writer.toml    # 그룹 메타(아이콘·시트 순서)
//! │  └─ <Sheet>.md
//! └─ Projects/<Group>/.writer.toml
//!    └─ <Sheet>.md
//! ```

mod layout;
mod meta;
mod preview;

pub use layout::{FolderKind, initialize, inspect};
pub use meta::{create_sheet, move_sheet};
pub use preview::{Preview, read_preview};

use std::io;
use std::path::{Path, PathBuf};

use layout::{ensure_layout, write_default_meta};
use meta::read_group_meta;

/// 라이브러리 표식 디렉터리/파일.
pub const MARKER_DIR: &str = ".writer";
pub const MARKER_FILE: &str = "library.toml";

/// 그룹 폴더 안의 메타 파일 이름.
pub const GROUP_META_FILE: &str = ".writer.toml";

/// 고정 섹션 순서.
pub const SECTIONS: [SectionKind; 2] = [SectionKind::Notes, SectionKind::Projects];

/// 메모 아래 기본 그룹 이름.
pub const DEFAULT_INBOX: &str = "Inbox";

/// 프로젝트 아래 기본(샘플) 그룹 이름.
pub const DEFAULT_PROJECT: &str = "샘플 프로젝트";

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
}

/// 프로젝트 안의 그룹(폴더).
#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    /// 그룹 디렉터리 경로 (CRUD는 후속 단계에서 사용).
    #[allow(dead_code)]
    pub path: PathBuf,
    /// 그룹 메타(`.writer.toml`)에 지정된 아이콘 이름(없으면 기본).
    pub icon: Option<String>,
    pub sheets: Vec<Sheet>,
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
            entries: scan_section(&root.join(kind.dir()))?,
        });
    }
    Ok(Library {
        root: root.to_path_buf(),
        sections,
    })
}

fn scan_section(dir: &Path) -> io::Result<Vec<Entry>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut sheets = Vec::new();
    let mut groups = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            groups.push(scan_group(&path)?);
        } else if is_markdown(&path) {
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

fn scan_group(dir: &Path) -> io::Result<Group> {
    let meta = read_group_meta(dir);

    let mut sheets = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && is_markdown(&path) {
            sheets.push(sheet_from_path(&path));
        }
    }

    // 메타에 나열된 파일을 그 순서대로 앞에 두고, 나머지는 최근 수정순으로 뒤에 붙인다.
    sort_recent_first(&mut sheets);
    if !meta.sheets.is_empty() {
        let mut ordered = Vec::with_capacity(sheets.len());
        for name in &meta.sheets {
            if let Some(index) = sheets
                .iter()
                .position(|sheet| sheet_file_name(sheet) == *name)
            {
                ordered.push(sheets.remove(index));
            }
        }
        ordered.append(&mut sheets);
        sheets = ordered;
    }

    Ok(Group {
        name: meta.title.unwrap_or_else(|| file_name(dir)),
        path: dir.to_path_buf(),
        icon: meta.icon,
        sheets,
    })
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
    Sheet {
        path: path.to_path_buf(),
        modified: std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|delta| delta.as_nanos())
            .unwrap_or(0),
    }
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
        // 샘플 프로젝트가 함께 있으므로 이름으로 찾는다.
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
}
