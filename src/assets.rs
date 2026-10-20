//! 앱 에셋 소스: gpui-kit 전체 카탈로그(`AllAssets`)를 감싸 Lucide 아이콘의 선 두께를 조정한다.
//!
//! Lucide 원본은 24px 기준 `stroke-width="2"`라 16px 안팎에서는 굵어 보인다.
//! 로드 시점에 선 두께만 바꾸고, 나머지 에셋(창 컨트롤 등 `stroke-width="2"`가 없는 SVG)은 그대로 넘긴다.

use std::borrow::Cow;

use gpui_kit::assets::AllAssets;
use gpui_kit::gpui::{AssetSource, Result, SharedString};

/// 아이콘 선 두께 (Lucide 원본 2.0).
pub const ICON_STROKE_WIDTH: &str = "1.5";

const LUCIDE_STROKE: &str = r#"stroke-width="2""#;

/// 앱에 등록하는 에셋 소스.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let asset = AllAssets.load(path)?;
        if !path.ends_with(".svg") {
            return Ok(asset);
        }
        Ok(asset.map(adjust_stroke))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        AllAssets.list(path)
    }
}

/// SVG의 Lucide 기본 선 두께를 `ICON_STROKE_WIDTH`로 바꾼다. 해당 속성이 없으면 원본을 그대로 돌려준다.
fn adjust_stroke(svg: Cow<'static, [u8]>) -> Cow<'static, [u8]> {
    let Ok(text) = std::str::from_utf8(&svg) else {
        return svg;
    };
    if !text.contains(LUCIDE_STROKE) {
        return svg;
    }
    let replaced = text.replace(
        LUCIDE_STROKE,
        &format!(r#"stroke-width="{ICON_STROKE_WIDTH}""#),
    );
    Cow::Owned(replaced.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lucide_icon_stroke_is_adjusted() {
        let svg = Assets
            .load("icons/folder.svg")
            .unwrap()
            .expect("folder 아이콘 없음");
        let text = std::str::from_utf8(&svg).unwrap();
        assert!(text.contains(&format!(r#"stroke-width="{ICON_STROKE_WIDTH}""#)));
        assert!(!text.contains(LUCIDE_STROKE));
    }

    #[test]
    fn svg_without_lucide_stroke_is_untouched() {
        let original = Cow::Borrowed(br#"<svg fill="currentColor"/>"#.as_slice());
        assert!(matches!(adjust_stroke(original), Cow::Borrowed(_)));
    }
}
