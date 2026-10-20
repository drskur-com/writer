//! 프로젝트 원고: 빌드(미리보기·내보내기)에 들어가는 시트와 그 순서.
//!
//! 프로젝트 바로 아래 시트 → 원고 그룹(메타 `groups` 순서, 그룹마다 자기 시트 → 하위 그룹, 깊이 우선).
//! 시트는 그룹의 수동 순서(메타 `sheets`)를 따른다. 추가자료(`Extras/`)는 빠진다.

use std::path::PathBuf;

use super::Group;

/// 원고에 들어가는 시트 하나.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManuscriptSheet {
    pub path: PathBuf,
    /// 앞 시트에 이어지는지(ADR-0012). 이어지지 않으면 새 페이지에서 시작한다.
    /// 그룹의 첫 시트는 접합 표시가 있어도 이어지지 않는다.
    pub glued: bool,
}

/// 프로젝트 원고의 시트를 빌드 순서대로 늘어놓는다.
pub fn manuscript_sheets(project: &Group) -> Vec<ManuscriptSheet> {
    let mut out = Vec::new();
    collect(project, &mut out);
    out
}

fn collect(group: &Group, out: &mut Vec<ManuscriptSheet>) {
    for (ix, sheet) in group.sheets.iter().enumerate() {
        out.push(ManuscriptSheet {
            path: sheet.path.clone(),
            glued: ix > 0 && sheet.glued,
        });
    }
    for child in &group.groups {
        collect(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::test_util::temp_dir;
    use crate::library::*;

    #[test]
    fn manuscript_follows_group_order_and_skips_extras() {
        let root = temp_dir("manuscript");
        initialize(&root).unwrap();
        let projects = root.join(SectionKind::Projects.dir());
        let dir = create_project(&projects, "소설", None, None).unwrap();
        let first = create_group(&dir, "1부", None, None).unwrap();
        let nested = create_group(&first, "1장", None, None).unwrap();
        let second = create_group(&dir, "2부", None, None).unwrap();
        let a = create_sheet(&first).unwrap();
        let b = create_sheet(&first).unwrap();
        let c = create_sheet(&nested).unwrap();
        let d = create_sheet(&second).unwrap();
        let top = create_sheet(&dir).unwrap();
        create_sheet(&dir.join(EXTRAS_DIR).join(TEMPLATES_GROUP)).unwrap();

        let mut library = scan(&root).unwrap();
        // b를 a에 접합한다.
        let group = library.find_group_mut(&first).unwrap();
        let order: Vec<_> = group.sheets.iter().map(|s| s.path.clone()).collect();
        glue_sheets(group, 0..2).unwrap();
        let library = scan(&root).unwrap();
        let project = library.find_project(&dir).unwrap();

        let sheets = manuscript_sheets(project);
        let paths: Vec<_> = sheets.iter().map(|s| s.path.clone()).collect();
        assert_eq!(paths, vec![top, order[0].clone(), order[1].clone(), c, d]);
        assert!(paths.contains(&a) && paths.contains(&b));
        let glued: Vec<_> = sheets.iter().map(|s| s.glued).collect();
        assert_eq!(glued, vec![false, false, true, false, false]);

        std::fs::remove_dir_all(&root).ok();
    }
}
