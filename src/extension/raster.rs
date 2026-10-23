//! 모양 목록을 PNG로 그린다(화면과 같은 [`super::render`]). 어시스턴트의 `run_extension`이 그린 결과를 모델에 보여 준다.
//!
//! - 캔버스 밖은 잘린다(에디터와 같다). 배경은 테마 배경색이다.
//! - 프레임이 여럿이면 격자로 놓고 프레임마다 `t = …` 이름표를 붙인다.

use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, Rect};

use super::canvas::{Align, Color, Shape};
use super::render::{Renderer, draw_cell};

/// 이미지 긴 변의 상한(모델이 줄이지 않고 받는 크기).
pub const MAX_SIDE: f32 = 1568.0;
/// 작은 캔버스를 키우는 최대 배율(선과 글자가 또렷하게 보이도록).
const MAX_SCALE: f32 = 2.0;
/// 격자에서 프레임 사이 간격과 이름표 높이(캔버스 좌표).
const GAP: f32 = 12.0;
const LABEL: f32 = 18.0;

/// 한 프레임: 이름표(여럿일 때만 그린다)와 모양 목록.
pub struct Frame<'a> {
    pub label: String,
    pub shapes: &'a [Shape],
}

/// 프레임들을 PNG로 그린다. 모든 프레임은 `width` × `height` 캔버스다.
pub fn render_png(
    frames: &[Frame],
    width: f32,
    height: f32,
    background: Color,
) -> Result<Vec<u8>, String> {
    render_png_at(frames, width, height, background, MAX_SIDE, MAX_SCALE)
}

/// [`render_png`]를 긴 변 `max_side`(px), 배율 `max_scale`까지로 그린다(인쇄용 내보내기는 더 크게).
pub fn render_png_at(
    frames: &[Frame],
    width: f32,
    height: f32,
    background: Color,
    max_side: f32,
    max_scale: f32,
) -> Result<Vec<u8>, String> {
    if frames.is_empty() || width <= 0.0 || height <= 0.0 {
        return Err("Nothing to render.".into());
    }
    let labeled = frames.len() > 1;
    let cell_h = height + if labeled { LABEL } else { 0.0 };
    let (cols, rows) = grid(frames.len(), width, cell_h);
    let total_w = cols as f32 * width + (cols - 1) as f32 * GAP;
    let total_h = rows as f32 * cell_h + (rows - 1) as f32 * GAP;
    let scale = (max_side / total_w.max(total_h)).min(max_scale);
    let px = |v: f32| ((v * scale).ceil() as u32).max(1);
    let pixmap = Renderer::new().render_cells(px(total_w), px(total_h), |ctx, fonts| {
        // 프레임 사이는 회색으로 두어 경계가 보이게 한다.
        let gap = if labeled {
            [200.0 / 255.0, 200.0 / 255.0, 200.0 / 255.0, 1.0]
        } else {
            background
        };
        ctx.set_paint(rgba(gap));
        ctx.fill_rect(&Rect::new(
            0.0,
            0.0,
            f64::from(px(total_w)),
            f64::from(px(total_h)),
        ));
        for (ix, frame) in frames.iter().enumerate() {
            let (col, row) = (ix % cols, ix / cols);
            let x = col as f32 * (width + GAP);
            let y = row as f32 * (cell_h + GAP);
            let at = |dy: f32| {
                Affine::translate((
                    (x * scale).round() as f64,
                    ((y + dy) * scale).round() as f64,
                )) * Affine::scale(f64::from(scale))
            };
            let top = if labeled {
                let text = Shape::Text {
                    text: frame.label.clone(),
                    origin: [4.0, 2.0],
                    size: 12.0,
                    color: [0.25, 0.25, 0.25, 1.0],
                    align: Align::Left,
                    width: None,
                };
                let label = [235.0 / 255.0, 235.0 / 255.0, 235.0 / 255.0, 1.0];
                draw_cell(
                    ctx,
                    fonts,
                    std::slice::from_ref(&text),
                    at(0.0),
                    [width, LABEL],
                    Some(label),
                );
                LABEL
            } else {
                0.0
            };
            draw_cell(
                ctx,
                fonts,
                frame.shapes,
                at(top),
                [width, height],
                Some(background),
            );
        }
    });
    let (w, h) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    let data = pixmap.take_rgba8(vello_cpu::peniko::ImageAlphaType::Alpha);
    let image = image::RgbaImage::from_raw(w, h, data).ok_or("The canvas is too large.")?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|err| err.to_string())?;
    Ok(png)
}

fn rgba([r, g, b, a]: Color) -> AlphaColor<Srgb> {
    AlphaColor::new([r, g, b, a])
}

/// 프레임 `n`개를 놓을 (열, 행): 전체의 긴 변이 가장 짧은 것.
fn grid(n: usize, width: f32, height: f32) -> (usize, usize) {
    (1..=n)
        .map(|cols| (cols, n.div_ceil(cols)))
        .min_by(|a, b| {
            let side = |(cols, rows): (usize, usize)| {
                let w = cols as f32 * (width + GAP);
                let h = rows as f32 * (height + GAP);
                w.max(h)
            };
            side(*a).total_cmp(&side(*b))
        })
        .unwrap_or((1, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{FillRule, Paint, Segment};

    fn pixel(png: &[u8], x: u32, y: u32) -> [u8; 4] {
        let image = image::load_from_memory(png).unwrap().to_rgba8();
        image.get_pixel(x, y).0
    }

    #[test]
    fn draws_shapes_cut_to_the_canvas_at_double_scale() {
        let square = vec![
            Segment::Move([10.0, 10.0]),
            Segment::Line([60.0, 10.0]),
            Segment::Line([60.0, 60.0]),
            Segment::Line([10.0, 60.0]),
            Segment::Close,
        ];
        let shapes = vec![Shape::Fill {
            path: square,
            paint: Paint::Solid([1.0, 0.0, 0.0, 1.0]),
            rule: FillRule::NonZero,
        }];
        let frames = [Frame {
            label: String::new(),
            shapes: &shapes,
        }];
        let png = render_png(&frames, 100.0, 50.0, [1.0; 4]).unwrap();
        let image = image::load_from_memory(&png).unwrap();
        assert_eq!((image.width(), image.height()), (200, 100));
        assert_eq!(pixel(&png, 60, 60), [255, 0, 0, 255]);
        assert_eq!(pixel(&png, 150, 60), [255, 255, 255, 255]);
    }

    #[test]
    fn clips_to_a_rectangle_and_lays_frames_out_in_a_grid() {
        let full = vec![
            Segment::Move([0.0, 0.0]),
            Segment::Line([100.0, 0.0]),
            Segment::Line([100.0, 100.0]),
            Segment::Line([0.0, 100.0]),
            Segment::Close,
        ];
        let shapes = vec![
            Shape::Clip(Some([0.0, 0.0, 50.0, 100.0])),
            Shape::Fill {
                path: full,
                paint: Paint::Solid([0.0, 0.0, 1.0, 1.0]),
                rule: FillRule::NonZero,
            },
        ];
        let single = [Frame {
            label: String::new(),
            shapes: &shapes,
        }];
        let png = render_png(&single, 100.0, 100.0, [1.0; 4]).unwrap();
        assert_eq!(pixel(&png, 20, 100), [0, 0, 255, 255]);
        assert_eq!(pixel(&png, 180, 100), [255, 255, 255, 255]);

        let frames: Vec<Frame> = (0..4)
            .map(|ix| Frame {
                label: format!("t = {ix}"),
                shapes: &shapes,
            })
            .collect();
        let png = render_png(&frames, 100.0, 100.0, [1.0; 4]).unwrap();
        let image = image::load_from_memory(&png).unwrap();
        // 2 × 2 격자: (100·2 + 12) × ((100 + 18)·2 + 12), 2배.
        assert_eq!((image.width(), image.height()), (424, 496));
    }

    #[test]
    fn grid_keeps_the_longest_side_short() {
        assert_eq!(grid(1, 100.0, 100.0), (1, 1));
        assert_eq!(grid(4, 100.0, 100.0), (2, 2));
        // 넓은 캔버스는 위아래로 쌓는다.
        assert_eq!(grid(3, 500.0, 100.0), (1, 3));
    }
}
