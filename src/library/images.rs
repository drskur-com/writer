//! 마크다운 이미지 경로(`![캡션](경로)`, ADR-0007)를 디스크의 파일로 찾는다.
//!
//! 미리보기·PDF·DOCX·ePub과 에디터가 같은 규칙을 써야 같은 그림이 보인다. 상대 경로는
//! 시트 파일이 든 폴더 기준이고, 그 밖에 프로젝트 폴더와 첨부파일 그룹도 본다.
//!
//! 시트나 그룹을 다른 폴더로 옮기면(`../` 깊이가 달라진다) 마크다운에 적힌 상대 경로가
//! 어긋난다. 그런 경로도 찾을 수 있도록 `../` 뒤의 꼬리를 시트의 위 폴더들(프로젝트 폴더까지)
//! 에서 한 번 더 찾는다. 그래야 옮긴 뒤에도 ePub이 깨진 이미지 경로를 만들지 않는다.

use std::path::{Component, Path, PathBuf};

use super::layout::has_marker;
use super::{ATTACHMENTS_GROUP, EXTRAS_DIR, GROUP_META_FILE, SECTIONS};

/// 웹 주소인지. 웹 이미지는 파일이 아니라 주소를 그대로 쓴다.
pub fn is_web_address(destination: &str) -> bool {
    destination.starts_with("http://") || destination.starts_with("https://")
}

/// 이미지 경로를 디스크의 파일로 찾는다. 웹 주소이거나 파일이 없으면 `None`.
/// 찾은 경로는 `..`를 푼 실제 경로라 같은 파일이면 같은 값이다(내보내기에서 한 번만 넣는다).
///
/// `project_dir`는 프로젝트 폴더(원고·추가자료의 기준)다. 모르면 `None`을 주고,
/// 그때는 시트 폴더 주변만 본다.
pub fn resolve_image_path(
    sheet_dir: &Path,
    project_dir: Option<&Path>,
    destination: &str,
) -> Option<PathBuf> {
    if is_web_address(destination) {
        return None;
    }
    let path = Path::new(destination.strip_prefix("file://").unwrap_or(destination));
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }

    let mut candidates = vec![sheet_dir.join(path)];
    if let Some(project_dir) = project_dir {
        candidates.push(project_dir.join(path));
        candidates.push(
            project_dir
                .join(EXTRAS_DIR)
                .join(ATTACHMENTS_GROUP)
                .join(path),
        );
    }
    let found = candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .or_else(|| {
            // `../`가 어긋난 경로: `../`를 뗀 꼬리를 시트의 위 폴더들에서 다시 찾는다.
            let tail = without_leading_parents(path)?;
            if tail.as_path() == path {
                return None;
            }
            search_dirs(sheet_dir, project_dir)
                .into_iter()
                .map(|dir| dir.join(&tail))
                .find(|candidate| candidate.is_file())
        })?;
    Some(found.canonicalize().unwrap_or(found))
}

/// 어긋난 경로를 다시 찾을 시트 폴더와 그 위 폴더들(프로젝트 폴더까지).
///
/// 프로젝트 폴더를 알면 거기까지 올라간다. 모르면 라이브러리 섹션(`Projects/`·`Notes/`)
/// 바로 아래 폴더까지 올라가고, 라이브러리 밖이면 `.writer.toml`이 있는 그룹 폴더까지만 본다.
/// 그룹 폴더에 `.writer.toml`이 꼭 있지는 않으므로 라이브러리 안에서는 그것에 기대지 않는다.
fn search_dirs<'a>(sheet_dir: &'a Path, project_dir: Option<&Path>) -> Vec<&'a Path> {
    let ancestors: Vec<&Path> = sheet_dir.ancestors().collect();
    let end = match project_dir {
        Some(project_dir) => ancestors.iter().position(|dir| *dir == project_dir),
        None => ancestors.iter().position(|dir| {
            dir.parent().is_some_and(|section| {
                SECTIONS.iter().any(|kind| section.ends_with(kind.dir()))
                    && section.parent().is_some_and(has_marker)
            })
        }),
    };
    match end {
        Some(end) => ancestors[..=end].to_vec(),
        None => ancestors
            .into_iter()
            .take_while(|dir| dir.join(GROUP_META_FILE).is_file())
            .collect(),
    }
}

/// 앞머리의 `.`·`..`를 뗀 경로. 뗄 것이 없거나 남는 것이 없으면 `None`.
fn without_leading_parents(path: &Path) -> Option<PathBuf> {
    let mut parts = path.components().peekable();
    while matches!(parts.peek(), Some(Component::CurDir | Component::ParentDir)) {
        parts.next();
    }
    parts.peek()?;
    Some(parts.collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::test_util::temp_dir;

    /// `Projects/책/본문/1장` 아래 시트, `Projects/책/Extras/Attachments`의 이미지.
    fn project() -> (PathBuf, PathBuf) {
        let root = temp_dir("images");
        let project = root.join("Projects").join("책");
        let attachments = project.join(EXTRAS_DIR).join(ATTACHMENTS_GROUP);
        std::fs::create_dir_all(&attachments).unwrap();
        std::fs::create_dir_all(project.join("본문").join("1장")).unwrap();
        for group in [
            &project,
            &project.join("본문"),
            &project.join("본문").join("1장"),
        ] {
            std::fs::write(group.join(GROUP_META_FILE), "").unwrap();
        }
        std::fs::write(attachments.join("a.png"), "png").unwrap();
        let sheet_dir = project.join("본문").join("1장");
        (project, sheet_dir)
    }

    /// `Extras/Attachments/a.png`의 실제 경로(`..`·심볼릭 링크를 푼 경로).
    fn attachment(project: &Path) -> PathBuf {
        project
            .join(EXTRAS_DIR)
            .join(ATTACHMENTS_GROUP)
            .join("a.png")
            .canonicalize()
            .unwrap()
    }

    #[test]
    fn finds_relative_path_from_sheet_folder() {
        let (project, sheet_dir) = project();
        assert_eq!(
            resolve_image_path(&sheet_dir, Some(&project), "../../Extras/Attachments/a.png"),
            Some(attachment(&project))
        );
    }

    #[test]
    fn finds_stale_parent_path_in_groups_without_meta_file() {
        // 라이브러리 안에서는 `.writer.toml`이 없는 그룹이어도 프로젝트 폴더까지 올라간다.
        let root = temp_dir("images-library");
        crate::library::initialize(&root).unwrap();
        let project = root.join("Projects").join("책");
        let attachments = project.join(EXTRAS_DIR).join(ATTACHMENTS_GROUP);
        let sheet_dir = project.join("본문").join("1장");
        std::fs::create_dir_all(&attachments).unwrap();
        std::fs::create_dir_all(&sheet_dir).unwrap();
        std::fs::write(attachments.join("a.png"), "png").unwrap();
        assert_eq!(
            resolve_image_path(&sheet_dir, None, "../Extras/Attachments/a.png"),
            Some(attachment(&project))
        );
        // 프로젝트 폴더 밖(`Projects/`)까지는 올라가지 않는다.
        std::fs::write(root.join("Projects").join("b.png"), "png").unwrap();
        assert_eq!(resolve_image_path(&sheet_dir, None, "../b.png"), None);
    }

    #[test]
    fn finds_stale_parent_path_from_upper_folders() {
        // 그룹을 옮겨 `../` 깊이가 어긋난 경로도 같은 파일을 찾는다.
        let (project, sheet_dir) = project();
        assert_eq!(
            resolve_image_path(&sheet_dir, Some(&project), "../Extras/Attachments/a.png"),
            Some(attachment(&project))
        );
        // 프로젝트 폴더를 몰라도(에디터·미리보기) 위 폴더를 훑어 찾는다.
        assert_eq!(
            resolve_image_path(&sheet_dir, None, "../Extras/Attachments/a.png"),
            Some(attachment(&project))
        );
        // 시트를 프로젝트 폴더 맨 위로 옮겨 `..`가 하나 더 붙은 경우.
        assert_eq!(
            resolve_image_path(&project, None, "../../Extras/Attachments/a.png"),
            Some(attachment(&project))
        );
    }

    #[test]
    fn finds_project_relative_path() {
        let (project, sheet_dir) = project();
        assert_eq!(
            resolve_image_path(&sheet_dir, Some(&project), "Extras/Attachments/a.png"),
            Some(attachment(&project))
        );
    }

    #[test]
    fn leaves_web_addresses_and_missing_files() {
        let (project, sheet_dir) = project();
        assert_eq!(
            resolve_image_path(&sheet_dir, Some(&project), "https://x.io/a.png"),
            None
        );
        assert_eq!(
            resolve_image_path(&sheet_dir, Some(&project), "../Extras/Attachments/b.png"),
            None
        );
        assert_eq!(
            resolve_image_path(
                &sheet_dir,
                Some(&project),
                attachment(&project).to_str().unwrap()
            ),
            Some(attachment(&project))
        );
    }
}
