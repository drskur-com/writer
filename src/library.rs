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

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// 라이브러리 표식 디렉터리/파일.
pub const MARKER_DIR: &str = ".writer";
pub const MARKER_FILE: &str = "library.toml";

/// 고정 섹션 순서.
pub const SECTIONS: [SectionKind; 3] = [
    SectionKind::Inbox,
    SectionKind::Notes,
    SectionKind::Projects,
];

/// 라이브러리 최상위 섹션.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SectionKind {
    /// 빠른 캡처용 임시 시트.
    Inbox,
    /// 메모(평평, 최신순).
    Notes,
    /// 프로젝트(그룹 중첩, 수동 순서).
    Projects,
}

impl SectionKind {
    /// 물리 디렉터리 이름.
    pub fn dir(self) -> &'static str {
        match self {
            SectionKind::Inbox => "Inbox",
            SectionKind::Notes => "Notes",
            SectionKind::Projects => "Projects",
        }
    }

    /// 화면 표시 이름.
    pub fn title(self) -> &'static str {
        match self {
            SectionKind::Inbox => "Inbox",
            SectionKind::Notes => "Notes",
            SectionKind::Projects => "Projects",
        }
    }

    /// 화면 표시 아이콘.
    pub fn icon(self) -> &'static str {
        match self {
            SectionKind::Inbox => "📥",
            SectionKind::Notes => "📝",
            SectionKind::Projects => "📚",
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
    for kind in SECTIONS {
        std::fs::create_dir_all(root.join(kind.dir()))?;
    }
    let meta_dir = root.join(MARKER_DIR);
    std::fs::create_dir_all(&meta_dir)?;
    std::fs::write(meta_dir.join(MARKER_FILE), "schema_version = 1\n")
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
        if path.is_dir() {
            if kind == SectionKind::Projects {
                groups.push(scan_group(&path)?);
            }
        } else if is_markdown(&path) {
            sheets.push(sheet_from_path(&path));
        }
    }

    match kind {
        // 프로젝트: 그룹을 먼저, 각각 이름순. 시트도 이름순.
        SectionKind::Projects => {
            sheets.sort_by_key(|sheet| sheet.title.to_lowercase());
            groups.sort_by_key(|group| group.name.to_lowercase());
            let mut entries: Vec<Entry> = groups.into_iter().map(Entry::Group).collect();
            entries.extend(sheets.into_iter().map(Entry::Sheet));
            Ok(entries)
        }
        // 메모/인박스: 최근 수정순.
        _ => {
            sheets.sort_by_key(|sheet| std::cmp::Reverse(sheet.modified));
            Ok(sheets.into_iter().map(Entry::Sheet).collect())
        }
    }
}

fn scan_group(dir: &Path) -> io::Result<Group> {
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
        sheets,
    })
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
        assert_eq!(inspect(&root).unwrap(), FolderKind::Library);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn inspect_reports_empty_and_nested() {
        let root = temp_dir("inspect");
        initialize(&root).unwrap();

        // 라이브러리 내부의 폴더는 Nested
        assert_eq!(
            inspect(&root.join("Inbox")).unwrap_err(),
            InspectError::Nested
        );

        // 라이브러리 밖의 빈 폴더는 Empty
        let outside = temp_dir("outside");
        assert_eq!(inspect(&outside).unwrap(), FolderKind::Empty);

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
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
        assert!(
            matches!(projects.entries.first(), Some(Entry::Group(group)) if group.name == "Novel")
        );
        if let Some(Entry::Group(group)) = projects.entries.first() {
            let titles: Vec<_> = group.sheets.iter().map(|s| s.title.clone()).collect();
            assert_eq!(titles, vec!["01", "02"]);
        }

        std::fs::remove_dir_all(&root).ok();
    }
}
