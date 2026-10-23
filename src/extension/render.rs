//! 모양 목록을 픽셀로 그린다(vello_cpu, ADR-0042). 화면(에디터 블록·미리보기·플레이 창)과 PNG(어시스턴트의
//! `run_extension`, DOCX)가 같이 써서 어디서나 같은 그림이 나온다.
//!
//! - 그라디언트는 캔버스 좌표 그대로 정확히 칠한다(선형·방사형, 멈춤 여럿).
//! - `Shape::Clip`(사각형)과 `Shape::ClipPath`(경로)는 vello의 자르기 경로로 건다. 글자도 경로로 잘린다.
//! - 글자는 시스템 서체(PDF 내보내기와 같은 이름 목록)의 윤곽으로 그린다. 커닝·합자는 없다.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, BezPath, Cap, Join, Point as KPoint, Rect, Stroke};
use vello_cpu::peniko::{Fill, Gradient};
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use super::canvas::{
    Align, Color, FillRule, LINE_HEIGHT, LineCap, LineJoin, Paint, Segment, Shape,
};

/// 한 변의 픽셀 상한(vello_cpu의 크기는 u16이다).
pub const MAX_PIXELS: u32 = 8192;

/// 렌더 문맥. 내부 버퍼를 다시 쓰도록 캔버스(또는 작업)마다 하나를 들고 쓴다.
pub struct Renderer {
    ctx: RenderContext,
    resources: Resources,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Self {
        // 한 스레드로 돈다. 여러 스레드는 필터(블러)와 함께 쓰지 못한다.
        let settings = RenderSettings {
            num_threads: 0,
            ..RenderSettings::default()
        };
        Self {
            ctx: RenderContext::new_with(1, 1, settings),
            resources: Resources::new(),
        }
    }

    /// 모양을 `scale`배로 `width` × `height` px에 그린다. `background`가 없으면 바탕은 투명하다.
    /// 픽셀은 미리 곱한 RGBA다.
    pub fn render(
        &mut self,
        shapes: &[Shape],
        scale: f32,
        width: u32,
        height: u32,
        background: Option<Color>,
    ) -> Pixmap {
        self.render_cells(width, height, |ctx, fonts| {
            draw_cell(
                ctx,
                fonts,
                shapes,
                Affine::scale(f64::from(scale)),
                [width as f32 / scale, height as f32 / scale],
                background,
            );
        })
    }

    /// 빈 `width` × `height` px 위에 `paint`가 그린 것을 픽셀로 만든다.
    pub(super) fn render_cells(
        &mut self,
        width: u32,
        height: u32,
        paint: impl FnOnce(&mut RenderContext, &mut Fonts),
    ) -> Pixmap {
        let (w, h) = (clamp_side(width), clamp_side(height));
        self.ctx.reset_and_resize(w, h);
        let mut fonts = fonts().lock().unwrap_or_else(|err| err.into_inner());
        paint(&mut self.ctx, &mut fonts);
        drop(fonts);
        self.ctx.flush();
        let mut pixmap = Pixmap::new(w, h);
        self.ctx.render(&mut pixmap, &mut self.resources);
        pixmap
    }
}

fn clamp_side(v: u32) -> u16 {
    v.clamp(1, MAX_PIXELS) as u16
}

/// 캔버스 하나를 `base` 변환(캔버스 좌표 → 픽셀)으로 그린다. 캔버스(`size`, 캔버스 좌표) 밖은 잘린다.
pub(super) fn draw_cell(
    ctx: &mut RenderContext,
    fonts: &mut Fonts,
    shapes: &[Shape],
    base: Affine,
    size: [f32; 2],
    background: Option<Color>,
) {
    ctx.set_transform(base);
    let canvas = Rect::new(0.0, 0.0, f64::from(size[0]), f64::from(size[1]));
    ctx.push_clip_rect(&canvas);
    if let Some(background) = background {
        ctx.set_paint(color(background));
        ctx.fill_rect(&canvas);
    }
    draw(ctx, fonts, shapes, base);
    ctx.pop_clip();
}

fn draw(ctx: &mut RenderContext, fonts: &mut Fonts, shapes: &[Shape], base: Affine) {
    let mut clip_rect: Option<[f32; 4]> = None;
    let mut clip_paths: Vec<&(Vec<Segment>, FillRule)> = Vec::new();
    // 지금 걸어 둔 자르기 수(바뀌면 모두 풀고 다시 건다).
    let mut pushed = 0;
    for shape in shapes {
        match shape {
            Shape::Clip(_) | Shape::ClipPath(_) => {
                match shape {
                    Shape::Clip(rect) => clip_rect = *rect,
                    Shape::ClipPath(Some(path)) => clip_paths.push(path),
                    _ => clip_paths.clear(),
                }
                for _ in 0..pushed {
                    ctx.pop_clip();
                }
                pushed = 0;
                ctx.set_transform(base);
                if let Some([x, y, w, h]) = clip_rect {
                    let (x, y) = (f64::from(x), f64::from(y));
                    let (w, h) = (f64::from(w.max(0.0)), f64::from(h.max(0.0)));
                    ctx.push_clip_rect(&Rect::new(x, y, x + w, y + h));
                    pushed += 1;
                }
                for (path, rule) in &clip_paths {
                    ctx.set_fill_rule(fill_rule(*rule));
                    ctx.push_clip_path(&bez(path));
                    pushed += 1;
                }
            }
            Shape::Fill { path, paint, rule } => {
                if path.is_empty() {
                    continue;
                }
                ctx.set_fill_rule(fill_rule(*rule));
                set_paint(ctx, paint);
                ctx.fill_path(&bez(path));
            }
            Shape::Stroke { path, paint, style } => {
                if path.is_empty() || style.width <= 0.0 {
                    continue;
                }
                let stroke = Stroke::new(f64::from(style.width))
                    .with_miter_limit(f64::from(style.miter_limit))
                    .with_caps(match style.cap {
                        LineCap::Butt => Cap::Butt,
                        LineCap::Round => Cap::Round,
                        LineCap::Square => Cap::Square,
                    })
                    .with_join(match style.join {
                        LineJoin::Miter => Join::Miter,
                        LineJoin::Round => Join::Round,
                        LineJoin::Bevel => Join::Bevel,
                    });
                ctx.set_stroke(stroke);
                set_paint(ctx, paint);
                ctx.stroke_path(&bez(path));
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
                ctx.set_fill_rule(Fill::NonZero);
                ctx.set_paint(color(*text_color));
                ctx.fill_path(&path);
            }
        }
    }
    for _ in 0..pushed {
        ctx.pop_clip();
    }
}

fn bez(segments: &[Segment]) -> BezPath {
    let p = |[x, y]: [f32; 2]| KPoint::new(f64::from(x), f64::from(y));
    let mut path = BezPath::new();
    for segment in segments {
        match *segment {
            Segment::Move(a) => path.move_to(p(a)),
            Segment::Line(a) => path.line_to(p(a)),
            Segment::Cubic(a, b, c) => path.curve_to(p(a), p(b), p(c)),
            Segment::Close => path.close_path(),
        }
    }
    path
}

fn fill_rule(rule: FillRule) -> Fill {
    match rule {
        FillRule::NonZero => Fill::NonZero,
        FillRule::EvenOdd => Fill::EvenOdd,
    }
}

fn color([r, g, b, a]: Color) -> AlphaColor<Srgb> {
    let c = |v: f32| v.clamp(0.0, 1.0);
    AlphaColor::new([c(r), c(g), c(b), c(a)])
}

fn set_paint(ctx: &mut RenderContext, paint: &Paint) {
    let stops = |stops: &[(f32, Color)]| -> Vec<(f32, AlphaColor<Srgb>)> {
        stops.iter().map(|(at, c)| (*at, color(*c))).collect()
    };
    let last = |stops: &[(f32, Color)]| stops.last().map_or([0.0; 4], |(_, c)| *c);
    match paint {
        Paint::Solid(c) => ctx.set_paint(color(*c)),
        // 두 점이 같으면(반지름이 0이면) 바깥, 곧 끝 색만 남는다.
        Paint::Linear { from, to, stops: s } if from == to || s.len() < 2 => {
            ctx.set_paint(color(last(s)));
        }
        Paint::Radial {
            radius, stops: s, ..
        } if *radius <= 0.0 || s.len() < 2 => {
            ctx.set_paint(color(last(s)));
        }
        Paint::Linear { from, to, stops: s } => {
            let p = |[x, y]: [f32; 2]| KPoint::new(f64::from(x), f64::from(y));
            ctx.set_paint(Gradient::new_linear(p(*from), p(*to)).with_stops(stops(s).as_slice()));
        }
        Paint::Radial {
            center,
            radius,
            stops: s,
        } => {
            let center = KPoint::new(f64::from(center[0]), f64::from(center[1]));
            ctx.set_paint(Gradient::new_radial(center, *radius).with_stops(stops(s).as_slice()));
        }
    }
}

/// 미리 곱한 RGBA를 곱하지 않은 BGRA로 바꾼다(GPUI 이미지).
pub fn to_bgra(pixmap: Pixmap) -> Vec<u8> {
    let mut data = pixmap.take_rgba8(vello_cpu::peniko::ImageAlphaType::Alpha);
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    data
}

/// 모양 목록의 열쇠. 같으면 같은 그림이다.
pub fn shapes_key(shapes: &[Shape]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut f = |value: f32| value.to_bits().hash(&mut hasher);
    for shape in shapes {
        hash_shape(shape, &mut f);
    }
    shapes.len().hash(&mut hasher);
    hasher.finish()
}

fn hash_shape(shape: &Shape, f: &mut dyn FnMut(f32)) {
    let path = |path: &[Segment], f: &mut dyn FnMut(f32)| {
        for segment in path {
            match *segment {
                Segment::Move(p) => [1.0, p[0], p[1]].into_iter().for_each(&mut *f),
                Segment::Line(p) => [2.0, p[0], p[1]].into_iter().for_each(&mut *f),
                Segment::Cubic(a, b, p) => {
                    f(3.0);
                    [a, b, p].iter().flatten().for_each(|v| f(*v));
                }
                Segment::Close => f(4.0),
            }
        }
    };
    let stops = |stops: &[(f32, Color)], f: &mut dyn FnMut(f32)| {
        for (at, c) in stops {
            f(*at);
            c.iter().for_each(|v| f(*v));
        }
    };
    let paint = |paint: &Paint, f: &mut dyn FnMut(f32)| match paint {
        Paint::Solid(c) => {
            f(10.0);
            c.iter().for_each(|v| f(*v));
        }
        Paint::Linear { from, to, stops: s } => {
            f(11.0);
            [from, to].iter().copied().flatten().for_each(|v| f(*v));
            stops(s, f);
        }
        Paint::Radial {
            center,
            radius,
            stops: s,
        } => {
            f(12.0);
            center.iter().for_each(|v| f(*v));
            f(*radius);
            stops(s, f);
        }
    };
    match shape {
        Shape::Fill {
            path: p,
            paint: c,
            rule,
        } => {
            f(100.0 + *rule as u8 as f32);
            path(p, f);
            paint(c, f);
        }
        Shape::Stroke {
            path: p,
            paint: c,
            style,
        } => {
            [
                200.0,
                style.width,
                style.join as u8 as f32,
                style.cap as u8 as f32,
                style.miter_limit,
            ]
            .into_iter()
            .for_each(&mut *f);
            path(p, f);
            paint(c, f);
        }
        Shape::Text {
            text,
            origin,
            size,
            color,
            align,
            width,
        } => {
            f(300.0 + *align as u8 as f32);
            for c in text.chars() {
                f(c as u32 as f32);
            }
            [origin[0], origin[1], *size, width.unwrap_or(-1.0)]
                .into_iter()
                .for_each(&mut *f);
            color.iter().for_each(|v| f(*v));
        }
        Shape::Clip(rect) => {
            f(400.0);
            rect.iter().flatten().for_each(|v| f(*v));
        }
        Shape::ClipPath(next) => {
            f(500.0);
            if let Some((p, rule)) = next {
                f(*rule as u8 as f32);
                path(p, f);
            }
        }
    }
}

/// 시스템 서체. 처음 글자를 그릴 때 한 번 찾는다([`preload_fonts`]로 미리 찾아 둘 수 있다).
pub struct Fonts {
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

/// 서체를 작업 스레드에서 미리 찾는다. 첫 그림이 UI 스레드에서 서체를 기다리지 않게 한다.
pub fn preload_fonts() {
    std::thread::spawn(|| {
        if let Ok(mut fonts) = fonts().lock() {
            fonts.faces();
        }
    });
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

    /// 글자 한 줄의 윤곽(캔버스 좌표). 줄 높이 안에서 세로 가운데에 놓는다.
    fn line(
        &mut self,
        text: &str,
        [x, y]: [f32; 2],
        size: f32,
        align: Align,
        width: Option<f32>,
    ) -> Option<BezPath> {
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
        let mut path = BezPath::new();
        for (face, glyph, advance) in glyphs {
            if let Some(glyph) = glyph {
                let mut outline = Outline {
                    path: &mut path,
                    x: pen,
                    y: baseline,
                    scale: em(face),
                };
                face.outline_glyph(glyph, &mut outline);
            }
            pen += advance;
        }
        (!path.is_empty()).then_some(path)
    }
}

/// 글리프 윤곽(글꼴 단위, 위가 +)을 캔버스 좌표로 옮겨 담는다.
struct Outline<'a> {
    path: &'a mut BezPath,
    x: f32,
    y: f32,
    scale: f32,
}

impl Outline<'_> {
    fn at(&self, x: f32, y: f32) -> KPoint {
        KPoint::new(
            f64::from(self.x + x * self.scale),
            f64::from(self.y - y * self.scale),
        )
    }
}

impl ttf_parser::OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.path.move_to(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.path.line_to(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, p) = (self.at(x1, y1), self.at(x, y));
        self.path.quad_to(a, p);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b, p) = (self.at(x1, y1), self.at(x2, y2), self.at(x, y));
        self.path.curve_to(a, b, p);
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, y: f32, s: f32) -> Vec<Segment> {
        vec![
            Segment::Move([x, y]),
            Segment::Line([x + s, y]),
            Segment::Line([x + s, y + s]),
            Segment::Line([x, y + s]),
            Segment::Close,
        ]
    }

    fn at(pixmap: &Pixmap, x: u16, y: u16) -> [u8; 4] {
        let p = pixmap.sample(x, y);
        [p.r, p.g, p.b, p.a]
    }

    #[test]
    fn clips_by_a_path_and_keeps_the_rest_transparent() {
        let shapes = vec![
            Shape::ClipPath(Some((square(0.0, 0.0, 50.0), FillRule::NonZero))),
            Shape::Fill {
                path: square(0.0, 0.0, 100.0),
                paint: Paint::Solid([1.0, 0.0, 0.0, 1.0]),
                rule: FillRule::NonZero,
            },
            Shape::ClipPath(None),
            Shape::Fill {
                path: square(60.0, 60.0, 40.0),
                paint: Paint::Solid([0.0, 0.0, 1.0, 1.0]),
                rule: FillRule::NonZero,
            },
        ];
        let pixmap = Renderer::new().render(&shapes, 2.0, 200, 200, None);
        assert_eq!(at(&pixmap, 50, 50), [255, 0, 0, 255]);
        assert_eq!(at(&pixmap, 150, 50), [0, 0, 0, 0]);
        assert_eq!(at(&pixmap, 150, 150), [0, 0, 255, 255]);
    }

    #[test]
    fn three_stop_gradient_hits_the_middle_color() {
        let shapes = vec![Shape::Fill {
            path: square(0.0, 0.0, 100.0),
            paint: Paint::Linear {
                from: [0.0, 0.0],
                to: [100.0, 0.0],
                stops: vec![
                    (0.0, [1.0, 0.0, 0.0, 1.0]),
                    (0.5, [0.0, 1.0, 0.0, 1.0]),
                    (1.0, [0.0, 0.0, 1.0, 1.0]),
                ],
            },
            rule: FillRule::NonZero,
        }];
        let pixmap = Renderer::new().render(&shapes, 1.0, 100, 100, None);
        let [r, g, b, _] = at(&pixmap, 50, 50);
        assert!(g > 240 && r < 15 && b < 15, "{r} {g} {b}");
    }

    #[test]
    fn same_shapes_have_the_same_key() {
        let a = vec![Shape::Clip(Some([0.0, 0.0, 1.0, 1.0]))];
        let b = vec![Shape::Clip(Some([0.0, 0.0, 1.0, 2.0]))];
        assert_eq!(shapes_key(&a), shapes_key(&a.clone()));
        assert_ne!(shapes_key(&a), shapes_key(&b));
    }
}

