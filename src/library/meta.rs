//! 그룹 메타(`<Group>/.writer.toml`) 읽기/쓰기와 시트 파일 만들기·순서 바꾸기.

use std::io;
use std::path::{Path, PathBuf};

use super::{GROUP_META_FILE, Group, sheet_file_name};

/// 그룹 폴더의 `.writer.toml` 스키마.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct GroupMeta {
    /// 아이콘 이름(예: inbox, folder, book).
    pub(super) icon: Option<String>,
    /// 표시 이름(없으면 폴더 이름).
    pub(super) title: Option<String>,
    /// 시트 파일명 순서. 나열되지 않은 파일은 뒤에 이름순으로 붙는다.
    pub(super) sheets: Vec<String>,
}

/// 그룹 폴더의 `.writer.toml` 을 읽는다. 없거나 깨지면 기본값.
pub(super) fn read_group_meta(dir: &Path) -> GroupMeta {
    std::fs::read_to_string(dir.join(GROUP_META_FILE))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// 그룹 메타 파일이 없으면 기본값으로 만든다(멱등).
pub(super) fn write_group_meta_if_absent(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::test_util::{stem, temp_dir};
    use crate::library::*;

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
}
