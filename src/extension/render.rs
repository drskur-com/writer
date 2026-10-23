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
use vello_cpu::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::peniko::{
    BlendMode, Compose, Extend, Fill, Gradient, ImageAlphaType, ImageQuality, ImageSampler, Mix,
};
use vello_cpu::{ImageSource, PixelMetadata};
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use super::canvas::{Blend, Effects, 
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
    let mut effects = Effects::default();
    ctx.set_blend_mode(blend_mode(Blend::Normal));
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
            Shape::Image {
                image,
                src: [x, y, w, h],
                transform,
                opacity,
                smooth,
            } => {
                if *w <= 0.0 || *h <= 0.0 || *opacity <= 0.0 {
                    continue;
                }
                let pixels = image.prepared.get_or_init(|| {
                    let rgba = &image.rgba;
                    Arc::new(Pixmap::from_parts(
                        rgba.as_raw().clone(),
                        rgba.width() as u16,
                        rgba.height() as u16,
                        PixelMetadata {
                            may_have_transparency: true,
                            alpha_type: ImageAlphaType::Alpha,
                        },
                    ))
                });
                // 화면용 픽셀은 줄였을 수 있다: 원래 크기 좌표로 맞춘다.
                let shrink = f64::from(image.width) / f64::from(image.rgba.width());
                let brush = vello_cpu::Image {
                    image: ImageSource::Pixmap(pixels.clone()),
                    sampler: ImageSampler {
                        x_extend: Extend::Pad,
                        y_extend: Extend::Pad,
                        quality: if *smooth {
                            ImageQuality::Medium
                        } else {
                            ImageQuality::Low
                        },
                        alpha: opacity.clamp(0.0, 1.0),
                    },
                };
                let [a, b, c, d, e, f] = transform.map(f64::from);
                let rect = Rect::new(
                    f64::from(*x),
                    f64::from(*y),
                    f64::from(x + w),
                    f64::from(y + h),
                );
                ctx.set_transform(base * Affine::new([a, b, c, d, e, f]));
                with_effects(ctx, &effects, |ctx| {
                    ctx.set_paint(brush.clone());
                    ctx.set_paint_transform(Affine::scale(shrink));
                    ctx.fill_rect(&rect);
                    ctx.reset_paint_transform();
                });
                ctx.set_transform(base);
            }
            Shape::Scene3D(mesh) => {
                let [s, ..] = base.as_coeffs();
                let Some(pixels) = super::raster3d::rasterize(mesh, s as f32, MAX_PIXELS) else {
                    continue;
                };
                let (w, h, k) = (pixels.width, pixels.height, f64::from(pixels.scale));
                let pixmap = Arc::new(Pixmap::from_parts(
                    pixels.rgba,
                    w as u16,
                    h as u16,
                    PixelMetadata {
                        may_have_transparency: true,
                        alpha_type: ImageAlphaType::Alpha,
                    },
                ));
                let brush = vello_cpu::Image {
                    image: ImageSource::Pixmap(pixmap),
                    sampler: ImageSampler::default(),
                };
                let [bx, by, ..] = mesh.bounds.map(f64::from);
                let mut clip = BezPath::new();
                for (ix, [x, y]) in mesh.clip.iter().enumerate() {
                    let p = KPoint::new(f64::from(*x), f64::from(*y));
                    if ix == 0 { clip.move_to(p) } else { clip.line_to(p) }
                }
                clip.close_path();
                ctx.set_transform(base);
                ctx.set_fill_rule(Fill::NonZero);
                ctx.push_clip_path(&clip);
                ctx.set_transform(base * Affine::translate((bx, by)) * Affine::scale(1.0 / k));
                with_effects(ctx, &effects, |ctx| {
                    ctx.set_paint(brush.clone());
                    ctx.fill_rect(&Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
                });
                ctx.set_transform(base);
                ctx.pop_clip();
            }
            Shape::Effects(next) => {
                effects = *next;
                ctx.set_blend_mode(blend_mode(effects.blend));
            }
            Shape::Fill { path, paint, rule } => {
                if path.is_empty() {
                    continue;
                }
                let path = bez(path);
                with_effects(ctx, &effects, |ctx| {
                    ctx.set_fill_rule(fill_rule(*rule));
                    set_paint(ctx, paint);
                    ctx.fill_path(&path);
                });
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
                let path = bez(path);
                with_effects(ctx, &effects, |ctx| {
                    ctx.set_stroke(stroke.clone());
                    set_paint(ctx, paint);
                    ctx.stroke_path(&path);
                });
            }
            Shape::Text {
                text,
                origin,
                size,
                color: text_color,
                align,
                width,
                angle,
            } => {
                let Some(mut path) = fonts.line(text, *origin, *size, *align, *width) else {
                    continue;
                };
                if *angle != 0.0 {
                    let center = KPoint::new(f64::from(origin[0]), f64::from(origin[1]));
                    path.apply_affine(Affine::rotate_about(f64::from(*angle), center));
                }
                with_effects(ctx, &effects, |ctx| {
                    ctx.set_fill_rule(Fill::NonZero);
                    ctx.set_paint(color(*text_color));
                    ctx.fill_path(&path);
                });
            }
        }
    }
    for _ in 0..pushed {
        ctx.pop_clip();
    }
    ctx.set_blend_mode(blend_mode(Blend::Normal));
    ctx.reset_filter_effect();
}

/// `draw`로 모양을 그린다. 그림자가 있으면 먼저 그림자만 그리고, 흐림이 있으면 모양을 흐려 그린다.
fn with_effects(ctx: &mut RenderContext, effects: &Effects, draw: impl Fn(&mut RenderContext)) {
    if let Some(shadow) = effects.shadow {
        ctx.set_filter_effect(Filter::from_primitive(FilterPrimitive::DropShadowOnly {
            dx: shadow.offset[0],
            dy: shadow.offset[1],
            std_deviation: shadow.blur,
            color: color(shadow.color),
            edge_mode: EdgeMode::None,
        }));
        draw(ctx);
    }
    if effects.blur > 0.0 {
        ctx.set_filter_effect(Filter::from_primitive(FilterPrimitive::GaussianBlur {
            std_deviation: effects.blur,
            edge_mode: EdgeMode::None,
        }));
    } else {
        ctx.reset_filter_effect();
    }
    draw(ctx);
    ctx.reset_filter_effect();
}

fn blend_mode(blend: Blend) -> BlendMode {
    let mix = match blend {
        Blend::Normal | Blend::Add => Mix::Normal,
        Blend::Multiply => Mix::Multiply,
        Blend::Screen => Mix::Screen,
        Blend::Overlay => Mix::Overlay,
        Blend::Darken => Mix::Darken,
        Blend::Lighten => Mix::Lighten,
        Blend::ColorDodge => Mix::ColorDodge,
        Blend::ColorBurn => Mix::ColorBurn,
        Blend::HardLight => Mix::HardLight,
        Blend::SoftLight => Mix::SoftLight,
        Blend::Difference => Mix::Difference,
        Blend::Exclusion => Mix::Exclusion,
        Blend::Hue => Mix::Hue,
        Blend::Saturation => Mix::Saturation,
        Blend::Color => Mix::Color,
        Blend::Luminosity => Mix::Luminosity,
    };
    let compose = if blend == Blend::Add {
        Compose::Plus
    } else {
        Compose::SrcOver
    };
    BlendMode::new(mix, compose)
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
            angle,
        } => {
            f(300.0 + *align as u8 as f32);
            for c in text.chars() {
                f(c as u32 as f32);
            }
            [origin[0], origin[1], *size, width.unwrap_or(-1.0), *angle]
                .into_iter()
                .for_each(&mut *f);
            color.iter().for_each(|v| f(*v));
        }
        Shape::Clip(rect) => {
            f(400.0);
            rect.iter().flatten().for_each(|v| f(*v));
        }
        Shape::Image {
            image,
            src,
            transform,
            opacity,
            smooth,
        } => {
            f(700.0 + f32::from(u8::from(*smooth)));
            f(f32::from_bits(image.id as u32));
            f(f32::from_bits((image.id >> 32) as u32));
            src.iter().chain(transform).for_each(|v| f(*v));
            f(*opacity);
        }
        Shape::Scene3D(mesh) => {
            f(800.0);
            f(f32::from_bits(mesh.key as u32));
            f(f32::from_bits((mesh.key >> 32) as u32));
        }
        Shape::Effects(effects) => {
            f(600.0 + effects.blend as u8 as f32);
            f(effects.blur);
            if let Some(shadow) = effects.shadow {
                [shadow.blur, shadow.offset[0], shadow.offset[1]].into_iter().for_each(&mut *f);
                shadow.color.iter().for_each(|v| f(*v));
            }
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

    fn fill(path: Vec<Segment>, c: Color) -> Shape {
        Shape::Fill {
            path,
            paint: Paint::Solid(c),
            rule: FillRule::NonZero,
        }
    }

    #[test]
    fn shadow_and_blur_follow_canvas_units() {
        use crate::extension::canvas::Shadow;
        let shapes = vec![
            Shape::Effects(Effects {
                shadow: Some(Shadow {
                    color: [0.0, 0.0, 0.0, 1.0],
                    blur: 0.0,
                    offset: [30.0, 0.0],
                }),
                ..Effects::default()
            }),
            fill(square(10.0, 10.0, 20.0), [1.0, 0.0, 0.0, 1.0]),
        ];
        let pixmap = Renderer::new().render(&shapes, 2.0, 200, 100, None);
        // 모양은 그대로, 그림자는 캔버스 좌표로 30만큼(2배라 60px) 오른쪽에 있다.
        assert_eq!(at(&pixmap, 40, 40), [255, 0, 0, 255]);
        assert_eq!(at(&pixmap, 70, 40)[3], 0);
        assert_eq!(at(&pixmap, 110, 40), [0, 0, 0, 255]);

        let blurred = vec![
            Shape::Effects(Effects {
                blur: 4.0,
                ..Effects::default()
            }),
            fill(square(10.0, 10.0, 20.0), [1.0, 0.0, 0.0, 1.0]),
        ];
        let pixmap = Renderer::new().render(&blurred, 2.0, 200, 100, None);
        // 가장자리는 반쯤 비치고, 모양 밖으로도 번진다.
        let edge = at(&pixmap, 20, 40)[3];
        assert!(edge > 60 && edge < 200, "{edge}");
        assert!(at(&pixmap, 14, 40)[3] > 0);
    }

    #[test]
    fn multiply_darkens_what_is_below() {
        let shapes = vec![
            fill(square(0.0, 0.0, 10.0), [0.0, 0.0, 1.0, 1.0]),
            Shape::Effects(Effects {
                blend: Blend::Multiply,
                ..Effects::default()
            }),
            fill(square(0.0, 0.0, 10.0), [1.0, 0.0, 0.0, 1.0]),
        ];
        let pixmap = Renderer::new().render(&shapes, 1.0, 10, 10, None);
        assert_eq!(at(&pixmap, 5, 5), [0, 0, 0, 255]);
    }

    #[test]
    fn rotated_text_turns_around_its_origin() {
        let text = |angle: f32| {
            vec![Shape::Text {
                text: "가나다라마바사".into(),
                origin: [50.0, 50.0],
                size: 10.0,
                color: [0.0, 0.0, 0.0, 1.0],
                align: Align::Left,
                width: None,
                angle,
            }]
        };
        let ink = |pixmap: &Pixmap, x0: u16, y0: u16, x1: u16, y1: u16| {
            (x0..x1)
                .flat_map(|x| (y0..y1).map(move |y| (x, y)))
                .filter(|&(x, y)| pixmap.sample(x, y).a > 0)
                .count()
        };
        let mut renderer = Renderer::new();
        let flat = renderer.render(&text(0.0), 1.0, 120, 120, None);
        let turned = renderer.render(&text(std::f32::consts::FRAC_PI_2), 1.0, 120, 120, None);
        // 그대로면 오른쪽으로, 시계 방향 90도면 아래로 쓴다.
        assert!(ink(&flat, 60, 45, 110, 65) > 50);
        assert_eq!(ink(&flat, 30, 60, 50, 110), 0);
        assert!(ink(&turned, 30, 60, 50, 110) > 50);
        assert_eq!(ink(&turned, 60, 45, 110, 65), 0);
    }

    #[test]
    fn same_shapes_have_the_same_key() {
        let a = vec![Shape::Clip(Some([0.0, 0.0, 1.0, 1.0]))];
        let b = vec![Shape::Clip(Some([0.0, 0.0, 1.0, 2.0]))];
        assert_eq!(shapes_key(&a), shapes_key(&a.clone()));
        assert_ne!(shapes_key(&a), shapes_key(&b));
    }
}
