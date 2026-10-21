//! 마크다운 이미지 경로(`![캡션](경로)`, ADR-0007)를 디스크의 파일로 찾는다.
//!
//! 미리보기·PDF·DOCX·ePub과 에디터가 같은 규칙을 써야 같은 그림이 보인다. 상대 경로는
//! 시트 파일이 든 폴더 기준이고, 그 밖에 프로젝트 폴더와 첨부파일 그룹도 본다.
//!
//! 시트나 그룹을 다른 폴더로 옮기면(`../` 깊이가 달라진다) 마크다운에 적힌 상대 경로가
//! 어긋난다. 그런 경로도 찾을 수 있도록 `../` 뒤의 꼬리를 시트의 위 폴더들(프로젝트 폴더까지)
//! 에서 한 번 더 찾는다. 그래야 옮긴 뒤에도 ePub이 깨진 이미지 경로를 만들지 않는다.

use std::path::{Component, Path, PathBuf};

use super::{ATTACHMENTS_GROUP, EXTRAS_DIR, GROUP_META_FILE};

/// 웹 주소인지. 웹 이미지는 파일이 아니라 주소를 그대로 쓴다.
pub fn is_web_address(destination: &str) -> bool {
    destination.starts_with("http://") || destination.starts_with("https://")
}

/// 이미지 경로를 디스크의 파일로 찾는다. 웹 주소이거나 파일이 없으면 `None`.
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
    if let Some(found) = candidates.into_iter().find(|candidate| candidate.is_file()) {
        return Some(found);
    }

    // `../`가 어긋난 경로: `../`를 뗀 꼬리를 시트의 위 폴더들에서 다시 찾는다.
    // 프로젝트·그룹 폴더는 `.writer.toml`이 있으므로 거기까지만 올라간다.
    let tail = without_leading_parents(path)?;
    if tail.as_path() == path {
        return None;
    }
    sheet_dir
        .ancestors()
        .take_while(|ancestor| ancestor.join(GROUP_META_FILE).is_file())
        .map(|ancestor| ancestor.join(&tail))
        .find(|candidate| candidate.is_file())
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

    /// `Extras/Attachments/a.png`의 실제 경로.
    fn attachment(project: &Path) -> PathBuf {
        project
            .join(EXTRAS_DIR)
            .join(ATTACHMENTS_GROUP)
            .join("a.png")
    }

    #[test]
    fn finds_relative_path_from_sheet_folder() {
        let (project, sheet_dir) = project();
        // `..`가 남은 경로를 그대로 돌려주므로 실제 파일인지만 본다.
        let found =
            resolve_image_path(&sheet_dir, Some(&project), "../../Extras/Attachments/a.png")
                .unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            attachment(&project).canonicalize().unwrap()
        );
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
