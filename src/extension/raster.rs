//! 모양 목록을 PNG로 그린다(tiny-skia). 어시스턴트의 `run_extension`이 그린 결과를 모델에 보여 준다.
//!
//! - 캔버스 밖은 잘린다(에디터와 같다). 배경은 테마 배경색이다.
//! - 글자는 시스템 서체(PDF 내보내기와 같은 이름 목록)의 윤곽으로 그린다. 커닝·합자는 없어 화면과 폭이 조금 다르다.
//! - 프레임이 여럿이면 격자로 놓고 프레임마다 `t = …` 이름표를 붙인다.

use std::sync::{Arc, Mutex, OnceLock};

use tiny_skia::{
    FillRule as SkFillRule, GradientStop, LinearGradient, Mask, Paint as SkPaint, PathBuilder,
    Pixmap, PixmapPaint, RadialGradient, Shader, SpreadMode, Stroke, Transform,
};

use super::canvas::{
    Align, Color, FillRule, LINE_HEIGHT, LineCap, LineJoin, Paint, Segment, Shape,
};

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
    let mut image = Pixmap::new(px(total_w), px(total_h)).ok_or("The canvas is too large.")?;
    // 프레임 사이는 회색으로 두어 경계가 보이게 한다.
    image.fill(if labeled {
        tiny_skia::Color::from_rgba8(200, 200, 200, 255)
    } else {
        color(background)
    });
    let mut fonts = fonts().lock().map_err(|_| "Font lookup failed.")?;
    for (ix, frame) in frames.iter().enumerate() {
        let (col, row) = (ix % cols, ix / cols);
        let x = col as f32 * (width + GAP);
        let y = row as f32 * (cell_h + GAP);
        let mut cell = Pixmap::new(px(width), px(cell_h)).ok_or("The canvas is too large.")?;
        cell.fill(color(background));
        let top = if labeled {
            let mut label = Pixmap::new(px(width), px(LABEL)).ok_or("The canvas is too large.")?;
            label.fill(tiny_skia::Color::from_rgba8(235, 235, 235, 255));
            let text = Shape::Text {
                text: frame.label.clone(),
                origin: [4.0, 2.0],
                size: 12.0,
                color: [0.25, 0.25, 0.25, 1.0],
                align: Align::Left,
                width: None,
            };
            draw(&mut label, std::slice::from_ref(&text), scale, &mut fonts);
            cell.draw_pixmap(
                0,
                0,
                label.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            LABEL
        } else {
            0.0
        };
        let mut canvas = Pixmap::new(px(width), px(height)).ok_or("The canvas is too large.")?;
        canvas.fill(color(background));
        draw(&mut canvas, frame.shapes, scale, &mut fonts);
        cell.draw_pixmap(
            0,
            (top * scale).round() as i32,
            canvas.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
        image.draw_pixmap(
            (x * scale).round() as i32,
            (y * scale).round() as i32,
            cell.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    image.encode_png().map_err(|err| err.to_string())
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

fn draw(pixmap: &mut Pixmap, shapes: &[Shape], scale: f32, fonts: &mut Fonts) {
    let transform = Transform::from_scale(scale, scale);
    let mut clip_rect: Option<[f32; 4]> = None;
    let mut clip_paths: Vec<(Vec<Segment>, FillRule)> = Vec::new();
    let mut mask: Option<Mask> = None;
    for shape in shapes {
        match shape {
            Shape::Clip(rect) => {
                clip_rect = *rect;
                mask = clip_mask(pixmap, clip_rect, &clip_paths, transform);
            }
            Shape::ClipPath(next) => {
                match next {
                    Some(path) => clip_paths.push(path.clone()),
                    None => clip_paths.clear(),
                }
                mask = clip_mask(pixmap, clip_rect, &clip_paths, transform);
            }
            Shape::Fill { path, paint, rule } => {
                if let (Some(path), Some(paint)) = (sk_path(path), sk_paint(paint)) {
                    pixmap.fill_path(&path, &paint, fill_rule(*rule), transform, mask.as_ref());
                }
            }
            Shape::Stroke { path, paint, style } => {
                if let (Some(path), Some(paint)) = (sk_path(path), sk_paint(paint)) {
                    let stroke = Stroke {
                        width: style.width,
                        miter_limit: style.miter_limit,
                        line_cap: match style.cap {
                            LineCap::Butt => tiny_skia::LineCap::Butt,
                            LineCap::Round => tiny_skia::LineCap::Round,
                            LineCap::Square => tiny_skia::LineCap::Square,
                        },
                        line_join: match style.join {
                            LineJoin::Miter => tiny_skia::LineJoin::Miter,
                            LineJoin::Round => tiny_skia::LineJoin::Round,
                            LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
                        },
                        dash: None,
                    };
                    pixmap.stroke_path(&path, &paint, &stroke, transform, mask.as_ref());
                }
            }
            Shape::Text {
                text,
                origin,
                size,
                color: text_color,
                align,
                width,
            } => {
                let Some(path) = fonts.line(text, *origin, *size, *align, *width) else {
                    continue;
                };
                let paint = sk_paint(&Paint::Solid(*text_color));
                if let Some(paint) = paint {
                    pixmap.fill_path(&path, &paint, SkFillRule::Winding, transform, mask.as_ref());
                }
            }
        }
    }
}

fn clip_mask(
    pixmap: &Pixmap,
    rect: Option<[f32; 4]>,
    paths: &[(Vec<Segment>, FillRule)],
    transform: Transform,
) -> Option<Mask> {
    if rect.is_none() && paths.is_empty() {
        return None;
    }
    let mut mask = Mask::new(pixmap.width(), pixmap.height())?;
    let [x, y, w, h] = rect.unwrap_or([
        0.0,
        0.0,
        pixmap.width() as f32 / transform.sx,
        pixmap.height() as f32 / transform.sy,
    ]);
    // 빈 사각형이면 아무것도 보이지 않는다(마스크가 비어 있다).
    if let Some(rect) = tiny_skia::Rect::from_xywh(x, y, w.max(0.0), h.max(0.0)) {
        mask.fill_path(
            &PathBuilder::from_rect(rect),
            SkFillRule::Winding,
            false,
            transform,
        );
    }
    for (path, rule) in paths {
        match sk_path(path) {
            Some(path) => mask.intersect_path(&path, fill_rule(*rule), true, transform),
            // 자를 경로가 비었으면 아무것도 보이지 않는다.
            None => mask = Mask::new(pixmap.width(), pixmap.height())?,
        }
    }
    Some(mask)
}

fn sk_path(segments: &[Segment]) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    for segment in segments {
        match *segment {
            Segment::Move([x, y]) => builder.move_to(x, y),
            Segment::Line([x, y]) => builder.line_to(x, y),
            Segment::Cubic([ax, ay], [bx, by], [x, y]) => builder.cubic_to(ax, ay, bx, by, x, y),
            Segment::Close => builder.close(),
        }
    }
    builder.finish()
}

fn fill_rule(rule: FillRule) -> SkFillRule {
    match rule {
        FillRule::NonZero => SkFillRule::Winding,
        FillRule::EvenOdd => SkFillRule::EvenOdd,
    }
}

fn color([r, g, b, a]: Color) -> tiny_skia::Color {
    let c = |v: f32| v.clamp(0.0, 1.0);
    tiny_skia::Color::from_rgba(c(r), c(g), c(b), c(a)).unwrap_or(tiny_skia::Color::BLACK)
}

fn pt([x, y]: [f32; 2]) -> tiny_skia::Point {
    tiny_skia::Point::from_xy(x, y)
}

fn sk_paint(paint: &Paint) -> Option<SkPaint<'static>> {
    let stops = |stops: &[(f32, Color)]| -> Vec<GradientStop> {
        stops
            .iter()
            .map(|(offset, c)| GradientStop::new(*offset, color(*c)))
            .collect()
    };
    let last = |stops: &[(f32, Color)]| stops.last().map_or([0.0; 4], |(_, c)| *c);
    let shader = match paint {
        Paint::Solid(c) => Shader::SolidColor(color(*c)),
        Paint::Linear { from, to, stops: s } => LinearGradient::new(
            pt(*from),
            pt(*to),
            stops(s),
            SpreadMode::Pad,
            Transform::identity(),
        )
        // 두 점이 같으면 바깥(끝 색)만 남는다.
        .unwrap_or(Shader::SolidColor(color(last(s)))),
        Paint::Radial {
            center,
            radius,
            stops: s,
        } => RadialGradient::new(
            pt(*center),
            pt(*center),
            *radius,
            stops(s),
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap_or(Shader::SolidColor(color(last(s)))),
    };
    Some(SkPaint {
        shader,
        anti_alias: true,
        ..SkPaint::default()
    })
}

/// 시스템 서체. 처음 글자를 그릴 때 한 번 찾는다.
struct Fonts {
    db: fontdb::Database,
    /// 기본 서체 다음 대체 서체들(글자가 있는 첫 서체로 그린다). `None`이면 아직 찾지 않았다.
    faces: Option<Vec<(Arc<Vec<u8>>, u32)>>,
}

fn fonts() -> &'static Mutex<Fonts> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Mutex::new(Fonts { db, faces: None })
    })
}

impl Fonts {
    fn faces(&mut self) -> &[(Arc<Vec<u8>>, u32)] {
        if self.faces.is_none() {
            let db = &self.db;
            let faces = crate::export::pdf::DEFAULT_FAMILIES
                .iter()
                .chain(crate::export::pdf::FALLBACK_FAMILIES.iter())
                .map(|name| fontdb::Family::Name(name))
                .chain([fontdb::Family::SansSerif])
                .filter_map(|family| {
                    let families = [family];
                    let id = db.query(&fontdb::Query {
                        families: &families,
                        ..fontdb::Query::default()
                    })?;
                    db.with_face_data(id, |data, index| (Arc::new(data.to_vec()), index))
                })
                .collect();
            self.faces = Some(faces);
        }
        self.faces.as_deref().unwrap_or_default()
    }

    /// 글자 한 줄의 윤곽(캔버스 좌표). 줄 높이 안에서 세로 가운데에 놓는다(에디터와 같다).
    fn line(
        &mut self,
        text: &str,
        [x, y]: [f32; 2],
        size: f32,
        align: Align,
        width: Option<f32>,
    ) -> Option<tiny_skia::Path> {
        if text.trim().is_empty() || size <= 0.0 {
            return None;
        }
        let faces: Vec<ttf_parser::Face> = self
            .faces()
            .iter()
            .filter_map(|(data, index)| ttf_parser::Face::parse(data, *index).ok())
            .collect();
        let primary = faces.first()?;
        let em = |face: &ttf_parser::Face| size / f32::from(face.units_per_em());
        let ascent = f32::from(primary.ascender()) * em(primary);
        let descent = -f32::from(primary.descender()) * em(primary);
        let baseline = y + (size * LINE_HEIGHT - ascent - descent) / 2.0 + ascent;
        // 글자마다 (서체, 글리프, 앞으로 갈 폭).
        let glyphs: Vec<(&ttf_parser::Face, Option<ttf_parser::GlyphId>, f32)> = text
            .chars()
            .map(|c| {
                let found = faces
                    .iter()
                    .find_map(|face| face.glyph_index(c).map(|glyph| (face, glyph)));
                match found {
                    Some((face, glyph)) => {
                        let advance =
                            f32::from(face.glyph_hor_advance(glyph).unwrap_or(0)) * em(face);
                        (face, Some(glyph), advance)
                    }
                    None => (primary, None, size * 0.5),
                }
            })
            .collect();
        let line_width: f32 = glyphs.iter().map(|(_, _, advance)| advance).sum();
        let mut pen = x + match (align, width) {
            (Align::Center, Some(width)) => (width - line_width) / 2.0,
            (Align::Right, Some(width)) => width - line_width,
            _ => 0.0,
        };
        let mut builder = PathBuilder::new();
        for (face, glyph, advance) in glyphs {
            if let Some(glyph) = glyph {
                let mut outline = Outline {
                    builder: &mut builder,
                    x: pen,
                    y: baseline,
                    scale: em(face),
                };
                face.outline_glyph(glyph, &mut outline);
            }
            pen += advance;
        }
        builder.finish()
    }
}

/// 글리프 윤곽(글꼴 단위, 위가 +)을 캔버스 좌표로 옮겨 담는다.
struct Outline<'a> {
    builder: &'a mut PathBuilder,
    x: f32,
    y: f32,
    scale: f32,
}

impl Outline<'_> {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.scale, self.y - y * self.scale)
    }
}

impl ttf_parser::OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.at(x1, y1);
        let (x, y) = self.at(x, y);
        self.builder.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.at(x1, y1);
        let (x2, y2) = self.at(x2, y2);
        let (x, y) = self.at(x, y);
        self.builder.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
