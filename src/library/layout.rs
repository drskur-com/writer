//! 라이브러리 폴더 검증·초기화·레이아웃 보장.

use std::fmt;
use std::io;
use std::path::Path;

use super::meta::write_group_meta_if_absent;
use super::{
    DEFAULT_INBOX, DEFAULT_PROJECT, MARKER_DIR, MARKER_FILE, SECTIONS, SectionKind, is_markdown,
};

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
    write_group_meta_if_absent(&notes_inbox, Some("inbox"), None)?;

    // 프로젝트가 하나도 없으면 샘플 생성
    let projects_dir = root.join(SectionKind::Projects.dir());
    if is_empty_dir(&projects_dir)? {
        let sample = projects_dir.join(DEFAULT_PROJECT);
        std::fs::create_dir_all(&sample)?;
        write_group_meta_if_absent(&sample, Some("folder"), None)?;
    }

    Ok(())
}

fn is_empty_dir(dir: &Path) -> io::Result<bool> {
    if !dir.is_dir() {
        return Ok(true);
    }
    Ok(std::fs::read_dir(dir)?.next().is_none())
}

/// `.writer/library.toml` 마커를 보장한다(멱등).
pub(super) fn write_default_meta(root: &Path) -> io::Result<()> {
    let dir = root.join(MARKER_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(MARKER_FILE);
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, "schema_version = 1\n")
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
    use crate::library::test_util::temp_dir;

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

        assert!(
            root.join("Notes")
                .join(DEFAULT_INBOX)
                .join("note.md")
                .is_file()
        );
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
}
