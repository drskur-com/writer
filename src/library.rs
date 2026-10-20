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

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

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

/// 시트 본문에서 얻은 목록용 미리보기.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    /// 표시 제목: 본문 첫 줄(헤딩 표시 제거). 빈 시트는 빈 문자열.
    pub title: String,
    /// 목록 카드에 보일 발췌: 제목 다음 줄들을 한 줄로 이은 것.
    pub excerpt: String,
}

impl Preview {
    /// 본문이 비어 있는지(공백·frontmatter뿐이어도 빈 것). 목록에서 "빈 시트"로 표시한다.
    pub fn is_empty(&self) -> bool {
        self.title.is_empty()
    }
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

/// 그룹 폴더의 `.writer.toml` 스키마.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct GroupMeta {
    /// 아이콘 이름(예: inbox, folder, book).
    icon: Option<String>,
    /// 표시 이름(없으면 폴더 이름).
    title: Option<String>,
    /// 시트 파일명 순서. 나열되지 않은 파일은 뒤에 이름순으로 붙는다.
    sheets: Vec<String>,
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

/// 레이아웃과 기본 메타를 보장한 뒤 라이브러리를 스캔한다.
pub fn open(root: &Path) -> io::Result<Library> {
    ensure_layout(root)?;
    write_default_meta(root)?;
    scan(root)
}

/// `.writer/library.toml` 마커를 보장한다(멱등).
fn write_default_meta(root: &Path) -> io::Result<()> {
    let dir = root.join(MARKER_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(MARKER_FILE);
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, "schema_version = 1\n")
}

/// 그룹 폴더의 `.writer.toml` 을 읽는다. 없거나 깨지면 기본값.
fn read_group_meta(dir: &Path) -> GroupMeta {
    std::fs::read_to_string(dir.join(GROUP_META_FILE))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// 그룹 메타 파일이 없으면 기본값으로 만든다(멱등).
fn write_group_meta_if_absent(
    dir: &Path,
    icon: Option<&str>,
    title: Option<&str>,
) -> io::Result<()> {
    let path = dir.join(GROUP_META_FILE);
    if path.exists() {
        return Ok(());
    }
    let mut table = toml::Table::new();
    if let Some(icon) = icon {
        table.insert("icon".to_string(), toml::Value::String(icon.to_string()));
    }
    if let Some(title) = title {
        table.insert("title".to_string(), toml::Value::String(title.to_string()));
    }
    let text = toml::to_string_pretty(&table)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    std::fs::write(path, text)
}

/// 그룹 안에서 시트를 `from` 위치에서 `to` 위치로 옮기고, 새 순서를 `.writer.toml`에 쓴다.
///
/// 범위를 벗어나면 아무것도 하지 않는다. 쓰기에 실패하면 메모리의 순서도 되돌린다.
pub fn move_sheet(group: &mut Group, from: usize, to: usize) -> io::Result<()> {
    if from >= group.sheets.len() || to >= group.sheets.len() || from == to {
        return Ok(());
    }
    let sheet = group.sheets.remove(from);
    group.sheets.insert(to, sheet);
    let order: Vec<String> = group.sheets.iter().map(sheet_file_name).collect();
    if let Err(err) = write_sheet_order(&group.path, &order) {
        let sheet = group.sheets.remove(to);
        group.sheets.insert(from, sheet);
        return Err(err);
    }
    Ok(())
}

/// 그룹 `.writer.toml`의 `sheets`를 `order`(파일명 목록)로 바꾼다. 다른 키는 그대로 둔다.
///
/// 메타 파일이 깨져 있으면 덮어쓰지 않고 오류를 돌려준다. 임시 파일에 쓴 뒤 바꿔치기한다.
fn write_sheet_order(group_dir: &Path, order: &[String]) -> io::Result<()> {
    let path = group_dir.join(GROUP_META_FILE);
    let mut table: toml::Table = match std::fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => toml::Table::new(),
        Err(err) => return Err(err),
    };
    table.insert(
        "sheets".to_string(),
        toml::Value::Array(order.iter().cloned().map(toml::Value::String).collect()),
    );
    let text = toml::to_string_pretty(&table)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let temp = group_dir.join(format!("{GROUP_META_FILE}.tmp"));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, &path)
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

/// `dir` 안에 빈 시트 파일(`<ULID>.md`)을 만들고 경로를 돌려준다.
///
/// 파일명은 바뀌지 않는 식별자다. 표시 제목은 본문에서 얻는다.
/// `create_new`로 만들어 기존 파일을 덮어쓰지 않는다.
pub fn create_sheet(dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.md", ulid::Ulid::generate()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    Ok(path)
}

/// 제목·발췌를 얻으려고 읽는 파일 앞부분 크기.
const PREVIEW_BYTES: u64 = 4096;

/// 발췌 최대 글자 수.
const EXCERPT_CHARS: usize = 200;

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

/// 시트 파일 앞부분을 읽어 미리보기를 만든다. 읽지 못하면 빈 미리보기.
pub fn read_preview(path: &Path) -> Preview {
    let (title, excerpt) = title_and_excerpt(&read_head(path));
    Preview { title, excerpt }
}

/// 파일 앞부분(`PREVIEW_BYTES`)을 UTF-8로 읽는다. 끝에서 잘린 글자는 버린다.
fn read_head(path: &Path) -> String {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(PREVIEW_BYTES).read_to_end(&mut bytes);
    }
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => {
            let valid = err.utf8_error().valid_up_to();
            let mut bytes = err.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).unwrap_or_default()
        }
    }
}

/// 본문에서 제목(첫 줄)과 발췌(그다음 줄들)를 뽑는다.
///
/// BOM과 앞쪽 YAML frontmatter(`---` … `---`/`...`)는 건너뛰고,
/// 각 줄의 ATX 헤딩 표시(`# `, 끝의 `#`)는 지운다. 빈 줄은 무시한다.
fn title_and_excerpt(text: &str) -> (String, String) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines().peekable();

    if lines.peek().is_some_and(|line| line.trim_end() == "---") {
        lines.next();
        for line in lines.by_ref() {
            if matches!(line.trim_end(), "---" | "...") {
                break;
            }
        }
    }

    let mut content = lines.map(strip_heading).filter(|line| !line.is_empty());
    let title = content.next().unwrap_or_default().to_string();

    let mut excerpt = String::new();
    for line in content {
        if !excerpt.is_empty() {
            excerpt.push(' ');
        }
        excerpt.push_str(line);
        if excerpt.chars().count() >= EXCERPT_CHARS {
            break;
        }
    }
    let excerpt = excerpt.chars().take(EXCERPT_CHARS).collect();
    (title, excerpt)
}

/// 앞뒤 공백과 ATX 헤딩 표시를 지운다. `#태그`처럼 뒤에 공백이 없으면 헤딩이 아니다.
fn strip_heading(line: &str) -> &str {
    let line = line.trim();
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = &line[hashes..];
        if rest.is_empty() {
            return "";
        }
        if rest.starts_with([' ', '\t']) {
            return rest.trim().trim_end_matches('#').trim_end();
        }
    }
    line
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

    fn stem(sheet: &Sheet) -> String {
        sheet
            .path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

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

    #[test]
    fn scan_reads_group_icon_from_meta() {
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
    fn scan_applies_group_meta_sheet_order() {
        let root = temp_dir("order");
        initialize(&root).unwrap();

        let inbox = root.join("Notes").join(DEFAULT_INBOX);
        for stem in ["a", "b", "c", "d"] {
            std::fs::write(inbox.join(format!("{stem}.md")), stem).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // 메타 순서: c, a (b·d는 미나열 -> 뒤에 최근 수정순: d, b)
        std::fs::write(
            inbox.join(GROUP_META_FILE),
            "icon = \"inbox\"\nsheets = [\"c.md\", \"a.md\"]\n",
        )
        .unwrap();

        let library = scan(&root).unwrap();
        let notes = library
            .sections
            .iter()
            .find(|section| section.kind == SectionKind::Notes)
            .unwrap();
        let group = notes
            .entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Group(group) if group.name == DEFAULT_INBOX => Some(group),
                _ => None,
            })
            .unwrap();
        let titles: Vec<_> = group.sheets.iter().map(stem).collect();
        assert_eq!(titles, vec!["c", "a", "d", "b"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn title_and_excerpt_from_content() {
        // 헤딩 표시 제거, 빈 줄 무시, 다음 줄들은 발췌로
        assert_eq!(
            title_and_excerpt("# 제목 #\n\n첫 문단\n둘째 줄\n"),
            ("제목".to_string(), "첫 문단 둘째 줄".to_string())
        );
        // 헤딩이 아니어도 첫 줄이 제목, `#태그`는 헤딩이 아니다
        assert_eq!(
            title_and_excerpt("  그냥 메모\n#태그"),
            ("그냥 메모".to_string(), "#태그".to_string())
        );
        // frontmatter와 BOM은 건너뛴다
        assert_eq!(
            title_and_excerpt("\u{feff}---\nkeywords: [a]\n---\n## 본문\n"),
            ("본문".to_string(), String::new())
        );
        // 공백·frontmatter뿐이면 빈 시트
        assert_eq!(title_and_excerpt(" \n\n"), (String::new(), String::new()));
        assert_eq!(
            title_and_excerpt("---\ngoal: 1\n---\n"),
            (String::new(), String::new())
        );
        // 발췌는 EXCERPT_CHARS에서 자른다
        let long = format!("t\n{}", "가".repeat(500));
        assert_eq!(title_and_excerpt(&long).1.chars().count(), EXCERPT_CHARS);
    }

    #[test]
    fn move_sheet_writes_order_and_keeps_meta() {
        let root = temp_dir("move");
        initialize(&root).unwrap();
        let inbox = root.join("Notes").join(DEFAULT_INBOX);
        for stem in ["a", "b", "c"] {
            std::fs::write(inbox.join(format!("{stem}.md")), stem).unwrap();
        }
        std::fs::write(
            inbox.join(GROUP_META_FILE),
            "icon = \"inbox\"\nsheets = [\"a.md\", \"b.md\", \"c.md\"]\n",
        )
        .unwrap();

        let mut group = scan_group(&inbox).unwrap();
        move_sheet(&mut group, 0, 2).unwrap();
        let moved: Vec<_> = group.sheets.iter().map(stem).collect();
        assert_eq!(moved, vec!["b", "c", "a"]);

        // 다시 읽어도 같은 순서, 아이콘은 그대로
        let rescanned = scan_group(&inbox).unwrap();
        let titles: Vec<_> = rescanned.sheets.iter().map(stem).collect();
        assert_eq!(titles, vec!["b", "c", "a"]);
        assert_eq!(rescanned.icon.as_deref(), Some("inbox"));
        assert!(!inbox.join(format!("{GROUP_META_FILE}.tmp")).exists());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn create_sheet_picks_unique_empty_file() {
        let root = temp_dir("create");
        initialize(&root).unwrap();
        let inbox = root.join("Notes").join(DEFAULT_INBOX);

        let first = create_sheet(&inbox).unwrap();
        let second = create_sheet(&inbox).unwrap();
        assert_ne!(first, second);
        for path in [&first, &second] {
            let stem = path.file_stem().unwrap().to_str().unwrap();
            assert!(stem.parse::<ulid::Ulid>().is_ok(), "{stem}");
        }

        assert!(read_preview(&first).is_empty());

        std::fs::remove_dir_all(&root).ok();
    }

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
