//! 아이콘 팩 매핑: 도메인 개념 → [`IconName`].
//!
//! 아이콘 세트는 gpui-kit-assets 의 **Lucide** 카탈로그를 사용한다(에셋 소스는
//! `AllAssets`). 화면 코드는 여기 정의된 함수만 호출하고, 아이콘을 바꾸려면
//! 이 파일만 수정하면 된다. (도메인/뷰가 특정 아이콘에 직접 의존하지 않도록 분리)

use gpui_kit::assets::IconName;

/// 사용 중인 아이콘 팩 이름 (설정/About 표기용).
pub const PACK: &str = "Lucide";

/// 문자열 아이콘 이름을 [`IconName`] 으로 해석한다. 모르면 폴더.
pub fn by_name(name: &str) -> IconName {
    match name {
        "inbox" => IconName::Inbox,
        "notebook" => IconName::NotebookPen,
        "book" => IconName::BookOpen,
        "file" => IconName::FileText,
        "folder" => IconName::Folder,
        _ => IconName::Folder,
    }
}

/// 그룹 아이콘: 메타 이름이 있으면 그걸, 없으면 기본 폴더.
pub fn group_icon(name: Option<&str>) -> IconName {
    name.map(by_name).unwrap_or(IconName::Folder)
}

/// 사이드바 토글 아이콘(패널).
pub fn panel() -> IconName {
    IconName::PanelLeft
}

/// 메모 "모두" 보기.
pub fn all_sheets() -> IconName {
    IconName::Layers
}

/// 새 그룹.
pub fn new_group() -> IconName {
    IconName::FolderPlus
}

/// 시트 목록 필터/정렬.
pub fn filter() -> IconName {
    IconName::ListFilter
}

/// 새 시트 작성.
pub fn compose() -> IconName {
    IconName::SquarePen
}

/// 탐색: 뒤로/앞으로.
pub fn back() -> IconName {
    IconName::ChevronLeft
}

pub fn forward() -> IconName {
    IconName::ChevronRight
}

/// 검색.
pub fn search() -> IconName {
    IconName::Search
}

/// 서식: 굵게/기울임/링크/문단.
pub fn bold() -> IconName {
    IconName::Bold
}

pub fn italic() -> IconName {
    IconName::Italic
}

pub fn link() -> IconName {
    IconName::Link
}

pub fn paragraph() -> IconName {
    IconName::Pilcrow
}

/// 공유/내보내기.
pub fn share() -> IconName {
    IconName::Share
}

pub fn export() -> IconName {
    IconName::FileUp
}

/// 프리뷰(오른쪽 패널) 토글.
pub fn preview() -> IconName {
    IconName::PanelRight
}
