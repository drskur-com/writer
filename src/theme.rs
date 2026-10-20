//! 앱 테마 정의와 전환.
//!
//! 색상의 단일 출처(single source of truth)는 `themes/writer.json` 이며,
//! gpui-component 의 `ThemeRegistry` / `ThemeConfig` 스키마를 그대로 따른다.
//! 앱 UI 는 하드코딩된 색 대신 `cx.theme()` 토큰을 사용한다.
//!
//! ```ignore
//! theme::register(cx);              // 커스텀 테마 등록
//! theme::set_theme("Writer Dark", cx); // 이름으로 적용
//! ```

use gpui_kit::component::{Theme, ThemeRegistry};
use gpui_kit::gpui::{App, SharedString};

#[cfg(test)]
use gpui_kit::component::ThemeSet;

/// 앱이 등록하는 커스텀 테마 세트 (gpui-component 테마 스키마).
const WRITER_THEMES: &str = include_str!("../themes/writer.json");

/// 기본 다크 테마 이름.
pub const DEFAULT_DARK: &str = "Writer Dark";

/// 기본 라이트 테마 이름.
pub const DEFAULT_LIGHT: &str = "Writer Light";

/// 커스텀 테마를 레지스트리에 등록한다. `gpui_kit::init` 이후에 호출해야 한다.
pub fn register(cx: &mut App) {
    if let Err(err) = ThemeRegistry::global_mut(cx).load_themes_from_str(WRITER_THEMES) {
        // 테마가 깨져도 앱 시작은 막지 않고 기본 테마로 계속 진행한다.
        eprintln!("테마 로드 실패: {err:#}");
    }
}

/// 메뉴에 노출할 테마 이름 목록. `ThemeRegistry::sorted_themes` 순서를 따른다.
///
/// gpui-component 기본 테마(`is_default`)는 앱 레이아웃용 토큰이 맞춰져 있지 않으므로 뺀다.
/// `themes/writer.json`에 테마를 추가하면 자동으로 포함된다.
pub fn theme_names(cx: &App) -> Vec<SharedString> {
    ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .filter(|theme| !theme.is_default)
        .map(|theme| theme.name.clone())
        .collect()
}

/// 이름으로 테마를 적용하고 성공 여부를 돌려준다. 등록되지 않은 이름이면 아무 것도 하지 않고 `false`.
///
/// 모드(라이트/다크)까지 함께 전환하므로 호출 후 메뉴를 다시 만들면
/// 체크 표시가 새 테마를 가리킨다.
pub fn set_theme(name: &str, cx: &mut App) -> bool {
    let Some(config) = ThemeRegistry::global(cx).themes().get(name).cloned() else {
        return false;
    };
    Theme::update(cx, |theme| theme.apply_config(&config));
    true
}

/// 현재 활성 테마 이름.
pub fn active_name(cx: &App) -> SharedString {
    Theme::global(cx).theme_name().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_themes_parse() {
        let set: ThemeSet =
            serde_json::from_str(WRITER_THEMES).expect("themes/writer.json 파싱 실패");
        assert!(
            set.themes.iter().any(|theme| theme.name == DEFAULT_DARK),
            "{DEFAULT_DARK} 테마가 없습니다"
        );
        assert!(
            set.themes.iter().any(|theme| theme.name == DEFAULT_LIGHT),
            "{DEFAULT_LIGHT} 테마가 없습니다"
        );
    }
}
