//! 에디터의 이미지 줄 아래에 이미지를 그린다(ADR-0008).
//!
//! 이미지만 있는 줄(`![캡션](경로){속성}`, ADR-0007)은 그대로 고칠 수 있는 글자로 두고,
//! 그 아래에 gpui-base 줄 블록(`EditorState::set_line_blocks`)으로 이미지와 캡션을 그린다.
//! 이미지 줄 글자는 커서가 그 줄에 들어올 때만 보인다(`LineStyle::collapsed`).
//! 크기·정렬은 속성을 따르고, 없으면 원본 크기를 본문 폭 안에 맞춘다.

use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::editor::EDITOR_MAX_WIDTH;
use super::style::Palette;
use crate::markdown::{self, ImageAlign, ImageOptions};
use crate::{icons, library};

/// 이미지 위 여백.
const PADDING_TOP: f32 = 4.0;

/// 이미지 아래 여백(다음 줄과 띄운다).
const PADDING_BOTTOM: f32 = 16.0;

/// 크기를 적지 않은 이미지의 최대 높이. 세로로 긴 사진이 화면을 다 차지하지 않게 한다.
const MAX_AUTO_HEIGHT: f32 = 480.0;

/// 비율을 모르는 이미지(웹 주소)를 그릴 칸 높이.
const UNKNOWN_HEIGHT: f32 = 240.0;

/// 이미지를 칸보다 크게 그려 잘라 내는 폭(가장자리 번짐 숨김, `render` 참고).
const EDGE_BLEED: f32 = 1.0;

/// 이미지 아래 캡션 줄 높이.
const CAPTION_HEIGHT: f32 = 24.0;

/// 이미지를 찾지 못했을 때 안내 줄 높이.
const MISSING_HEIGHT: f32 = 28.0;

/// 로컬 이미지 파일 정보. 헤더만 읽는다.
#[derive(Clone, Copy)]
struct ImageFile {
    width: f32,
    height: f32,
    modified: u128,
}

/// 줄 하나에 그릴 이미지.
enum Source {
    /// 파일, 수정 시각, 썸네일 긴 변.
    File(PathBuf, u128, Pixels),
    Url(SharedUri),
    /// 파일이 없거나 이미지로 읽을 수 없다. 적힌 경로를 보여 준다.
    Missing(String),
}

struct Placement {
    source: Source,
    /// 이미지 크기(여백 제외).
    size: Size<Pixels>,
    align: ImageAlign,
    /// 이미지 아래에 보일 캡션(alt). 비었거나 이미지를 못 찾았으면 `None`.
    caption: Option<SharedString>,
}

/// 열린 시트의 이미지 줄 판정·배치. 에디터 줄 블록의 측정·그리기에서 함께 쓴다.
pub(super) struct ImageBlocks {
    /// 상대 경로의 기준(시트가 든 폴더).
    dir: PathBuf,
    /// 읽은 이미지 헤더. 키 입력마다 같은 줄을 다시 재므로 파일을 매번 열지 않게 한다.
    /// 없는 파일은 남기지 않는다(나중에 생기면 바로 보이도록).
    files: RefCell<HashMap<PathBuf, ImageFile>>,
}

impl ImageBlocks {
    pub(super) fn new(sheet: &Path) -> Self {
        Self {
            dir: sheet.parent().map(Path::to_path_buf).unwrap_or_default(),
            files: RefCell::default(),
        }
    }

    /// 줄 아래에 둘 블록 높이. 이미지 줄이 아니면 0.
    pub(super) fn measure(&self, line: &str, wrap_width: Option<Pixels>) -> Pixels {
        let width = wrap_width.unwrap_or(px(EDITOR_MAX_WIDTH));
        match self.place(line, width) {
            Some(placement) => {
                let caption = if placement.caption.is_some() {
                    CAPTION_HEIGHT
                } else {
                    0.0
                };
                placement.size.height + px(PADDING_TOP + caption + PADDING_BOTTOM)
            }
            None => px(0.),
        }
    }

    /// 줄 아래 블록에 이미지를 그린다.
    pub(super) fn render(&self, line: &str, size: Size<Pixels>, cx: &App) -> AnyElement {
        let Some(placement) = self.place(line, size.width) else {
            return Empty.into_any_element();
        };
        let pal = Palette::from_theme(cx.theme());
        let image = match placement.source {
            Source::Missing(destination) => div()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(pal.muted)
                .child(Icon::new(icons::image()).small())
                .child(t!("editor.image.not_found", path = destination))
                .into_any_element(),
            source => {
                // 파일은 칸 비율이 이미지 비율과 같아 가장자리까지 채우고, 웹 이미지는 비율을 몰라
                // 칸 안에 맞춘다.
                let (source, fit) = match source {
                    Source::File(path, modified, long_side) => (
                        crate::thumbnail::source(path, modified, long_side),
                        ObjectFit::Cover,
                    ),
                    Source::Url(uri) => (uri.into(), ObjectFit::Contain),
                    Source::Missing(_) => unreachable!(),
                };
                // gpui는 아틀라스에서 이미지를 선형 보간으로 읽는데 타일 경계에서 멈추지 않는다.
                // 그래서 줄이거나 소수 픽셀 위치(줄 높이 25.6px)에 그리면 오른쪽·아래 가장자리에
                // 옆 타일 색이 섞여 가는 테두리처럼 보인다. 칸보다 조금 크게 그리고 칸에서 잘라
                // 그 가장자리를 숨긴다.
                let bleed = px(EDGE_BLEED);
                div()
                    .relative()
                    .flex_shrink_0()
                    .w(placement.size.width)
                    .h(placement.size.height)
                    .overflow_hidden()
                    .rounded(px(4.))
                    .child(
                        img(source)
                            .absolute()
                            .left(-bleed)
                            .top(-bleed)
                            .w(placement.size.width + bleed * 2.)
                            .h(placement.size.height + bleed * 2.)
                            .object_fit(fit)
                            .rounded(px(4.))
                            .with_fallback(move || {
                                Icon::new(icons::image())
                                    .text_color(pal.muted)
                                    .into_any_element()
                            }),
                    )
                    .into_any_element()
            }
        };
        div()
            .w(size.width)
            .h(size.height)
            .pt(px(PADDING_TOP))
            .flex()
            .flex_col()
            .map(|this| match placement.align {
                ImageAlign::Left => this.items_start(),
                ImageAlign::Center => this.items_center(),
                ImageAlign::Right => this.items_end(),
            })
            .child(image)
            .when_some(placement.caption, |this, caption| {
                this.child(
                    div()
                        .h(px(CAPTION_HEIGHT))
                        .max_w(placement.size.width.max(px(240.)))
                        .flex()
                        .items_end()
                        .text_sm()
                        .text_color(pal.muted)
                        .truncate()
                        .child(caption),
                )
            })
            .into_any_element()
    }

    /// 이미지 줄이면 그릴 이미지와 크기를 정한다. `width`는 본문 폭.
    fn place(&self, line: &str, width: Pixels) -> Option<Placement> {
        let image = markdown::image_line(line)?;
        let destination = image.destination;
        let caption = (!image.alt.trim().is_empty()).then(|| image.alt.trim().to_string().into());
        if library::is_web_address(&destination) {
            return Some(Placement {
                size: image_size(None, &image.options, width),
                source: Source::Url(destination.into()),
                align: image.options.align,
                caption,
            });
        }
        let resolved = library::resolve_image_path(&self.dir, None, &destination)
            .and_then(|path| self.file(&path).map(|file| (path, file)));
        let Some((path, file)) = resolved else {
            return Some(Placement {
                source: Source::Missing(destination),
                size: size(width, px(MISSING_HEIGHT)),
                align: ImageAlign::Left,
                caption: None,
            });
        };
        let natural = size(px(file.width), px(file.height));
        // 화면에 그릴 크기로 줄인 썸네일을 쓴다(큰 사진을 통째로 디코딩하지 않음).
        // 크기는 본문 최대 폭 기준으로 고정해 창 폭이 바뀔 때마다 새로 만들지 않는다.
        let largest = image_size(Some(natural), &image.options, px(EDITOR_MAX_WIDTH));
        Some(Placement {
            size: image_size(Some(natural), &image.options, width),
            source: Source::File(path, file.modified, largest.width.max(largest.height)),
            align: image.options.align,
            caption,
        })
    }

    fn file(&self, path: &Path) -> Option<ImageFile> {
        if let Some(file) = self.files.borrow().get(path) {
            return Some(*file);
        }
        let size = imagesize::size(path).ok()?;
        if size.width == 0 || size.height == 0 {
            return None;
        }
        let file = ImageFile {
            width: size.width as f32,
            height: size.height as f32,
            modified: library::modified_nanos(path),
        };
        self.files.borrow_mut().insert(path.to_path_buf(), file);
        Some(file)
    }
}

/// 그릴 크기. `natural`은 원본 크기(모르면 `None`), `max_width`는 본문 폭.
///
/// - 크기를 둘 다 적었으면 그대로, 하나만 적었으면 원본 비율로 나머지를 정한다.
/// - 한 페이지 가득은 본문 폭에 맞춘다.
/// - 아무것도 적지 않았으면 원본 크기. 너무 크면 `MAX_AUTO_HEIGHT` 안으로 줄인다.
/// - 어느 경우든 본문 폭을 넘으면 비율을 지켜 줄인다.
fn image_size(
    natural: Option<Size<Pixels>>,
    options: &ImageOptions,
    max_width: Pixels,
) -> Size<Pixels> {
    let max_width = max_width.max(px(1.));
    let Some(natural) = natural else {
        // 비율을 모르면 정해진 칸에 맞춰 그린다(`object_fit(Contain)`).
        let width = options.width.map_or(max_width, |width| px(width as f32));
        let height = options
            .height
            .map_or(px(UNKNOWN_HEIGHT), |height| px(height as f32));
        return size(width.min(max_width), height);
    };
    let aspect = natural.height / natural.width;
    let (mut width, mut height, auto) = if options.full_page {
        (max_width, max_width * aspect, true)
    } else {
        match (options.width, options.height) {
            (Some(width), Some(height)) => (px(width as f32), px(height as f32), false),
            (Some(width), None) => (px(width as f32), px(width as f32) * aspect, false),
            (None, Some(height)) => (px(height as f32) / aspect, px(height as f32), false),
            (None, None) => (natural.width, natural.height, true),
        }
    };
    if width > max_width {
        height *= max_width / width;
        width = max_width;
    }
    if auto && height > px(MAX_AUTO_HEIGHT) {
        width *= px(MAX_AUTO_HEIGHT) / height;
        height = px(MAX_AUTO_HEIGHT);
    }
    size(width.round().max(px(1.)), height.round().max(px(1.)))
}

#[cfg(test)]
mod tests {
    // `gpui::*`의 `test` 매크로와 겹치지 않도록 하나씩 가져온다.
    use super::{
        CAPTION_HEIGHT, ImageBlocks, ImageOptions, MISSING_HEIGHT, PADDING_BOTTOM, PADDING_TOP,
        UNKNOWN_HEIGHT, image_size,
    };
    use gpui_kit::gpui::{px, size};

    fn options(width: Option<u32>, height: Option<u32>, full_page: bool) -> ImageOptions {
        ImageOptions {
            width,
            height,
            full_page,
            ..Default::default()
        }
    }

    #[test]
    fn image_size_follows_options_and_fits_the_text_width() {
        let photo = Some(size(px(1200.), px(800.)));
        let max = px(600.);
        // 자동: 본문 폭에 맞춘다.
        assert_eq!(
            image_size(photo, &options(None, None, false), max),
            size(px(600.), px(400.))
        );
        // 작은 이미지는 원본 크기.
        let icon = Some(size(px(64.), px(32.)));
        assert_eq!(
            image_size(icon, &options(None, None, false), max),
            size(px(64.), px(32.))
        );
        // 하나만 적으면 원본 비율.
        assert_eq!(
            image_size(photo, &options(Some(300), None, false), max),
            size(px(300.), px(200.))
        );
        assert_eq!(
            image_size(photo, &options(None, Some(100), false), max),
            size(px(150.), px(100.))
        );
        // 둘 다 적으면 그대로, 폭을 넘으면 비율대로 줄인다.
        assert_eq!(
            image_size(photo, &options(Some(300), Some(300), false), max),
            size(px(300.), px(300.))
        );
        assert_eq!(
            image_size(photo, &options(Some(1200), Some(300), false), max),
            size(px(600.), px(150.))
        );
        // 한 페이지 가득은 본문 폭.
        assert_eq!(
            image_size(icon, &options(None, None, true), max),
            size(px(600.), px(300.))
        );
        // 세로로 긴 자동 이미지는 높이를 제한한다.
        let tall = Some(size(px(400.), px(1600.)));
        assert_eq!(
            image_size(tall, &options(None, None, false), max),
            size(px(120.), px(480.))
        );
        // 비율을 모르면 정해진 칸.
        assert_eq!(
            image_size(None, &options(None, None, false), max),
            size(max, px(UNKNOWN_HEIGHT))
        );
    }

    #[test]
    fn measure_only_image_lines() {
        let dir = std::env::temp_dir().join(format!("writer-editor-image-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Extras")).unwrap();
        image::RgbImage::new(200, 100)
            .save(dir.join("Extras/a.png"))
            .unwrap();
        let blocks = ImageBlocks::new(&dir.join("sheet.md"));
        let padding = px(PADDING_TOP + PADDING_BOTTOM);
        assert_eq!(blocks.measure("본문", Some(px(600.))), px(0.));
        assert_eq!(
            blocks.measure("![](Extras/a.png)", Some(px(600.))),
            px(100.) + padding
        );
        assert_eq!(
            blocks.measure("![](Extras/a.png){width=100px}", Some(px(600.))),
            px(50.) + padding
        );
        // 캡션이 있으면 그 줄만큼 더 높다.
        assert_eq!(
            blocks.measure("![지도](Extras/a.png){width=100px}", Some(px(600.))),
            px(50. + CAPTION_HEIGHT) + padding
        );
        assert_eq!(
            blocks.measure("![](없음.png)", Some(px(600.))),
            px(MISSING_HEIGHT) + padding
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
