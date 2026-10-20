//! 그룹 메타(`<Group>/.writer.toml`) 읽기/쓰기, 프로젝트·그룹·시트 파일 만들기와 순서 바꾸기.

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use super::{
    ATTACHMENTS_GROUP, EXTRAS_DIR, GROUP_META_FILE, Group, SortMode, TEMPLATES_GROUP, file_name,
    sheet_file_name,
};

/// 그룹 폴더의 `.writer.toml` 스키마.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct GroupMeta {
    /// 아이콘 이름(예: inbox, folder, book).
    pub(super) icon: Option<String>,
    /// 색 이름(예: blue). 화면에서 테마 색으로 바꾼다.
    pub(super) color: Option<String>,
    /// 표시 이름(없으면 폴더 이름).
    pub(super) title: Option<String>,
    /// 시트 파일명 순서. 나열되지 않은 파일은 뒤에 이름순으로 붙는다.
    pub(super) sheets: Vec<String>,
    /// 앞 시트에 접합한 시트 파일명.
    pub(super) glued: Vec<String>,
    /// 하위 그룹 폴더 이름 순서. 나열되지 않은 그룹은 뒤에 이름순으로 붙는다.
    pub(super) groups: Vec<String>,
    /// 시트 정렬 방법(`SortMode::key`).
    pub(super) sort: Option<String>,
    /// 정렬 방향. 없으면 정렬 방법의 기본 방향.
    pub(super) descending: Option<bool>,
    /// 그룹이 담는 것(`GroupKind::key`). 없으면 시트.
    pub(super) kind: Option<String>,
    /// 파일 그룹의 파일 이름 순서. 나열되지 않은 파일은 뒤에 이름순으로 붙는다.
    pub(super) files: Vec<String>,
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
/// 접합한 시트는 묶음째 옮긴다(ADR-0012). 놓는 자리가 다른 묶음이면 그 묶음 앞(위로 옮길 때)이나
/// 뒤(아래로 옮길 때)에 놓아 묶음을 가르지 않는다. 같은 묶음 안에 놓으면 그대로 둔다.
pub fn move_sheet(group: &mut Group, from: usize, to: usize) -> io::Result<()> {
    if from >= group.sheets.len() || to >= group.sheets.len() || from == to {
        return Ok(());
    }
    let block = glued_block(group, from);
    let target = glued_block(group, to);
    if block == target {
        return Ok(());
    }
    let before = group.sheets.clone();
    let moved: Vec<_> = group.sheets.drain(block.clone()).collect();
    // 묶음을 빼낸 뒤의 자리: 위로 옮기면 대상 묶음 앞, 아래로 옮기면 대상 묶음 뒤.
    let at = if target.start < block.start {
        target.start
    } else {
        target.end - block.len()
    };
    group.sheets.splice(at..at, moved);
    write_order_and_glue(group, before)
}

/// `index` 시트가 든 접합 묶음(접합하지 않은 시트는 자기 하나).
fn glued_block(group: &Group, index: usize) -> Range<usize> {
    let glued = |index: usize| index > 0 && group.sheets[index].glued;
    let mut start = index;
    while glued(start) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < group.sheets.len() && group.sheets[end].glued {
        end += 1;
    }
    start..end
}

/// `range`의 시트를 하나로 잇는다: 첫 시트 뒤의 시트를 모두 앞 시트에 접합한다.
///
/// 지금 순서를 메타 `sheets`에도 적는다. 순서가 정해지지 않은 시트는 수정할 때마다 자리가 바뀌어
/// 접합한 이웃이 달라질 수 있어서다.
pub fn glue_sheets(group: &mut Group, range: Range<usize>) -> io::Result<()> {
    let end = range.end.min(group.sheets.len());
    if range.start + 1 >= end {
        return Ok(());
    }
    let before = group.sheets.clone();
    for sheet in &mut group.sheets[range.start + 1..end] {
        sheet.glued = true;
    }
    write_order_and_glue(group, before)
}

/// `range`의 시트를 앞뒤 시트에서 떼어 낸다: 범위의 시트와 범위 바로 뒤 시트의 접합을 푼다.
pub fn unglue_sheets(group: &mut Group, range: Range<usize>) -> io::Result<()> {
    let end = (range.end + 1).min(group.sheets.len());
    if range.start >= end {
        return Ok(());
    }
    let before = group.sheets.clone();
    for sheet in &mut group.sheets[range.start..end] {
        sheet.glued = false;
    }
    write_order_and_glue(group, before)
}

/// 시트 순서와 접합을 메타에 쓴다. 실패하면 `before`로 되돌린다.
fn write_order_and_glue(group: &mut Group, before: Vec<super::Sheet>) -> io::Result<()> {
    let names = |glued_only: bool| -> Vec<toml::Value> {
        group
            .sheets
            .iter()
            .enumerate()
            .filter(|(index, sheet)| !glued_only || (*index > 0 && sheet.glued))
            .map(|(_, sheet)| toml::Value::String(sheet_file_name(sheet)))
            .collect()
    };
    let order = names(false);
    let glued = names(true);
    let glued = (!glued.is_empty()).then_some(toml::Value::Array(glued));
    if let Err(err) = write_meta_values(
        &group.path,
        [
            ("sheets", Some(toml::Value::Array(order))),
            ("glued", glued),
        ],
    ) {
        group.sheets = before;
        return Err(err);
    }
    Ok(())
}

/// 그룹의 정렬 방법을 바꾸고 `.writer.toml`의 `sort`에 쓴다. 쓰기에 실패하면 메모리 값도 그대로 둔다.
///
/// 방향은 정렬 방법마다 기본이 달라서(날짜는 최신부터, 제목은 ㄱ부터) 방법을 바꾸면 지운다.
pub fn set_sort(group: &mut Group, mode: SortMode) -> io::Result<()> {
    write_meta_values(
        &group.path,
        [
            ("sort", Some(toml::Value::String(mode.key().to_string()))),
            ("descending", None),
        ],
    )?;
    group.sort = Some(mode);
    group.descending = None;
    Ok(())
}

/// 그룹의 정렬 방향을 바꾸고 `.writer.toml`의 `descending`에 쓴다.
pub fn set_sort_descending(group: &mut Group, descending: bool) -> io::Result<()> {
    write_meta_values(
        &group.path,
        [("descending", Some(toml::Value::Boolean(descending)))],
    )?;
    group.descending = Some(descending);
    Ok(())
}

/// 그룹 `.writer.toml`의 `key`(`sheets`·`groups`)를 `order`(이름 목록)로 바꾼다. 다른 키는 그대로 둔다.
pub(super) fn write_meta_list(group_dir: &Path, key: &str, order: &[String]) -> io::Result<()> {
    let order = order.iter().cloned().map(toml::Value::String).collect();
    write_meta_values(group_dir, [(key, Some(toml::Value::Array(order)))])
}

/// 그룹 `.writer.toml`의 키들을 한 번에 바꾼다(`None`이면 지운다). 다른 키는 그대로 둔다.
///
/// 메타 파일이 깨져 있으면 덮어쓰지 않고 오류를 돌려준다. 임시 파일에 쓴 뒤 바꿔치기한다.
pub(super) fn write_meta_values<'a>(
    group_dir: &Path,
    values: impl IntoIterator<Item = (&'a str, Option<toml::Value>)>,
) -> io::Result<()> {
    let path = group_dir.join(GROUP_META_FILE);
    let mut table: toml::Table = match std::fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => toml::Table::new(),
        Err(err) => return Err(err),
    };
    for (key, value) in values {
        match value {
            Some(value) => table.insert(key.to_string(), value),
            None => table.remove(key),
        };
    }
    let text = toml::to_string_pretty(&table)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let temp = group_dir.join(format!("{GROUP_META_FILE}.tmp"));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, &path)
}

/// `Projects/` 아래에 새 프로젝트(그룹 폴더 + `.writer.toml` + 기본 추가자료)를 만들고 경로를 돌려준다.
///
/// 폴더 이름은 `name`에서 경로 구분자를 뺀 것이고, 비어 있으면 "무제"다.
/// 같은 이름이 있으면 " 2", " 3"…을 붙인다. 표시 이름은 폴더 이름과 같다.
pub fn create_project(
    projects_dir: &Path,
    name: &str,
    icon: Option<&str>,
    color: Option<&str>,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(projects_dir)?;
    let dir = create_unique_dir(projects_dir, &project_dir_name(name))?;
    write_group_meta(&dir, icon, color)?;
    ensure_project_layout(&dir)?;
    Ok(dir)
}

/// 새 그룹 폴더에 아이콘·색만 담은 `.writer.toml`을 쓴다.
fn write_group_meta(dir: &Path, icon: Option<&str>, color: Option<&str>) -> io::Result<()> {
    let mut table = toml::Table::new();
    for (key, value) in [("icon", icon), ("color", color)] {
        if let Some(value) = value {
            table.insert(key.to_string(), toml::Value::String(value.to_string()));
        }
    }
    let text = toml::to_string_pretty(&table)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    std::fs::write(dir.join(GROUP_META_FILE), text)
}

/// 프로젝트에 추가자료 폴더가 없으면 기본 그룹(템플릿)과 함께 만든다(멱등).
///
/// 이미 있으면 건드리지 않는다. 사용자가 지운 기본 그룹을 다시 만들지 않기 위해서다.
pub(super) fn ensure_project_layout(project_dir: &Path) -> io::Result<()> {
    let extras = project_dir.join(EXTRAS_DIR);
    if extras.exists() {
        return Ok(());
    }
    let templates = extras.join(TEMPLATES_GROUP);
    std::fs::create_dir_all(&templates)?;
    write_group_meta_if_absent(&templates, Some("layout-template"), Some("템플릿"))
}

/// `parent` 아래에 새 그룹 폴더(+ `.writer.toml`)를 만들고 경로를 돌려준다.
///
/// 이름 규칙은 프로젝트와 같고, `Extras`는 추가자료 자리라 피한다.
/// 새 그룹은 `parent`의 그룹 순서(`groups`) 맨 뒤에 붙는다.
pub fn create_group(
    parent: &Path,
    name: &str,
    icon: Option<&str>,
    color: Option<&str>,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(parent)?;
    let mut base = project_dir_name(name);
    if base == EXTRAS_DIR {
        base.push_str(" 2");
    }
    let dir = create_unique_dir(parent, &base)?;
    let order = write_group_meta(&dir, icon, color).and_then(|()| {
        let mut order = group_order(parent)?;
        order.retain(|name| *name != file_name(&dir));
        order.push(file_name(&dir));
        write_meta_list(parent, "groups", &order)
    });
    if let Err(err) = order {
        // 방금 만든 폴더라 메타 파일 말고는 비어 있다.
        let _ = std::fs::remove_dir_all(&dir);
        return Err(err);
    }
    Ok(dir)
}

/// `parent` 바로 아래 그룹 폴더 이름을 화면 순서(메타 `groups` → 이름순)로 돌려준다.
pub(super) fn group_order(parent: &Path) -> io::Result<Vec<String>> {
    let meta = read_group_meta(parent);
    let mut names = Vec::new();
    for entry in std::fs::read_dir(parent)? {
        let path = entry?.path();
        if super::is_group_dir(&path) && file_name(&path) != EXTRAS_DIR {
            names.push(file_name(&path));
        }
    }
    names.sort_by_key(|name| name.to_lowercase());
    super::order_by_names(&mut names, &meta.groups, String::clone);
    Ok(names)
}

/// `parent/base`를 만든다. 이미 있으면 " 2", " 3"…을 붙여 비어 있는 이름을 찾는다.
fn create_unique_dir(parent: &Path, base: &str) -> io::Result<PathBuf> {
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            parent.join(base)
        } else {
            parent.join(format!("{base} {n}"))
        };
        // create_dir는 이미 있으면 실패하므로 동시에 만들어도 덮어쓰지 않는다.
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(err) => return Err(err),
        }
    }
}

/// 프로젝트 이름 → 폴더 이름: 경로 구분자를 `-`로 바꾸고, 앞의 `.`(숨김)을 떼고, 비면 "무제".
fn project_dir_name(name: &str) -> String {
    let base: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let base = base.trim_start_matches('.').trim();
    if base.is_empty() {
        "무제".to_string()
    } else {
        base.to_string()
    }
}

/// 프로젝트의 아이콘·색을 바꾸고, 이름이 바뀌었으면 폴더 이름도 바꾼다. 새 경로를 돌려준다.
///
/// 메타의 다른 키(시트 순서 등)는 그대로 둔다. 표시 이름은 폴더 이름을 따르므로 `title`은 지운다.
/// 바꿀 이름이 이미 있으면 " 2", " 3"…을 붙인다.
pub fn update_project(
    dir: &Path,
    name: &str,
    icon: Option<&str>,
    color: Option<&str>,
) -> io::Result<PathBuf> {
    let path = dir.join(GROUP_META_FILE);
    let mut table: toml::Table = match std::fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => toml::Table::new(),
        Err(err) => return Err(err),
    };
    for (key, value) in [("icon", icon), ("color", color)] {
        match value {
            Some(value) => table.insert(key.to_string(), toml::Value::String(value.to_string())),
            None => table.remove(key),
        };
    }
    table.remove("title");
    let text = toml::to_string_pretty(&table)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let temp = dir.join(format!("{GROUP_META_FILE}.tmp"));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, &path)?;

    let base = project_dir_name(name);
    if dir
        .file_name()
        .is_some_and(|current| current == base.as_str())
    {
        return Ok(dir.to_path_buf());
    }
    let parent = dir
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "상위 폴더가 없습니다"))?;
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            parent.join(&base)
        } else {
            parent.join(format!("{base} {n}"))
        };
        // rename은 빈 폴더를 덮어쓸 수 있으므로 먼저 확인한다.
        if candidate.exists() {
            n += 1;
            continue;
        }
        std::fs::rename(dir, &candidate)?;
        return Ok(candidate);
    }
}

/// 그룹의 이름·아이콘·색을 바꾸고 새 경로를 돌려준다.
///
/// 보통 그룹은 프로젝트처럼 폴더 이름을 바꾸고 상위 그룹 순서(`groups`)의 이름도 고친다.
/// `Extras`는 추가자료 자리라 피한다. 앱이 폴더 이름으로 찾는 기본 그룹(템플릿·첨부 파일)은
/// 폴더를 그대로 두고 표시 이름을 `title`에 쓴다.
pub fn update_group(
    dir: &Path,
    name: &str,
    icon: Option<&str>,
    color: Option<&str>,
) -> io::Result<PathBuf> {
    let parent = dir
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "상위 폴더가 없습니다"))?;
    let reserved = file_name(parent) == EXTRAS_DIR
        && [TEMPLATES_GROUP, ATTACHMENTS_GROUP].contains(&file_name(dir).as_str());
    if reserved {
        let text = |value: &str| Some(toml::Value::String(value.to_string()));
        write_meta_values(
            dir,
            [
                ("title", text(name.trim())),
                ("icon", icon.and_then(text)),
                ("color", color.and_then(text)),
            ],
        )?;
        return Ok(dir.to_path_buf());
    }

    let mut name = name.to_string();
    if project_dir_name(&name) == EXTRAS_DIR {
        name.push_str(" 2");
    }
    let mut order = group_order(parent)?;
    let new_dir = update_project(dir, &name, icon, color)?;
    if new_dir != dir {
        let old = file_name(dir);
        for entry in &mut order {
            if *entry == old {
                *entry = file_name(&new_dir);
            }
        }
        write_meta_list(parent, "groups", &order)?;
    }
    Ok(new_dir)
}

/// 프로젝트·그룹 폴더를 운영체제 휴지통으로 옮긴다(복원 가능).
///
/// macOS는 Finder 대신 `NSFileManager`로 옮긴다. Finder 방식은 자동화 권한을 묻느라
/// 응답을 기다리는 동안 UI가 멈춘다. 대신 Finder의 "되돌려 놓기"는 없을 수 있다.
pub fn move_to_trash(dir: &Path) -> io::Result<()> {
    #[allow(unused_mut)]
    let mut trash = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos as _};
        trash.set_delete_method(DeleteMethod::NsFileManager);
    }
    trash.delete(dir).map_err(io::Error::other)
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
    fn update_project_keeps_sheet_order_and_renames() {
        let root = temp_dir("update-project");
        initialize(&root).unwrap();
        let projects = root.join(SectionKind::Projects.dir());
        let dir = create_project(&projects, "소설", Some("book"), Some("blue")).unwrap();
        let sheet = create_sheet(&dir).unwrap();
        let file = sheet.file_name().unwrap().to_string_lossy().into_owned();
        write_meta_list(&dir, "sheets", std::slice::from_ref(&file)).unwrap();
        create_project(&projects, "에세이", None, None).unwrap();

        // 아이콘·색만 바꾸면 폴더는 그대로
        let same = update_project(&dir, "소설", Some("newspaper"), Some("red")).unwrap();
        assert_eq!(same, dir);
        // 이미 있는 이름으로 바꾸면 번호를 붙인다
        let renamed = update_project(&dir, "에세이", Some("newspaper"), Some("red")).unwrap();
        assert_eq!(renamed, projects.join("에세이 2"));
        assert!(!dir.exists());
        assert!(renamed.join(&file).is_file());

        let meta = read_group_meta(&renamed);
        assert_eq!(meta.icon.as_deref(), Some("newspaper"));
        assert_eq!(meta.color.as_deref(), Some("red"));
        assert_eq!(meta.sheets, vec![file]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn update_group_renames_in_place_and_keeps_reserved_folders() {
        let root = temp_dir("update-group");
        initialize(&root).unwrap();
        let projects = root.join(SectionKind::Projects.dir());
        let dir = create_project(&projects, "소설", None, None).unwrap();
        let first = create_group(&dir, "가", None, None).unwrap();
        create_group(&dir, "나", None, None).unwrap();

        // 이름을 바꿔도 순서 자리는 그대로다. `Extras`는 피한다.
        let renamed = update_group(&first, "하", Some("files"), Some("red")).unwrap();
        assert_eq!(renamed, dir.join("하"));
        assert_eq!(group_order(&dir).unwrap(), vec!["하", "나"]);
        let clash = update_group(&renamed, EXTRAS_DIR, None, None).unwrap();
        assert_eq!(clash, dir.join("Extras 2"));

        // 기본 그룹은 폴더를 두고 표시 이름만 바꾼다.
        let templates = dir.join(EXTRAS_DIR).join(TEMPLATES_GROUP);
        assert_eq!(
            update_group(&templates, "양식", None, Some("blue")).unwrap(),
            templates
        );
        let meta = read_group_meta(&templates);
        assert_eq!(meta.title.as_deref(), Some("양식"));
        assert_eq!(meta.icon, None);
        assert_eq!(meta.color.as_deref(), Some("blue"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn project_groups_keep_creation_order_and_extras_stay_apart() {
        let root = temp_dir("project-groups");
        initialize(&root).unwrap();
        let projects = root.join(SectionKind::Projects.dir());
        let dir = create_project(&projects, "소설", None, None).unwrap();

        // 이름순과 다른 순서로 만들어도 만든 순서를 지킨다.
        let prologue = create_group(&dir, "프롤로그", Some("files"), None).unwrap();
        let front = create_group(&dir, "앞부속", Some("files"), None).unwrap();
        let part = create_group(&dir, "제1부", None, None).unwrap();
        let chapter = create_group(&part, "1장", None, None).unwrap();
        create_sheet(&chapter).unwrap();
        // 추가자료 자리 이름은 원고 그룹으로 쓸 수 없다.
        let clash = create_group(&dir, EXTRAS_DIR, None, None).unwrap();
        assert_eq!(clash, dir.join("Extras 2"));
        let people = create_group(
            &dir.join(EXTRAS_DIR),
            "등장인물",
            Some("folder"),
            Some("blue"),
        )
        .unwrap();

        let library = scan(&root).unwrap();
        let project = library.find_project(&dir).unwrap();
        let names: Vec<_> = project.groups.iter().map(|g| g.path.clone()).collect();
        assert_eq!(names, vec![prologue, front, part.clone(), clash]);
        assert_eq!(project.groups[2].groups[0].path, chapter);
        assert_eq!(library.find_group(&chapter).unwrap().sheets.len(), 1);

        // 새 프로젝트는 기본 추가자료(템플릿)를 갖고, 새 추가자료는 그 뒤에 붙는다.
        let extras: Vec<_> = project.extras.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(extras, vec!["템플릿", "등장인물"]);
        assert_eq!(
            library.find_group(&people).unwrap().icon.as_deref(),
            Some("folder")
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn open_adds_extras_to_existing_project_once() {
        let root = temp_dir("project-extras");
        initialize(&root).unwrap();
        let dir = root.join(SectionKind::Projects.dir()).join("옛 프로젝트");
        std::fs::create_dir_all(&dir).unwrap();

        open(&root).unwrap();
        let templates = dir.join(EXTRAS_DIR).join(TEMPLATES_GROUP);
        assert!(templates.is_dir());

        // 사용자가 지운 기본 그룹은 다시 만들지 않는다.
        std::fs::remove_dir_all(&templates).unwrap();
        let library = open(&root).unwrap();
        assert!(!templates.exists());
        assert!(library.find_project(&dir).unwrap().extras.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn create_project_writes_meta_and_avoids_name_clash() {
        let root = temp_dir("project");
        initialize(&root).unwrap();
        let projects = root.join(SectionKind::Projects.dir());

        let first = create_project(&projects, " 소설/1부 ", Some("book"), Some("blue")).unwrap();
        assert_eq!(first, projects.join("소설-1부"));
        let second = create_project(&projects, "소설-1부", None, None).unwrap();
        assert_eq!(second, projects.join("소설-1부 2"));
        assert_eq!(
            create_project(&projects, "  ", None, None).unwrap(),
            projects.join("무제")
        );

        let library = scan(&root).unwrap();
        let groups: Vec<_> = library
            .sections
            .iter()
            .find(|section| section.kind == SectionKind::Projects)
            .unwrap()
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Group(group) => Some(group),
                _ => None,
            })
            .collect();
        let novel = groups.iter().find(|group| group.path == first).unwrap();
        assert_eq!(novel.icon.as_deref(), Some("book"));
        assert_eq!(novel.color.as_deref(), Some("blue"));
        assert_eq!(groups.len(), 3);

        std::fs::remove_dir_all(&root).ok();
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

        let mut group = scan_group(&inbox, false).unwrap();
        move_sheet(&mut group, 0, 2).unwrap();
        let moved: Vec<_> = group.sheets.iter().map(stem).collect();
        assert_eq!(moved, vec!["b", "c", "a"]);

        // 다시 읽어도 같은 순서, 아이콘은 그대로
        let rescanned = scan_group(&inbox, false).unwrap();
        let titles: Vec<_> = rescanned.sheets.iter().map(stem).collect();
        assert_eq!(titles, vec!["b", "c", "a"]);
        assert_eq!(rescanned.icon.as_deref(), Some("inbox"));
        assert!(!inbox.join(format!("{GROUP_META_FILE}.tmp")).exists());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn glue_and_unglue_sheets() {
        let root = temp_dir("glue");
        initialize(&root).unwrap();
        let inbox = root.join("Notes").join(DEFAULT_INBOX);
        for stem in ["a", "b", "c", "d"] {
            std::fs::write(inbox.join(format!("{stem}.md")), stem).unwrap();
        }
        std::fs::write(
            inbox.join(GROUP_META_FILE),
            "sheets = [\"a.md\", \"b.md\", \"c.md\", \"d.md\"]\n",
        )
        .unwrap();
        let glued =
            |group: &Group| -> Vec<bool> { group.sheets.iter().map(|sheet| sheet.glued).collect() };

        let mut group = scan_group(&inbox, false).unwrap();
        glue_sheets(&mut group, 0..3).unwrap();
        assert_eq!(glued(&group), vec![false, true, true, false]);
        assert_eq!(read_group_meta(&inbox).glued, vec!["b.md", "c.md"]);
        assert_eq!(glued(&scan_group(&inbox, false).unwrap()), glued(&group));

        // 가운데 시트를 떼면 앞뒤 모두 떨어진다.
        unglue_sheets(&mut group, 1..2).unwrap();
        assert_eq!(glued(&group), vec![false, false, false, false]);
        let text = std::fs::read_to_string(inbox.join(GROUP_META_FILE)).unwrap();
        assert!(!text.contains("glued"), "{text}");

        // 접합한 시트는 묶음째 옮기고, 다른 묶음을 가르지 않는다.
        glue_sheets(&mut group, 1..3).unwrap(); // a [b c] d
        let order = |group: &Group| -> Vec<String> { group.sheets.iter().map(stem).collect() };
        move_sheet(&mut group, 2, 3).unwrap();
        assert_eq!(order(&group), vec!["a", "d", "b", "c"]);
        assert_eq!(glued(&group), vec![false, false, false, true]);
        move_sheet(&mut group, 0, 3).unwrap();
        assert_eq!(order(&group), vec!["d", "b", "c", "a"]);
        move_sheet(&mut group, 3, 2).unwrap(); // c 위에 놓으면 묶음 앞으로
        assert_eq!(order(&group), vec!["d", "a", "b", "c"]);
        move_sheet(&mut group, 2, 3).unwrap(); // 같은 묶음 안은 그대로
        assert_eq!(order(&group), vec!["d", "a", "b", "c"]);
        let rescanned = scan_group(&inbox, false).unwrap();
        assert_eq!(order(&rescanned), order(&group));
        assert_eq!(glued(&rescanned), vec![false, false, false, true]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn set_sort_persists_and_keeps_other_keys() {
        let root = temp_dir("sort");
        initialize(&root).unwrap();
        let inbox = root.join("Notes").join(DEFAULT_INBOX);

        let mut group = scan_group(&inbox, false).unwrap();
        assert_eq!(group.sort, None);
        set_sort(&mut group, SortMode::Title).unwrap();
        assert_eq!(group.sort, Some(SortMode::Title));

        set_sort_descending(&mut group, true).unwrap();

        let rescanned = scan_group(&inbox, false).unwrap();
        assert_eq!(rescanned.sort, Some(SortMode::Title));
        assert_eq!(rescanned.descending, Some(true));
        assert_eq!(rescanned.icon.as_deref(), Some("inbox"));

        // 방법을 바꾸면 방향은 그 방법의 기본으로 돌아간다.
        set_sort(&mut group, SortMode::Created).unwrap();
        assert_eq!(scan_group(&inbox, false).unwrap().descending, None);

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
