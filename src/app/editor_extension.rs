//! 에디터의 확장 블록을 캔버스로 그린다(ADR-0026).
//!
//! `` ```ext <이름> `` 펜스는 그대로 고칠 수 있는 글자로 두고, 닫는 줄 아래에 gpui-base 줄 블록으로
//! 확장의 `draw`가 그린 모양을 그린다. 펜스 줄은 커서가 블록에 닿을 때만 보인다(표와 같다, ADR-0017).
//! 캔버스는 본문 폭보다 크면 비율을 지켜 줄이고 가운데에 둔다. `animated` 확장은 보이는 동안 매
//! 프레임 다시 그린다. 확장을 읽거나 그리다 실패하면 블록 자리에 까닭을 보인다.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::*;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use super::style::Palette;
use crate::extension::{self, Host, Segment, Shape};

/// 캔버스 위·아래 여백(이미지 블록과 같다).
const PADDING_TOP: f32 = 4.0;
const PADDING_BOTTOM: f32 = 16.0;

/// 오류 안내 줄 높이.
const ERROR_HEIGHT: f32 = 28.0;

/// 본문의 확장 블록 하나.
struct EditorBlock {
    block: extension::Block,
    /// 처음 본 때. 애니메이션의 `t`는 여기서 잰다.
    started: Instant,
    /// 정지 그림(`animated`가 아닌 확장): 그린 확장과 모양. 확장이 다시 읽히면 새로 그린다.
    still: RefCell<Option<Still>>,
}

/// 정지 그림: 그린 확장, 그때의 테마, 결과.
type Still = (Rc<extension::Loaded>, Theme, Result<Rc<Vec<Shape>>, String>);

type Theme = extension::Theme;

/// 열린 시트의 확장 블록 목록. 줄 스타일·줄 블록의 측정·그리기에서 함께 쓴다.
pub(super) struct ExtensionBlocks {
    host: Host,
    /// 확장 폴더의 파일과 수정 시각(마지막으로 본 것).
    files: RefCell<Vec<(String, Option<std::time::SystemTime>)>>,
    /// 줄 순서대로.
    blocks: RefCell<Vec<Rc<EditorBlock>>>,
}

impl ExtensionBlocks {
    pub(super) fn new(text: &str, host: Host) -> Self {
        let blocks = Self {
            files: RefCell::new(extension::scan(host.root())),
            host,
            blocks: RefCell::default(),
        };
        blocks.sync(text);
        blocks
    }

    /// 본문에 맞춰 블록을 다시 찾는다. 바뀐 블록이 닿았던 줄과 닿는 줄을 돌려준다(다시 배치할 줄).
    pub(super) fn sync(&self, text: &str) -> Vec<Range<usize>> {
        let old = self.blocks.take();
        let new: Vec<Rc<EditorBlock>> = extension::blocks(text)
            .into_iter()
            .map(|block| {
                old.iter()
                    .find(|kept| kept.block == block)
                    .cloned()
                    .unwrap_or_else(|| {
                        Rc::new(EditorBlock {
                            block,
                            started: Instant::now(),
                            still: RefCell::default(),
                        })
                    })
            })
            .collect();
        let changed = |from: &[Rc<EditorBlock>], to: &[Rc<EditorBlock>]| {
            from.iter()
                .filter(|block| !to.iter().any(|other| Rc::ptr_eq(block, other)))
                .map(|block| block.block.rows.clone())
                .collect::<Vec<_>>()
        };
        let mut rows = changed(&old, &new);
        rows.extend(changed(&new, &old));
        self.blocks.replace(new);
        rows
    }

    /// 확장 폴더가 지난번에 본 뒤로 바뀌었는지.
    pub(super) fn files_changed(&self) -> bool {
        let files = extension::scan(self.host.root());
        if *self.files.borrow() == files {
            return false;
        }
        self.files.replace(files);
        true
    }

    /// 줄이 확장 블록(펜스 포함)인지.
    pub(super) fn contains(&self, row: usize) -> bool {
        self.block_at(row).is_some()
    }

    /// 줄 아래 캔버스 높이. 블록의 닫는 줄이 아니면 0이다.
    pub(super) fn measure(&self, row: usize, wrap_width: Option<Pixels>) -> Pixels {
        let Some(block) = self.last_row_block(row) else {
            return px(0.);
        };
        let width = wrap_width.unwrap_or(px(super::editor::EDITOR_MAX_WIDTH));
        match &*self.host.get(&block.block.name) {
            Ok(extension) => {
                let (_, height) = fit(extension.size, width);
                height + px(PADDING_TOP + PADDING_BOTTOM)
            }
            Err(_) => px(ERROR_HEIGHT + PADDING_TOP + PADDING_BOTTOM),
        }
    }

    /// 블록 닫는 줄 아래에 캔버스를 그린다. 블록이 아니면 `None`이다.
    pub(super) fn render(
        &self,
        row: usize,
        size: Size<Pixels>,
        window: &mut Window,
        cx: &App,
    ) -> Option<AnyElement> {
        let block = self.last_row_block(row)?;
        let pal = Palette::from_theme(cx.theme());
        let loaded = self.host.get(&block.block.name);
        let extension = match &*loaded {
            Ok(extension) => extension,
            Err(err) => return Some(error_box(&block.block.name, err, size, pal)),
        };
        let theme = theme(pal);
        let shapes = if extension.animated {
            window.request_animation_frame();
            let t = block.started.elapsed().as_secs_f32();
            extension.draw(t, &block.block.args, &theme).map(Rc::new)
        } else {
            let mut still = block.still.borrow_mut();
            let fresh = still.as_ref().is_some_and(|(drawn, drawn_theme, _)| {
                Rc::ptr_eq(drawn, &loaded) && *drawn_theme == theme
            });
            if !fresh {
                let shapes = extension.draw(0.0, &block.block.args, &theme).map(Rc::new);
                *still = Some((loaded.clone(), theme, shapes));
            }
            still.as_ref().map(|(_, _, shapes)| shapes.clone())?
        };
        let shapes = match shapes {
            Ok(shapes) => shapes,
            Err(err) => return Some(error_box(&block.block.name, &err, size, pal)),
        };
        let (width, height) = fit(extension.size, size.width);
        let scale = f32::from(width) / extension.size.0;
        let left = (size.width - width) / 2.;
        Some(
            div()
                .w(size.width)
                .h(size.height)
                .pt(px(PADDING_TOP))
                .pl(left)
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, (), window, cx| {
                            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                                paint(&shapes, bounds.origin, scale, window, cx);
                            });
                        },
                    )
                    .w(width)
                    .h(height),
                )
                .into_any_element(),
        )
    }

    fn block_at(&self, row: usize) -> Option<Rc<EditorBlock>> {
        let blocks = self.blocks.borrow();
        let ix = blocks.partition_point(|block| block.block.rows.end <= row);
        blocks
            .get(ix)
            .filter(|block| block.block.rows.contains(&row))
            .cloned()
    }

    fn last_row_block(&self, row: usize) -> Option<Rc<EditorBlock>> {
        self.block_at(row)
            .filter(|block| block.block.rows.end == row + 1)
    }
}

/// 캔버스 크기를 폭 `width` 안에 맞춘다(키우지는 않는다).
fn fit((w, h): (f32, f32), width: Pixels) -> (Pixels, Pixels) {
    let scale = (f32::from(width) / w).min(1.0);
    (px(w * scale), px(h * scale))
}

fn theme(pal: Palette) -> Theme {
    let color = |hsla: Hsla| {
        let rgba = Rgba::from(hsla);
        [rgba.r, rgba.g, rgba.b, rgba.a]
    };
    Theme {
        foreground: color(pal.foreground),
        background: color(pal.background),
        muted: color(pal.muted),
        accent: color(pal.accent),
        border: color(pal.border),
    }
}

fn hsla([r, g, b, a]: extension::Color) -> Hsla {
    Rgba { r, g, b, a }.into()
}

fn error_box(name: &str, err: &str, size: Size<Pixels>, pal: Palette) -> AnyElement {
    div()
        .w(size.width)
        .h(size.height)
        .pt(px(PADDING_TOP))
        .child(
            div()
                .h(px(ERROR_HEIGHT))
                .px_2()
                .flex()
                .items_center()
                .rounded_md()
                .bg(pal.tint)
                .text_xs()
                .text_color(pal.danger)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(format!("확장 {name}: {err}")),
        )
        .into_any_element()
}

fn paint(shapes: &[Shape], origin: Point<Pixels>, scale: f32, window: &mut Window, cx: &mut App) {
    let at = |[x, y]: [f32; 2]| point(origin.x + px(x * scale), origin.y + px(y * scale));
    let build = |mut builder: PathBuilder, path: &[Segment]| {
        for segment in path {
            match *segment {
                Segment::Move(p) => builder.move_to(at(p)),
                Segment::Line(p) => builder.line_to(at(p)),
                Segment::Cubic(a, b, p) => builder.cubic_bezier_to(at(p), at(a), at(b)),
                Segment::Close => builder.close(),
            }
        }
        builder.build().ok()
    };
    let font = window.text_style().font();
    for shape in shapes {
        match shape {
            Shape::Fill { path, color } => {
                if let Some(path) = build(PathBuilder::fill(), path) {
                    window.paint_path(path, hsla(*color));
                }
            }
            Shape::Stroke { path, width, color } => {
                if *width <= 0.0 {
                    continue;
                }
                if let Some(path) = build(PathBuilder::stroke(px(width * scale)), path) {
                    window.paint_path(path, hsla(*color));
                }
            }
            Shape::Text {
                text,
                origin: text_origin,
                size,
                color,
                align,
                width,
            } => {
                if text.is_empty() {
                    continue;
                }
                let font_size = px(size * scale);
                let run = TextRun {
                    len: text.len(),
                    font: font.clone(),
                    color: hsla(*color),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line =
                    window
                        .text_system()
                        .shape_line(text.clone().into(), font_size, &[run], None);
                let align = match align {
                    extension::Align::Left => TextAlign::Left,
                    extension::Align::Center => TextAlign::Center,
                    extension::Align::Right => TextAlign::Right,
                };
                line.paint(
                    at(*text_origin),
                    font_size * extension::canvas::LINE_HEIGHT,
                    align,
                    width.map(|width| px(width * scale)),
                    window,
                    cx,
                )
                .ok();
            }
        }
    }
}
