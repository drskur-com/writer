//! 메모의 할 일 색인(ADR-0024).
//!
//! 원본은 메모 원문이다. 색인은 메모(`Notes/`)의 시트를 훑어 할 일과 그 원문 위치를 모은 것이고, 시작을 빠르게 하려고
//! 로컬 캐시(`crate::cache::Cache`, redb)에 메모 파일마다 한 항목씩 둔다. 수정 시각·크기가 같은 파일은 다시 읽지 않고,
//! 바뀐 파일 몫만 캐시에 쓴다.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use redb::{ReadableDatabase as _, ReadableTable as _, TableDefinition};
use serde::{Deserialize, Serialize};

use super::{SectionKind, is_markdown};
use crate::cache::Cache;
use crate::markdown::{self, Todo};

/// 메모 파일(라이브러리 기준 상대 경로) → 그 파일의 색인 항목(JSON).
const TODOS: TableDefinition<&str, &[u8]> = TableDefinition::new("todos");
/// 색인 정보. `parser`: 할 일을 읽는 규칙의 버전.
const TODOS_META: TableDefinition<&str, u64> = TableDefinition::new("todos_meta");

/// 할 일 읽기 규칙(`markdown::todos`)이나 항목 형식을 바꾸면 올린다. 다르면 캐시를 버리고 다시 읽는다.
const PARSER_VERSION: u64 = 1;

/// 메모 파일 하나의 색인.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileEntry {
    /// 수정 시각(나노초).
    modified: u64,
    size: u64,
    todos: Vec<Todo>,
}

pub struct TodoIndex {
    root: PathBuf,
    /// 라이브러리 기준 상대 경로(`/`로 이음) → 색인. 할 일이 없는 파일도 둔다(다시 읽지 않도록).
    files: BTreeMap<String, FileEntry>,
    cache: Option<Cache>,
}

impl TodoIndex {
    /// 캐시를 읽고 메모를 훑어 색인을 만든다. 바뀐 파일만 다시 읽고, 바뀐 몫을 캐시에 쓴다.
    pub fn load(root: &Path, cache: Option<Cache>) -> Self {
        let mut index = Self {
            root: root.to_path_buf(),
            files: cache.as_ref().map(read_cache).unwrap_or_default(),
            cache,
        };
        let mut seen = Vec::new();
        collect_markdown(&root.join(SectionKind::Notes.dir()), &mut seen);
        let present: std::collections::BTreeSet<String> =
            seen.iter().filter_map(|path| index.key(path)).collect();
        let removed: Vec<String> = index
            .files
            .keys()
            .filter(|key| !present.contains(*key))
            .cloned()
            .collect();
        for key in &removed {
            index.files.remove(key);
        }
        let changed: Vec<String> = seen
            .iter()
            .filter_map(|path| index.reread(path))
            .collect();
        index.write_cache(&changed, &removed);
        index
    }

    /// 바뀐 경로(폴더 감시)를 다시 읽는다. 지워진 파일·폴더는 색인에서 뺀다. 색인이 바뀌었으면 `true`.
    pub fn refresh(&mut self, paths: &[PathBuf]) -> bool {
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        for path in paths {
            let Some(key) = self.key(path) else {
                continue;
            };
            if path.is_dir() {
                let mut found = Vec::new();
                collect_markdown(path, &mut found);
                changed.extend(found.iter().filter_map(|path| self.reread(path)));
            } else if path.is_file() && is_markdown(path) {
                changed.extend(self.reread(path));
            } else {
                // 지워진 파일, 또는 지워진 폴더 아래 전부.
                let prefix = format!("{key}/");
                let gone: Vec<String> = self
                    .files
                    .keys()
                    .filter(|other| **other == key || other.starts_with(&prefix))
                    .cloned()
                    .collect();
                for key in gone {
                    self.files.remove(&key);
                    removed.push(key);
                }
            }
        }
        self.write_cache(&changed, &removed);
        !(changed.is_empty() && removed.is_empty())
    }

    /// 모든 할 일과 그 메모 파일 경로. 파일 경로순, 파일 안에서는 나온 순서다.
    pub fn todos(&self) -> impl Iterator<Item = (PathBuf, &Todo)> + '_ {
        self.files.iter().flat_map(|(key, entry)| {
            let path = self.root.join(key);
            entry.todos.iter().map(move |todo| (path.clone(), todo))
        })
    }

    /// 메모(`Notes/`) 안 파일의 색인 키. 메모 밖이면 `None`.
    fn key(&self, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(&self.root).ok()?;
        let mut parts = relative.components();
        if parts.next()? != Component::Normal(SectionKind::Notes.dir().as_ref()) {
            return None;
        }
        let parts: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        Some(parts.join("/"))
    }

    /// 파일이 색인과 다르면(수정 시각·크기) 다시 읽고 키를 돌려준다. 같으면 `None`.
    fn reread(&mut self, path: &Path) -> Option<String> {
        let key = self.key(path)?;
        // 상태 확인(stat)은 한 번만 한다. 시작할 때 메모 수만큼 하므로 이 비용이 대부분이다.
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |delta| delta.as_nanos() as u64);
        let size = meta.len();
        if self
            .files
            .get(&key)
            .is_some_and(|entry| entry.modified == modified && entry.size == size)
        {
            return None;
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let entry = FileEntry {
            modified,
            size,
            todos: markdown::todos(&text),
        };
        self.files.insert(key.clone(), entry);
        Some(key)
    }

    /// 바뀐 항목과 지운 항목을 한 트랜잭션으로 캐시에 쓴다. 실패해도 색인(메모리)은 그대로 쓴다.
    fn write_cache(&self, changed: &[String], removed: &[String]) {
        let Some(cache) = &self.cache else {
            return;
        };
        if changed.is_empty() && removed.is_empty() {
            return;
        }
        let write = || -> Result<(), redb::Error> {
            let txn = cache.db().begin_write()?;
            {
                let mut meta = txn.open_table(TODOS_META)?;
                meta.insert("parser", PARSER_VERSION)?;
                let mut table = txn.open_table(TODOS)?;
                for key in removed {
                    table.remove(key.as_str())?;
                }
                for key in changed {
                    let Some(entry) = self.files.get(key) else {
                        continue;
                    };
                    let Ok(bytes) = serde_json::to_vec(entry) else {
                        continue;
                    };
                    table.insert(key.as_str(), bytes.as_slice())?;
                }
            }
            txn.commit()?;
            Ok(())
        };
        if let Err(err) = write() {
            eprintln!("할 일 캐시를 쓸 수 없습니다: {err}");
        }
    }
}

/// 캐시의 색인을 읽는다. 읽기 규칙 버전이 다르거나 읽지 못하면 빈 색인(전부 다시 읽는다).
fn read_cache(cache: &Cache) -> BTreeMap<String, FileEntry> {
    let read = || -> Result<BTreeMap<String, FileEntry>, redb::Error> {
        let txn = cache.db().begin_read()?;
        let version = match txn.open_table(TODOS_META) {
            Ok(meta) => meta.get("parser")?.map(|value| value.value()),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(err) => return Err(err.into()),
        };
        if version != Some(PARSER_VERSION) {
            return Ok(BTreeMap::new());
        }
        let table = txn.open_table(TODOS)?;
        let mut files = BTreeMap::new();
        for row in table.iter()? {
            let (key, value) = row?;
            if let Ok(entry) = serde_json::from_slice::<FileEntry>(value.value()) {
                files.insert(key.value().to_string(), entry);
            }
        }
        Ok(files)
    };
    read().unwrap_or_default()
}

/// `dir` 아래(하위 그룹 포함)의 마크다운 파일. 숨김 폴더·파일은 뺀다.
fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        // 폴더 항목의 종류는 대개 목록과 함께 와서 따로 stat하지 않는다.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect_markdown(&path, out);
        } else if is_markdown(&path) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::initialize;
    use crate::library::test_util::temp_dir;

    fn titles(index: &TodoIndex) -> Vec<String> {
        index.todos().map(|(_, todo)| todo.title.clone()).collect()
    }

    #[test]
    fn indexes_notes_only_and_reuses_the_cache() {
        let root = temp_dir("todo-index");
        initialize(&root).unwrap();
        let inbox = root.join("Notes").join("Inbox");
        std::fs::write(inbox.join("a.md"), "- [ ] 하나 📅 2026-10-30\n- [x] 둘\n").unwrap();
        let project = root.join("Projects").join("책");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("원고.md"), "- [ ] 원고 안 체크\n").unwrap();
        let cache_path = root.join("cache").join("cache.redb");

        let index = TodoIndex::load(&root, Cache::open_at(&cache_path));
        assert_eq!(titles(&index), ["하나", "둘"]);
        let (path, todo) = index.todos().next().unwrap();
        assert_eq!(path, inbox.join("a.md"));
        assert_eq!(todo.due.as_deref(), Some("2026-10-30"));
        drop(index);

        // 캐시에서 읽은 항목은 다시 파싱하지 않는다: 같은 수정 시각·크기면 캐시 값을 쓴다.
        let cached = read_cache(&Cache::open_at(&cache_path).unwrap());
        assert_eq!(cached.len(), 1);
        assert!(cached.contains_key("Notes/Inbox/a.md"));
        let index = TodoIndex::load(&root, Cache::open_at(&cache_path));
        assert_eq!(titles(&index), ["하나", "둘"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn refresh_rereads_changed_files_and_drops_removed_ones() {
        let root = temp_dir("todo-refresh");
        initialize(&root).unwrap();
        let inbox = root.join("Notes").join("Inbox");
        let note = inbox.join("a.md");
        std::fs::write(&note, "- [ ] 하나\n").unwrap();
        let cache_path = root.join("cache").join("cache.redb");
        let mut index = TodoIndex::load(&root, Cache::open_at(&cache_path));

        std::fs::write(&note, "- [ ] 하나\n- [ ] 새로 추가\n").unwrap();
        let group = root.join("Notes").join("독서");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("b.md"), "- [ ] 책 읽기\n").unwrap();
        assert!(index.refresh(&[note.clone(), group.clone()]));
        assert_eq!(titles(&index), ["하나", "새로 추가", "책 읽기"]);
        // 같은 내용이면 바뀐 것이 없다.
        assert!(!index.refresh(std::slice::from_ref(&note)));

        std::fs::remove_dir_all(&group).unwrap();
        assert!(index.refresh(&[group]));
        assert_eq!(titles(&index), ["하나", "새로 추가"]);
        drop(index);
        let cached = read_cache(&Cache::open_at(&cache_path).unwrap());
        assert_eq!(cached.keys().collect::<Vec<_>>(), ["Notes/Inbox/a.md"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn works_without_a_cache() {
        let root = temp_dir("todo-nocache");
        initialize(&root).unwrap();
        std::fs::write(root.join("Notes").join("Inbox").join("a.md"), "- [ ] 하나\n").unwrap();
        assert_eq!(titles(&TodoIndex::load(&root, None)), ["하나"]);
        std::fs::remove_dir_all(&root).ok();
    }
}
