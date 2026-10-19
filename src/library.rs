//! 라이브러리 도메인 모델과 폴더 검증/초기화/스캔.
//!
//! 레이아웃(ADR-0003):
//! ```text
//! <LibraryRoot>/
//! ├─ .writer/library.toml
//! ├─ Inbox/
//! ├─ Notes/
//! └─ Projects/<Project>/<Group>/<Sheet>.md
//! ```

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// 라이브러리 표식 디렉터리/파일.
pub const MARKER_DIR: &str = ".writer";
pub const MARKER_FILE: &str = "library.toml";

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
    /// 프로젝트(그룹, 시트는 이름순).
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

/// 문서(시트).
#[derive(Clone, Debug)]
pub struct Sheet {
    pub title: String,
    /// 시트 파일 경로 (열기/저장은 후속 단계에서 사용).
    #[allow(dead_code)]
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
    /// `library.toml` 에 지정된 아이콘 이름(없으면 기본).
    pub icon: Option<String>,
    pub sheets: Vec<Sheet>,
}

/// 폴더 검증 결과.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FolderKind {
    /// 이미 Writer 라이브러리(마커 존재).
    Library,
    /// 비어 있음.
    Empty,
    /// 마크다운 파일만 있음(확인 후 사용 가능).
    MarkdownOnly,
    /// 다른 내용이 있음(하위 폴더 생성 제안).
    NonEmpty,
}

/// 폴더 검증 실패.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InspectError {
    NotFound,
    NotADirectory,
    /// 선택 경로의 조상에 이미 라이브러리가 있음.
    Nested,
    NotWritable,
    Io(String),
}

impl fmt::Display for InspectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InspectError::NotFound => write!(f, "폴더를 찾을 수 없습니다"),
            InspectError::NotADirectory => write!(f, "폴더가 아닙니다"),
            InspectError::Nested => write!(f, "이미 다른 라이브러리 내부입니다"),
            InspectError::NotWritable => write!(f, "쓰기 권한이 없습니다"),
            InspectError::Io(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for InspectError {}

/// 경로가 라이브러리 표식(`.writer/library.toml`)을 가지는지.
pub fn has_marker(path: &Path) -> bool {
    path.join(MARKER_DIR).join(MARKER_FILE).is_file()
}

/// 후보 폴더를 검사해 어떤 종류인지 판정한다.
pub fn inspect(path: &Path) -> Result<FolderKind, InspectError> {
    let meta = std::fs::metadata(path).map_err(|_| InspectError::NotFound)?;
    if !meta.is_dir() {
        return Err(InspectError::NotADirectory);
    }
    if has_marker(path) {
        return Ok(FolderKind::Library);
    }
    if path.ancestors().skip(1).any(has_marker) {
        return Err(InspectError::Nested);
    }
    if !is_writable(path) {
        return Err(InspectError::NotWritable);
    }

    let mut any = false;
    let mut has_markdown = false;
    for entry in std::fs::read_dir(path).map_err(|err| InspectError::Io(err.to_string()))? {
        let entry = entry.map_err(|err| InspectError::Io(err.to_string()))?;
        any = true;
        if is_markdown(&entry.path()) {
            has_markdown = true;
        }
    }

    Ok(if !any {
        FolderKind::Empty
    } else if has_markdown {
        FolderKind::MarkdownOnly
    } else {
        FolderKind::NonEmpty
    })
}

/// 라이브러리 골격(섹션 폴더 + 마커)을 만든다.
///
/// 반쪽 라이브러리를 피하려고 마커 파일을 마지막에 쓴다.
pub fn initialize(root: &Path) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    ensure_layout(root)?;
    write_default_meta(root)
}

/// 라이브러리 레이아웃을 최신 상태로 보장한다(멱등).
///
/// - `Notes/`·`Projects/` 폴더를 보장한다.
/// - 구버전 최상위 `Inbox/` 를 `Notes/Inbox/` 로 이관한다.
/// - `Projects/` 가 비어 있으면 `샘플 프로젝트/` 를 만든다.
pub fn ensure_layout(root: &Path) -> io::Result<()> {
    for kind in SECTIONS {
        std::fs::create_dir_all(root.join(kind.dir()))?;
    }

    // 구버전 최상위 Inbox -> Notes/Inbox 이관
    let legacy_inbox = root.join("Inbox");
    let notes_inbox = root.join(SectionKind::Notes.dir()).join(DEFAULT_INBOX);
    if legacy_inbox.is_dir() {
        std::fs::create_dir_all(&notes_inbox)?;
        for entry in std::fs::read_dir(&legacy_inbox)? {
            let entry = entry?;
            let dest = notes_inbox.join(entry.file_name());
            if !dest.exists() {
                std::fs::rename(entry.path(), &dest)?;
            }
        }
        let _ = std::fs::remove_dir(&legacy_inbox);
    }
    std::fs::create_dir_all(&notes_inbox)?;

    // 프로젝트가 하나도 없으면 샘플 생성
    let projects_dir = root.join(SectionKind::Projects.dir());
    if is_empty_dir(&projects_dir)? {
        std::fs::create_dir_all(projects_dir.join(DEFAULT_PROJECT))?;
    }

    Ok(())
}

fn is_empty_dir(dir: &Path) -> io::Result<bool> {
    if !dir.is_dir() {
        return Ok(true);
    }
    Ok(std::fs::read_dir(dir)?.next().is_none())
}

/// 레이아웃과 기본 메타를 보장한 뒤 라이브러리를 스캔한다.
pub fn open(root: &Path) -> io::Result<Library> {
    ensure_layout(root)?;
    write_default_meta(root)?;
    scan(root)
}

/// `.writer/library.toml` 마커를 보장하고 기본 노드 메타를 채운다(멱등).
///
/// 기존 파일이 있으면 보존하며 필요한 키만 추가한다.
fn write_default_meta(root: &Path) -> io::Result<()> {
    let dir = root.join(MARKER_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(MARKER_FILE);

    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default();
    if !table.contains_key("schema_version") {
        table.insert("schema_version".to_string(), toml::Value::Integer(1));
    }
    if !table.contains_key("nodes") {
        table.insert("nodes".to_string(), toml::Value::Table(toml::Table::new()));
    }
    if let Some(nodes) = table.get_mut("nodes").and_then(|value| value.as_table_mut()) {
        let inbox_key = format!("{}/{}", SectionKind::Notes.dir(), DEFAULT_INBOX);
        if !nodes.contains_key(&inbox_key) {
            let mut entry = toml::Table::new();
            entry.insert("icon".to_string(), toml::Value::String("inbox".to_string()));
            nodes.insert(inbox_key, toml::Value::Table(entry));
        }
    }

    let text = toml::to_string_pretty(&table)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    std::fs::write(path, text)
}

/// 라이브러리를 스캔해 모델로 읽는다.
pub fn scan(root: &Path) -> io::Result<Library> {
    let icons = load_node_icons(root);
    let mut sections = Vec::with_capacity(SECTIONS.len());
    for kind in SECTIONS {
        sections.push(Section {
            kind,
            entries: scan_section(root, &root.join(kind.dir()), kind, &icons)?,
        });
    }
    Ok(Library {
        root: root.to_path_buf(),
        sections,
    })
}

fn scan_section(
    root: &Path,
    dir: &Path,
    kind: SectionKind,
    icons: &HashMap<String, String>,
) -> io::Result<Vec<Entry>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut sheets = Vec::new();
    let mut groups = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            groups.push(scan_group(root, &path, icons)?);
        } else if is_markdown(&path) {
            sheets.push(sheet_from_path(&path));
        }
    }

    // 그룹을 먼저(이름순), 그 뒤 시트.
    groups.sort_by_key(|group| group.name.to_lowercase());
    match kind {
        SectionKind::Projects => sheets.sort_by_key(|sheet| sheet.title.to_lowercase()),
        SectionKind::Notes => sheets.sort_by_key(|sheet| std::cmp::Reverse(sheet.modified)),
    }
    let mut entries: Vec<Entry> = groups.into_iter().map(Entry::Group).collect();
    entries.extend(sheets.into_iter().map(Entry::Sheet));
    Ok(entries)
}

fn scan_group(
    root: &Path,
    dir: &Path,
    icons: &HashMap<String, String>,
) -> io::Result<Group> {
    let mut sheets = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && is_markdown(&path) {
            sheets.push(sheet_from_path(&path));
        }
    }
    sheets.sort_by_key(|sheet| sheet.title.to_lowercase());
    Ok(Group {
        name: file_name(dir),
        path: dir.to_path_buf(),
        icon: icons.get(&relative_key(root, dir)).cloned(),
        sheets,
    })
}

/// 라이브러리 루트 기준 상대 경로를 `/` 구분자 키로 만든다.
fn relative_key(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// `.writer/library.toml` 의 `[nodes]` 아이콘 매핑(키: 상대 경로)을 읽는다.
fn load_node_icons(root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let text = match std::fs::read_to_string(root.join(MARKER_DIR).join(MARKER_FILE)) {
        Ok(text) => text,
        Err(_) => return map,
    };
    let table: toml::Table = match toml::from_str(&text) {
        Ok(table) => table,
        Err(_) => return map,
    };
    let Some(nodes) = table.get("nodes").and_then(|value| value.as_table()) else {
        return map;
    };
    for (key, value) in nodes {
        if let Some(icon) = value.get("icon").and_then(|value| value.as_str()) {
            map.insert(key.clone(), icon.to_string());
        }
    }
    map
}

fn sheet_from_path(path: &Path) -> Sheet {
    Sheet {
        title: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
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

/// 임시 파일을 만들어 실제 쓰기 가능 여부를 확인한다.
fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".writer-write-probe");
    let result = std::fs::write(&probe, b"");
    if result.is_ok() {
        let _ = std::fs::remove_file(&probe);
    }
    result.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
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

    #[test]
    fn initialize_creates_sections_and_marker() {
        let root = temp_dir("init");
        initialize(&root).unwrap();

        assert!(has_marker(&root));
        for kind in SECTIONS {
            assert!(root.join(kind.dir()).is_dir(), "{:?}", kind);
        }
        assert!(root.join("Notes").join(DEFAULT_INBOX).is_dir());
        assert!(root.join("Projects").join(DEFAULT_PROJECT).is_dir());
        assert_eq!(inspect(&root).unwrap(), FolderKind::Library);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ensure_layout_migrates_legacy_inbox() {
        let root = temp_dir("migrate");
        initialize(&root).unwrap();

        // 구버전 형태로 되돌린다: Notes/Inbox 를 없애고 최상위 Inbox 를 만든다.
        std::fs::remove_dir_all(root.join("Notes").join(DEFAULT_INBOX)).unwrap();
        std::fs::create_dir_all(root.join("Inbox")).unwrap();
        std::fs::write(root.join("Inbox").join("note.md"), "n").unwrap();

        ensure_layout(&root).unwrap();

        assert!(root.join("Notes").join(DEFAULT_INBOX).join("note.md").is_file());
        assert!(!root.join("Inbox").exists());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn inspect_reports_empty_and_nested() {
        let root = temp_dir("inspect");
        initialize(&root).unwrap();

        // 라이브러리 내부의 폴더는 Nested
        assert_eq!(
            inspect(&root.join("Notes")).unwrap_err(),
            InspectError::Nested
        );

        // 라이브러리 밖의 빈 폴더는 Empty
        let outside = temp_dir("outside");
        assert_eq!(inspect(&outside).unwrap(), FolderKind::Empty);

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn scan_reads_icon_from_library_toml() {
        let root = temp_dir("icon");
        initialize(&root).unwrap();

        let library = scan(&root).unwrap();
        let notes = library
            .sections
            .iter()
            .find(|section| section.kind == SectionKind::Notes)
            .unwrap();
        let inbox = notes
            .entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Group(group) if group.name == DEFAULT_INBOX => Some(group),
                _ => None,
            })
            .expect("Inbox 그룹이 있어야 한다");
        assert_eq!(inbox.icon.as_deref(), Some("inbox"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_sorts_notes_by_modified_and_projects_by_name() {
        let root = temp_dir("scan");
        initialize(&root).unwrap();

        // 메모 2개: b가 더 최근
        std::fs::write(root.join("Notes").join("a.md"), "a").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(root.join("Notes").join("b.md"), "b").unwrap();

        // 프로젝트: 그룹 + 시트
        let project = root.join("Projects").join("Novel");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("02.md"), "2").unwrap();
        std::fs::write(project.join("01.md"), "1").unwrap();

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
                Entry::Sheet(sheet) => Some(sheet.title.clone()),
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
        let titles: Vec<_> = novel.sheets.iter().map(|s| s.title.clone()).collect();
        assert_eq!(titles, vec!["01", "02"]);

        std::fs::remove_dir_all(&root).ok();
    }
}
