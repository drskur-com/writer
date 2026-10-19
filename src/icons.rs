//! 아이콘 팩 매핑: 도메인 개념 → [`IconName`].
//!
//! 아이콘 세트는 gpui-kit-assets 의 **Lucide** 카탈로그를 사용한다(에셋 소스는
//! `AllAssets`). 화면 코드는 여기 정의된 함수만 호출하고, 아이콘을 바꾸려면
//! 이 파일만 수정하면 된다. (도메인/뷰가 특정 아이콘에 직접 의존하지 않도록 분리)

use gpui_kit::assets::IconName;

use crate::library::SectionKind;

/// 사용 중인 아이콘 팩 이름 (설정/About 표기용).
pub const PACK: &str = "Lucide";

/// 라이브러리 섹션 아이콘.
pub fn section(kind: SectionKind) -> IconName {
    match kind {
        SectionKind::Inbox => IconName::Inbox,
        SectionKind::Notes => IconName::NotebookPen,
        SectionKind::Projects => IconName::BookOpen,
    }
}

/// 프로젝트 그룹(폴더) 아이콘.
pub fn group() -> IconName {
    IconName::Folder
}

/// 시트(문서) 아이콘.
pub fn sheet() -> IconName {
    IconName::FileText
}
