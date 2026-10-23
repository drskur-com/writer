//! UI 언어(ADR-0027): 영어와 한국어. 문자열은 `locales/<언어>/<영역>.<언어>.yml`에 두고 `t!`로 읽는다.
//!
//! 언어는 설정에서 고르고, 고르지 않으면 시스템 언어를 따른다(한국어가 아니면 영어).
//! 바꾸면 바로 메뉴를 다시 설치하고 창을 다시 그린다. 모델이 읽는 글(도구 설명·시스템 프롬프트)과
//! 책 내용(내보내기의 목차 같은 글, 책의 `language`를 따른다)은 이 설정과 상관없다.

use std::borrow::Cow;

use gpui_kit::gpui::App;

/// 고를 수 있는 언어.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    /// 시스템 언어를 따른다.
    #[default]
    System,
    English,
    Korean,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::System, Language::English, Language::Korean];

    /// `config.toml`에 적는 이름. 시스템은 적지 않는다.
    pub fn key(self) -> Option<&'static str> {
        match self {
            Language::System => None,
            Language::English => Some("en"),
            Language::Korean => Some("ko"),
        }
    }

    pub fn from_key(key: Option<&str>) -> Self {
        match key {
            Some("en") => Language::English,
            Some("ko") => Language::Korean,
            _ => Language::System,
        }
    }

    /// 선택지 이름. 언어 이름은 그 언어로 적는다.
    pub fn title(self) -> Cow<'static, str> {
        match self {
            Language::System => t!("settings.language.system"),
            Language::English => "English".into(),
            Language::Korean => "한국어".into(),
        }
    }

    /// 실제로 쓸 로캘(`en`·`ko`).
    pub fn locale(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Korean => "ko",
            Language::System => system_locale(),
        }
    }
}

/// 시스템 언어. 한국어가 아니면 영어.
fn system_locale() -> &'static str {
    match sys_locale::get_locale() {
        Some(locale) if locale.to_lowercase().starts_with("ko") => "ko",
        _ => "en",
    }
}

/// 언어를 적용한다. 메뉴를 다시 설치하고 창을 다시 그린다.
pub fn apply(language: Language, cx: &mut App) {
    rust_i18n::set_locale(language.locale());
    crate::menus::install_menus(cx);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 로캘마다 모든 키에 번역이 있는지 본다. 없는 키는 영어로 떨어지거나 키 이름이 그대로 보인다.
    #[test]
    fn every_key_has_english_and_korean() {
        let backend = crate::_rust_i18n_backend();
        let keys = |locale: &str| -> std::collections::BTreeSet<String> {
            backend
                .messages_for_locale(locale)
                .unwrap_or_default()
                .into_iter()
                .map(|(key, _)| key.into_owned())
                .collect()
        };
        let (en, ko) = (keys("en"), keys("ko"));
        assert!(!en.is_empty());
        assert_eq!(
            en.difference(&ko).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "ko 없음"
        );
        assert_eq!(
            ko.difference(&en).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "en 없음"
        );
        assert_eq!(t!("menu.file", locale = "en"), "File");
        assert_eq!(t!("menu.file", locale = "ko"), "파일");
        assert_eq!(
            t!(
                "settings.ai.region.hint_profile",
                locale = "ko",
                region = "us-east-1"
            ),
            "프로필 리전 us-east-1을 씁니다."
        );
    }

    #[test]
    fn language_keys_round_trip() {
        for language in Language::ALL {
            assert_eq!(Language::from_key(language.key()), language);
        }
    }
}
