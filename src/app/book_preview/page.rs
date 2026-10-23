//! 배치한 페이지 하나를 그린다. 글자·선은 canvas로, 이미지는 그 위에 겹친 `img`로 그린다.

use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::gpui::*;
use std::rc::Rc;

use super::layout::{BookLayout, ImageFile, Item, MUTED_INK};
use crate::icons;

/// 종이 색. 앱 테마와 상관없이 책은 흰 종이에 검은 글자다.
const PAPER: Hsla = Hsla {
    h: 0.,
    s: 0.,
    l: 1.,
    a: 1.,
};

/// 이미지를 칸보다 크게 그려 잘라 내는 폭(가장자리 번짐 숨김).
const EDGE_BLEED: f32 = 1.0;

/// `index` 페이지를 `scale`배로 그린다.
pub(super) fn page(layout: Rc<BookLayout>, index: usize, scale: f32) -> AnyElement {
    let setup = layout.setup;
    let Some(page) = layout.pages.get(index) else {
        return Empty.into_any_element();
    };
    let scaled = move |bounds: Bounds<Pixels>| {
        Bounds::new(
            point(bounds.origin.x * scale, bounds.origin.y * scale),
            size(bounds.size.width * scale, bounds.size.height * scale),
        )
    };
    let images: Vec<AnyElement> = page
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Image {
                bounds,
                source,
                cover,
            } => {
                // 소수 픽셀 위치에 그리면 가장자리가 흐려지므로 정수 픽셀로 맞춘다.
                let bounds = scaled(*bounds);
                let bounds = Bounds::new(
                    point(bounds.origin.x.round(), bounds.origin.y.round()),
                    size(bounds.size.width.round(), bounds.size.height.round()),
                );
                let fit = if *cover {
                    ObjectFit::Cover
                } else {
                    ObjectFit::Contain
                };
                let source: ImageSource = match source {
                    ImageFile::File { path, modified } => crate::thumbnail::source(
                        path.clone(),
                        *modified,
                        bounds.size.width.max(bounds.size.height),
                    ),
                    ImageFile::Url(uri) => uri.clone().into(),
                };
                Some(
                    div()
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .overflow_hidden()
                        // gpui는 아틀라스에서 이미지를 선형 보간으로 읽어 오른쪽·아래 가장자리에 옆 타일
                        // 색이 섞인다(회색 테두리처럼 보임). 칸보다 조금 크게 그리고 칸에서 잘라 숨긴다
                        // (`editor_image.rs`와 같은 방법).
                        .child(
                            img(source)
                                .absolute()
                                .left(-px(EDGE_BLEED))
                                .top(-px(EDGE_BLEED))
                                .w(bounds.size.width + px(EDGE_BLEED * 2.))
                                .h(bounds.size.height + px(EDGE_BLEED * 2.))
                                .object_fit(fit)
                                .with_fallback(|| {
                                    div()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Icon::new(icons::image()).small().text_color(MUTED_INK),
                                        )
                                        .into_any_element()
                                }),
                        )
                        .into_any_element(),
                )
            }
            _ => None,
        })
        .collect();

    div()
        .relative()
        .flex_shrink_0()
        .w(setup.width * scale)
        .h(setup.height * scale)
        .bg(PAPER)
        .shadow_md()
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let Some(page) = layout.pages.get(index) else {
                        return;
                    };
                    let origin = bounds.origin;
                    for item in &page.items {
                        match item {
                            Item::Text {
                                origin: at,
                                line_height,
                                font_size,
                                text,
                                runs,
                            } => {
                                let line = window.text_system().shape_line(
                                    text.clone(),
                                    *font_size * scale,
                                    runs,
                                    None,
                                );
                                line.paint(
                                    origin + point(at.x * scale, at.y * scale),
                                    *line_height * scale,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                )
                                .ok();
                            }
                            Item::Rule {
                                bounds: rule,
                                color,
                            } => {
                                let rule = scaled(*rule);
                                let rule = Bounds::new(
                                    origin + rule.origin,
                                    size(rule.size.width, rule.size.height.max(px(1.))),
                                );
                                window.paint_quad(fill(rule, *color));
                            }
                            Item::Image { .. } => {}
                            Item::Drawing { bounds, drawing } => {
                                let bounds = scaled(*bounds);
                                let factor = bounds.size.width / px(drawing.still.canvas.0);
                                // 캔버스 밖은 에디터처럼 자른다.
                                let mask = ContentMask {
                                    bounds: Bounds::new(origin + bounds.origin, bounds.size),
                                };
                                window.with_content_mask(Some(mask), |window| {
                                    crate::app::extension_painter::paint(
                                        &drawing.still.shapes,
                                        origin + bounds.origin,
                                        factor,
                                        &drawing.cache,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        }
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .children(images)
        .into_any_element()
}
