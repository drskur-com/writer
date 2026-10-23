//! 확장의 모양 목록을 GPUI로 그린다(ADR-0026).
//!
//! - 모양마다 lyon으로 삼각형을 만들고, (모양, 배율)마다 캐시한다. 같은 그림을 다시 그리거나(스크롤, 입력)
//!   주기가 있는 애니메이션의 같은 프레임, 움직이지 않는 부분은 다시 나누지 않는다.
//! - GPUI의 경로 그라디언트는 두 색 선형뿐이고 경로의 테두리 상자를 기준으로 계산한다. 그래서 캔버스 좌표의
//!   두 점 사이 그라디언트를 그 상자 기준 각도·자리로 바꿔 넘긴다([`linear_background`]).
//!   - 세 색 이상 선형: 삼각형을 멈춤 사이의 띠로 잘라 띠마다 두 색으로 칠한다(정확하다).
//!   - 방사형: 삼각형을 고리 × 부채꼴 칸으로 잘라 칸마다 반지름 방향의 두 색 선형으로 칠한다(근사, 칸 경계의
//!     오차는 0.25px 안쪽).
//! - `Shape::Clip`(사각형)은 뒤 모양들의 content mask가 된다. `Shape::ClipPath`(경로)는 그 경로의 삼각형과
//!   모양의 삼각형이 겹친 곳(볼록 ∩ 볼록)만 남긴다. 여럿이면 모두 겹친 곳이다. 글자는 GPU 글자라 경로로는 못
//!   자르고 그 겹친 곳의 테두리 상자로 자른다.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui_kit::gpui::*;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};

use crate::extension::{self, FillRule, LineCap, LineJoin, Paint, Shape};

/// 곡선을 펴는 허용 오차(px).
const TOLERANCE: f32 = 0.1;

/// 방사형 칸 경계(현)와 실제 원의 차이 상한(px).
const RADIAL_CHORD_ERROR: f32 = 0.25;

/// 캐시가 이보다 커지면 이번 프레임에 쓰지 않은 것을 버린다.
const CACHE_LIMIT: usize = 8_192;

type P = [f32; 2];
type Tri = [P; 3];

/// 그릴 조각: 캔버스 왼쪽 위가 (0, 0)인 경로와 칠.
struct Piece {
    path: Path<Pixels>,
    background: Background,
}

/// 모양별 삼각형 캐시. 캔버스(에디터 블록, 대화상자 미리보기)마다 하나.
#[derive(Default)]
pub(super) struct TessCache {
    entries: HashMap<u64, (u64, Rc<Vec<Piece>>)>,
    /// 경로 자르기 영역(삼각형). 열쇠는 자르는 경로들과 배율.
    regions: HashMap<u64, (u64, Rc<Vec<Tri>>)>,
    frame: u64,
}

impl TessCache {
    fn get(&mut self, key: u64, build: impl FnOnce() -> Vec<Piece>) -> Rc<Vec<Piece>> {
        let frame = self.frame;
        if let Some((used, pieces)) = self.entries.get_mut(&key) {
            *used = frame;
            return pieces.clone();
        }
        let pieces = Rc::new(build());
        self.entries.insert(key, (frame, pieces.clone()));
        pieces
    }

    fn region(&mut self, key: u64, build: impl FnOnce() -> Vec<Tri>) -> Rc<Vec<Tri>> {
        let frame = self.frame;
        if let Some((used, region)) = self.regions.get_mut(&key) {
            *used = frame;
            return region.clone();
        }
        let region = Rc::new(build());
        self.regions.insert(key, (frame, region.clone()));
        region
    }

    fn begin_frame(&mut self) {
        self.frame += 1;
    }

    fn end_frame(&mut self) {
        let frame = self.frame;
        if self.entries.len() > CACHE_LIMIT {
            self.entries.retain(|_, (used, _)| *used == frame);
        }
        if self.regions.len() > CACHE_LIMIT / 8 {
            self.regions.retain(|_, (used, _)| *used == frame);
        }
    }
}

/// 모양 목록을 `origin`에 `scale` 배로 그린다.
pub(super) fn paint(
    shapes: &[Shape],
    origin: Point<Pixels>,
    scale: f32,
    cache: &RefCell<TessCache>,
    window: &mut Window,
    cx: &mut App,
) {
    let mut cache = cache.borrow_mut();
    cache.begin_frame();
    let at = |[x, y]: P| point(origin.x + px(x * scale), origin.y + px(y * scale));
    let font = window.text_style().font();
    let mut clip: Option<Bounds<Pixels>> = None;
    // 경로 자르기: 지금 걸린 경로들과 그 겹친 영역(열쇠, 삼각형).
    let mut clip_paths: Vec<&(Vec<extension::Segment>, FillRule)> = Vec::new();
    let mut region: Option<(u64, Rc<Vec<Tri>>)> = None;
    for shape in shapes {
        if let Shape::ClipPath(next) = shape {
            match next {
                Some(path) => clip_paths.push(path),
                None => clip_paths.clear(),
            }
            region = (!clip_paths.is_empty()).then(|| {
                let key = clip_key(&clip_paths, scale);
                let tris = cache.region(key, || clip_region(&clip_paths, scale));
                (key, tris)
            });
            continue;
        }
        // 글자는 경로로 못 자르므로 영역의 테두리 상자로 자른다.
        let region_mask = region.as_ref().map(|(_, tris)| {
            if tris.is_empty() {
                return Bounds::default();
            }
            let [x0, y0, x1, y1] = bbox(&tris.iter().flatten().copied().collect::<Vec<_>>());
            Bounds {
                origin: point(origin.x + px(x0), origin.y + px(y0)),
                size: size(px(x1 - x0), px(y1 - y0)),
            }
        });
        let region_ref = region.as_ref();
        let mut paint_one = |window: &mut Window, cache: &mut TessCache| match shape {
            Shape::Fill { .. } | Shape::Stroke { .. } => {
                let key = match region_ref {
                    Some((region_key, _)) => combine(shape_key(shape, scale), *region_key),
                    None => shape_key(shape, scale),
                };
                let pieces = cache.get(key, || {
                    build(shape, scale, region_ref.map(|(_, tris)| tris.as_slice()))
                });
                for piece in pieces.iter() {
                    window.paint_path(translated(&piece.path, origin), piece.background);
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
                    return;
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
            Shape::Clip(_) | Shape::ClipPath(_) => {}
        };
        if let Shape::Clip(rect) = shape {
            clip = rect.map(|[x, y, w, h]| Bounds {
                origin: at([x, y]),
                size: size(px(w * scale), px(h * scale)),
            });
            continue;
        }
        let mask = match (
            clip,
            matches!(shape, Shape::Text { .. })
                .then_some(region_mask)
                .flatten(),
        ) {
            (Some(a), Some(b)) => Some(a.intersect(&b)),
            (a, b) => a.or(b),
        };
        match mask {
            Some(bounds) => window.with_content_mask(Some(ContentMask { bounds }), |window| {
                paint_one(window, &mut cache)
            }),
            None => paint_one(window, &mut cache),
        }
    }
    cache.end_frame();
}

pub(super) fn hsla([r, g, b, a]: extension::Color) -> Hsla {
    Rgba { r, g, b, a }.into()
}

fn translated(path: &Path<Pixels>, by: Point<Pixels>) -> Path<Pixels> {
    let mut path = path.clone();
    for vertex in &mut path.vertices {
        vertex.xy_position += by;
    }
    path.bounds.origin += by;
    path
}

/// 모양과 배율의 캐시 열쇠.
fn shape_key(shape: &Shape, scale: f32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut f = |value: f32| value.to_bits().hash(&mut hasher);
    f(scale);
    let hash_path = |path: &[extension::Segment], f: &mut dyn FnMut(f32)| {
        for segment in path {
            match *segment {
                extension::Segment::Move(p) => {
                    f(1.0);
                    f(p[0]);
                    f(p[1]);
                }
                extension::Segment::Line(p) => {
                    f(2.0);
                    f(p[0]);
                    f(p[1]);
                }
                extension::Segment::Cubic(a, b, p) => {
                    f(3.0);
                    [a, b, p].iter().flatten().for_each(|v| f(*v));
                }
                extension::Segment::Close => f(4.0),
            }
        }
    };
    let hash_paint = |paint: &Paint, f: &mut dyn FnMut(f32)| match paint {
        Paint::Solid(c) => {
            f(10.0);
            c.iter().for_each(|v| f(*v));
        }
        Paint::Linear { from, to, stops } => {
            f(11.0);
            [from, to].iter().copied().flatten().for_each(|v| f(*v));
            for (pos, c) in stops {
                f(*pos);
                c.iter().for_each(|v| f(*v));
            }
        }
        Paint::Radial {
            center,
            radius,
            stops,
        } => {
            f(12.0);
            center.iter().for_each(|v| f(*v));
            f(*radius);
            for (pos, c) in stops {
                f(*pos);
                c.iter().for_each(|v| f(*v));
            }
        }
    };
    match shape {
        Shape::Fill { path, paint, rule } => {
            f(100.0 + *rule as u8 as f32);
            hash_path(path, &mut f);
            hash_paint(paint, &mut f);
        }
        Shape::Stroke { path, paint, style } => {
            f(200.0);
            f(style.width);
            f(style.join as u8 as f32);
            f(style.cap as u8 as f32);
            f(style.miter_limit);
            hash_path(path, &mut f);
            hash_paint(paint, &mut f);
        }
        Shape::Text { .. } | Shape::Clip(_) | Shape::ClipPath(_) => {}
    }
    hasher.finish()
}

fn combine(a: u64, b: u64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (a, b).hash(&mut hasher);
    hasher.finish()
}

/// 자르는 경로들과 배율의 열쇠.
fn clip_key(paths: &[&(Vec<extension::Segment>, FillRule)], scale: f32) -> u64 {
    let mut key = 0x5eed_u64;
    for (path, rule) in paths {
        let shape = Shape::Fill {
            path: path.clone(),
            paint: Paint::Solid([0.0; 4]),
            rule: *rule,
        };
        key = combine(key, shape_key(&shape, scale));
    }
    key
}

/// 자르는 경로들이 모두 겹친 곳(삼각형, px).
fn clip_region(paths: &[&(Vec<extension::Segment>, FillRule)], scale: f32) -> Vec<Tri> {
    let mut region: Option<Vec<Tri>> = None;
    for (path, rule) in paths {
        let tris = fill_triangles(path, scale, *rule);
        region = Some(match region {
            None => tris,
            Some(region) => intersect(&region, &tris),
        });
    }
    region.unwrap_or_default()
}

/// 두 삼각형 묶음이 겹친 곳. 테셀레이션 결과는 서로 겹치지 않으므로 결과도 겹치지 않는다.
fn intersect(a: &[Tri], b: &[Tri]) -> Vec<Tri> {
    let b: Vec<(&Tri, [f32; 4])> = b
        .iter()
        .filter(|tri| tri_area(tri) > 1e-6)
        .map(|tri| (tri, bbox(tri)))
        .collect();
    let mut out = Vec::new();
    for ta in a {
        let box_a = bbox(ta);
        for (tb, box_b) in &b {
            if overlaps(box_a, *box_b) {
                fan(&clip_convex(ta, &tb[..]), &mut out);
            }
        }
    }
    out
}

fn tri_area([a, b, c]: &Tri) -> f32 {
    ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
}

/// 모양을 삼각형으로 나누고 칠별로 조각을 만든다(캔버스 왼쪽 위가 원점, px).
fn build(shape: &Shape, scale: f32, region: Option<&[Tri]>) -> Vec<Piece> {
    let (triangles, paint) = match shape {
        Shape::Fill { path, paint, rule } => (fill_triangles(path, scale, *rule), paint),
        Shape::Stroke { path, paint, style } => {
            if style.width <= 0.0 {
                return Vec::new();
            }
            (stroke_triangles(path, scale, style), paint)
        }
        Shape::Text { .. } | Shape::Clip(_) | Shape::ClipPath(_) => return Vec::new(),
    };
    let triangles = match region {
        Some(region) => intersect(&triangles, region),
        None => triangles,
    };
    if triangles.is_empty() {
        return Vec::new();
    }
    let s = |p: P| [p[0] * scale, p[1] * scale];
    match paint {
        Paint::Solid(color) => vec![piece(&triangles, |_| hsla(*color).into())],
        Paint::Linear { from, to, stops } => linear_pieces(&triangles, s(*from), s(*to), stops),
        Paint::Radial {
            center,
            radius,
            stops,
        } => radial_pieces(&triangles, s(*center), radius * scale, stops),
    }
    .into_iter()
    .flatten()
    .collect()
}

fn scaled_lyon(path: &[extension::Segment], scale: f32) -> lyon::path::Path {
    extension::lyon_path(path).transformed(&lyon::math::Transform::scale(scale, scale))
}

fn collect(buffers: VertexBuffers<lyon::math::Point, u32>) -> Vec<Tri> {
    let v = |ix: u32| {
        let p = buffers.vertices[ix as usize];
        [p.x, p.y]
    };
    buffers
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|ix| [v(ix[0]), v(ix[1]), v(ix[2])])
        .collect()
}

fn fill_triangles(path: &[extension::Segment], scale: f32, rule: FillRule) -> Vec<Tri> {
    let path = scaled_lyon(path, scale);
    let rule = match rule {
        FillRule::NonZero => lyon::tessellation::FillRule::NonZero,
        FillRule::EvenOdd => lyon::tessellation::FillRule::EvenOdd,
    };
    let options = FillOptions::tolerance(TOLERANCE).with_fill_rule(rule);
    let mut buffers: VertexBuffers<lyon::math::Point, u32> = VertexBuffers::new();
    let result = FillTessellator::new().tessellate_path(
        &path,
        &options,
        &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| vertex.position()),
    );
    if result.is_err() {
        return Vec::new();
    }
    collect(buffers)
}

fn stroke_triangles(
    path: &[extension::Segment],
    scale: f32,
    style: &extension::StrokeStyle,
) -> Vec<Tri> {
    let path = scaled_lyon(path, scale);
    let join = match style.join {
        LineJoin::Miter => lyon::tessellation::LineJoin::Miter,
        LineJoin::Round => lyon::tessellation::LineJoin::Round,
        LineJoin::Bevel => lyon::tessellation::LineJoin::Bevel,
    };
    let cap = match style.cap {
        LineCap::Butt => lyon::tessellation::LineCap::Butt,
        LineCap::Round => lyon::tessellation::LineCap::Round,
        LineCap::Square => lyon::tessellation::LineCap::Square,
    };
    let options = StrokeOptions::tolerance(TOLERANCE)
        .with_line_width(style.width * scale)
        .with_line_join(join)
        .with_line_cap(cap)
        .with_miter_limit(style.miter_limit.max(1.0));
    let mut buffers: VertexBuffers<lyon::math::Point, u32> = VertexBuffers::new();
    let result = StrokeTessellator::new().tessellate_path(
        &path,
        &options,
        &mut BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex| vertex.position()),
    );
    if result.is_err() {
        return Vec::new();
    }
    collect(buffers)
}

/// 삼각형들로 경로 하나를 만들고, 그 테두리 상자로 칠을 정한다.
fn piece(
    triangles: &[Tri],
    background: impl FnOnce(Bounds<Pixels>) -> Background,
) -> Option<Piece> {
    let first = triangles.first()?;
    let pt = |p: P| point(px(p[0]), px(p[1]));
    let mut path = Path::new(pt(first[0]));
    let st = (point(0., 1.), point(0., 1.), point(0., 1.));
    for tri in triangles {
        path.push_triangle((pt(tri[0]), pt(tri[1]), pt(tri[2])), st);
    }
    let background = background(path.bounds);
    Some(Piece { path, background })
}

fn color_at(stops: &[(f32, extension::Color)], t: f32) -> extension::Color {
    let first = stops[0];
    if t <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let ((p0, c0), (p1, c1)) = (pair[0], pair[1]);
        if t <= p1 {
            let k = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
            return std::array::from_fn(|ix| c0[ix] + (c1[ix] - c0[ix]) * k);
        }
    }
    stops[stops.len() - 1].1
}

/// 캔버스 좌표의 `a`(색 `c0`)에서 `b`(색 `c1`)까지의 선형 그라디언트를, 테두리 상자가 `bounds`인 경로에
/// 줄 GPUI 칠로 바꾼다. GPUI 셰이더는 상자 가운데를 지나는 각도 방향으로 자리를 재고 짧은 변을 긴 변에 맞춰
/// 늘인다. 그 늘임을 거꾸로 해 방향이 `b - a`와 같아지는 각도를 고르고, 두 점의 자리를 셰이더의 자리로 바꾼다.
fn linear_background(
    bounds: Bounds<Pixels>,
    a: P,
    b: P,
    c0: extension::Color,
    c1: extension::Color,
) -> Background {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let v = [b[0] - a[0], b[1] - a[1]];
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if w <= 0.0 || h <= 0.0 || len < 1e-4 {
        return hsla(c1).into();
    }
    let (angle_deg, p0, p1) = gradient_params(
        [f32::from(bounds.origin.x), f32::from(bounds.origin.y), w, h],
        a,
        b,
    );
    linear_gradient(
        angle_deg,
        linear_color_stop(hsla(c0), p0),
        linear_color_stop(hsla(c1), p1),
    )
}

/// (각도, 시작 자리, 끝 자리): GPUI `linear_gradient`에 넘길 값. `bounds`는 (x, y, w, h).
fn gradient_params(bounds: [f32; 4], a: P, b: P) -> (f32, f32, f32) {
    let [ox, oy, w, h] = bounds;
    let v = [b[0] - a[0], b[1] - a[1]];
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    let theta = if w > h {
        (v[1] * w / h).atan2(v[0])
    } else {
        v[1].atan2(v[0] * h / w)
    };
    let angle_deg = theta.to_degrees() + 90.0;
    let u = [v[0] / len, v[1] / len];
    let center = [ox + w / 2.0, oy + h / 2.0];
    let long = if v[0].abs() > v[1].abs() { w } else { h };
    let shift = (center[0] - a[0]) * u[0] + (center[1] - a[1]) * u[1];
    let at = |s: f32| (s * len - shift + long / 2.0) / long;
    (angle_deg, at(0.0), at(1.0))
}

fn linear_pieces(
    triangles: &[Tri],
    a: P,
    b: P,
    stops: &[(f32, extension::Color)],
) -> Vec<Option<Piece>> {
    let v = [b[0] - a[0], b[1] - a[1]];
    let len2 = v[0] * v[0] + v[1] * v[1];
    if len2 < 1e-8 {
        let color = stops[stops.len() - 1].1;
        return vec![piece(triangles, |_| hsla(color).into())];
    }
    if stops.len() == 2 {
        let (c0, c1) = (stops[0].1, stops[1].1);
        let (pa, pb) = (lerp_point(a, b, stops[0].0), lerp_point(a, b, stops[1].0));
        return vec![piece(triangles, |bounds| {
            linear_background(bounds, pa, pb, c0, c1)
        })];
    }
    // 멈춤 사이마다 띠 하나. 처음과 끝 띠는 바깥으로 끝없이 넓힌다.
    let s = move |p: P| ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / len2;
    let n = stops.len();
    (0..n - 1)
        .map(|ix| {
            let lo = (ix > 0).then_some(stops[ix].0);
            let hi = (ix < n - 2).then_some(stops[ix + 1].0);
            let mut band = Vec::new();
            for tri in triangles {
                let mut poly = tri.to_vec();
                if let Some(lo) = lo {
                    poly = clip_poly(&poly, |p| s(p) - lo);
                }
                if let Some(hi) = hi {
                    poly = clip_poly(&poly, |p| hi - s(p));
                }
                fan(&poly, &mut band);
            }
            let ((p0, c0), (p1, c1)) = (stops[ix], stops[ix + 1]);
            let (pa, pb) = (lerp_point(a, b, p0), lerp_point(a, b, p1));
            piece(&band, |bounds| linear_background(bounds, pa, pb, c0, c1))
        })
        .collect()
}

fn radial_pieces(
    triangles: &[Tri],
    center: P,
    radius: f32,
    stops: &[(f32, extension::Color)],
) -> Vec<Option<Piece>> {
    let last = stops[stops.len() - 1].1;
    let dist = |p: P| ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt();
    let far = triangles
        .iter()
        .flatten()
        .map(|p| dist(*p))
        .fold(0.0_f32, f32::max);
    if radius <= 0.0 {
        return vec![piece(triangles, |_| hsla(last).into())];
    }
    // 고리 경계: 중심, 멈춤 자리, 반지름, 가장 먼 점.
    let mut rings: Vec<f32> = std::iter::once(0.0)
        .chain(stops.iter().map(|(pos, _)| pos * radius))
        .chain([radius, far.max(radius) + 1.0])
        .collect();
    rings.sort_by(f32::total_cmp);
    rings.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    let outer = rings[rings.len() - 1];
    // 현과 원의 차이가 RADIAL_CHORD_ERROR 안쪽이 되는 부채꼴 수.
    let sectors = if outer <= RADIAL_CHORD_ERROR * 2.0 {
        8
    } else {
        (PI / (1.0 - RADIAL_CHORD_ERROR / outer).acos()).ceil() as usize
    }
    .clamp(8, 512);
    let at = |r: f32, angle: f32| [center[0] + r * angle.cos(), center[1] + r * angle.sin()];
    let tri_bounds: Vec<[f32; 4]> = triangles.iter().map(|tri| bbox(tri)).collect();
    let mut out = Vec::new();
    for pair in rings.windows(2) {
        let (r0, r1) = (pair[0], pair[1]);
        let (c0, c1) = (color_at(stops, r0 / radius), color_at(stops, r1 / radius));
        for j in 0..sectors {
            let (a0, a1) = (
                TAU * j as f32 / sectors as f32,
                TAU * (j + 1) as f32 / sectors as f32,
            );
            let cell: Vec<P> = if r0 <= 0.0 {
                vec![center, at(r1, a0), at(r1, a1)]
            } else {
                vec![at(r0, a0), at(r1, a0), at(r1, a1), at(r0, a1)]
            };
            let cell_box = bbox(&cell);
            let mut part = Vec::new();
            for (tri, tri_box) in triangles.iter().zip(&tri_bounds) {
                if !overlaps(cell_box, *tri_box) {
                    continue;
                }
                fan(&clip_convex(tri, &cell), &mut part);
            }
            if part.is_empty() {
                continue;
            }
            let mid = (a0 + a1) / 2.0;
            let (pa, pb) = (at(r0, mid), at(r1, mid));
            out.push(piece(&part, |bounds| {
                linear_background(bounds, pa, pb, c0, c1)
            }));
        }
    }
    out
}

fn lerp_point(a: P, b: P, t: f32) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn bbox(points: &[P]) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in points {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    b
}

fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// 볼록 다각형을 `f(p) >= 0`인 쪽으로 자른다(Sutherland–Hodgman).
fn clip_poly(poly: &[P], f: impl Fn(P) -> f32) -> Vec<P> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for ix in 0..poly.len() {
        let (a, b) = (poly[ix], poly[(ix + 1) % poly.len()]);
        let (fa, fb) = (f(a), f(b));
        if fa >= 0.0 {
            out.push(a);
        }
        if (fa >= 0.0) != (fb >= 0.0) {
            let t = fa / (fa - fb);
            out.push(lerp_point(a, b, t));
        }
    }
    out
}

/// 삼각형을 볼록 다각형 `cell` 안쪽으로 자른다.
fn clip_convex(tri: &Tri, cell: &[P]) -> Vec<P> {
    let n = cell.len();
    // 안쪽은 칸이 감는 방향(부호 있는 넓이)으로 정한다. 아주 가는 칸에서도 흔들리지 않는다.
    let twice_area: f32 = (0..n)
        .map(|ix| {
            let (a, b) = (cell[ix], cell[(ix + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if twice_area.abs() < 1e-9 {
        return Vec::new();
    }
    let orient = twice_area.signum();
    let mut poly = tri.to_vec();
    for ix in 0..n {
        if poly.is_empty() {
            break;
        }
        let (a, b) = (cell[ix], cell[(ix + 1) % n]);
        let side = |p: P| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        poly = clip_poly(&poly, |p| side(p) * orient);
    }
    poly
}

/// 볼록 다각형을 부채꼴 삼각형으로.
fn fan(poly: &[P], out: &mut Vec<Tri>) {
    for ix in 1..poly.len().saturating_sub(1) {
        out.push([poly[0], poly[ix], poly[ix + 1]]);
    }
}

#[cfg(test)]
mod tests {
    // `super::*`는 gpui의 `test` 매크로까지 들여와 `#[test]`를 가린다.
    use super::{
        FillRule, P, Piece, TAU, Tri, clip_convex, clip_region, color_at, fan, gradient_params,
        intersect, linear_pieces, radial_pieces,
    };
    use crate::extension::Segment;

    /// GPUI 셰이더(`fill_color`의 선형 그라디언트)를 그대로 옮긴 계산: 점 `p`의 t(0..1로 자르기 전).
    fn shader_t(bounds: [f32; 4], angle: f32, p0: f32, p1: f32, p: P) -> f32 {
        let [ox, oy, w, h] = bounds;
        let radians = ((angle % 360.0) - 90.0).to_radians();
        let mut dir = [radians.cos(), radians.sin()];
        if w > h {
            dir[1] *= h / w;
        } else {
            dir[0] *= w / h;
        }
        let half = [w / 2.0, h / 2.0];
        let center = [ox + half[0], oy + half[1]];
        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
        let mut t = ((p[0] - center[0]) * dir[0] + (p[1] - center[1]) * dir[1]) / len;
        t = if dir[0].abs() > dir[1].abs() {
            (t + half[0]) / w
        } else {
            (t + half[1]) / h
        };
        (t - p0) / (p1 - p0)
    }

    #[test]
    fn gradient_params_match_the_shader() {
        let cases: [([f32; 4], P, P); 4] = [
            ([0.0, 0.0, 200.0, 50.0], [10.0, 0.0], [150.0, 40.0]),
            ([30.0, 20.0, 40.0, 120.0], [30.0, 140.0], [70.0, 20.0]),
            ([0.0, 0.0, 100.0, 100.0], [-50.0, 50.0], [150.0, 50.0]),
            ([5.0, 5.0, 300.0, 10.0], [100.0, 5.0], [100.0, 15.0]),
        ];
        for (bounds, a, b) in cases {
            let (angle, p0, p1) = gradient_params(bounds, a, b);
            let v = [b[0] - a[0], b[1] - a[1]];
            let len2 = v[0] * v[0] + v[1] * v[1];
            for p in [
                [bounds[0], bounds[1]],
                [bounds[0] + bounds[2], bounds[1] + bounds[3]],
                [bounds[0] + bounds[2] * 0.3, bounds[1] + bounds[3] * 0.8],
            ] {
                let want = ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / len2;
                let got = shader_t(bounds, angle, p0, p1, p);
                assert!(
                    (want - got).abs() < 1e-3,
                    "{bounds:?} {a:?} {b:?} {p:?}: {want} vs {got}"
                );
            }
        }
    }

    fn area(tris: &[Tri]) -> f32 {
        tris.iter()
            .map(|[a, b, c]| {
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn clipping_keeps_the_area() {
        // 정사각형 100x100을 세 띠로 자르면 넓이 합이 그대로다.
        let square: Vec<Tri> = vec![
            [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]],
            [[0.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
        ];
        let stops = [
            (0.0, [1.0, 0.0, 0.0, 1.0]),
            (0.3, [0.0, 1.0, 0.0, 1.0]),
            (1.0, [0.0, 0.0, 1.0, 1.0]),
        ];
        let pieces = linear_pieces(&square, [0.0, 0.0], [100.0, 0.0], &stops);
        assert_eq!(pieces.len(), 2);
        // 칸으로 자른 방형도 넓이를 지킨다.
        let mut total = 0.0;
        for j in 0..16 {
            let a0 = TAU * j as f32 / 16.0;
            let a1 = TAU * (j + 1) as f32 / 16.0;
            let cell = vec![
                [50.0, 50.0],
                [50.0 + 200.0 * a0.cos(), 50.0 + 200.0 * a0.sin()],
                [50.0 + 200.0 * a1.cos(), 50.0 + 200.0 * a1.sin()],
            ];
            let mut part = Vec::new();
            for tri in &square {
                fan(&clip_convex(tri, &cell), &mut part);
            }
            total += area(&part);
        }
        assert!((total - 10_000.0).abs() < 1.0, "{total}");
    }

    #[test]
    fn radial_cells_cover_the_shape() {
        let square: Vec<Tri> = vec![
            [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]],
            [[0.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
        ];
        let stops = [(0.0, [1.0; 4]), (1.0, [0.0, 0.0, 0.0, 1.0])];
        let pieces: Vec<Piece> = radial_pieces(&square, [50.0, 50.0], 40.0, &stops)
            .into_iter()
            .flatten()
            .collect();
        assert!(!pieces.is_empty());
        let covered: f32 = pieces
            .iter()
            .map(|piece| {
                piece
                    .path
                    .vertices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|v| {
                        let p = |ix: usize| {
                            [
                                f32::from(v[ix].xy_position.x),
                                f32::from(v[ix].xy_position.y),
                            ]
                        };
                        area(&[[p(0), p(1), p(2)]])
                    })
                    .sum::<f32>()
            })
            .sum();
        assert!((covered - 10_000.0).abs() < 2.0, "{covered}");
    }

    #[test]
    fn clip_paths_keep_only_the_overlap() {
        let square = |x: f32, y: f32, w: f32| {
            vec![
                Segment::Move([x, y]),
                Segment::Line([x + w, y]),
                Segment::Line([x + w, y + w]),
                Segment::Line([x, y + w]),
                Segment::Close,
            ]
        };
        let (a, b) = (
            (square(0.0, 0.0, 10.0), FillRule::NonZero),
            (square(5.0, 5.0, 10.0), FillRule::NonZero),
        );
        // 두 정사각형이 겹친 5x5, 배율 2면 10x10.
        let region = clip_region(&[&a, &b], 2.0);
        assert!((area(&region) - 100.0).abs() < 0.01, "{}", area(&region));
        // 모양(0..20 정사각형, px)을 그 영역으로 자르면 넓이가 영역과 같다.
        let shape: Vec<Tri> = vec![
            [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0]],
            [[0.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
        ];
        assert!((area(&intersect(&shape, &region)) - 100.0).abs() < 0.01);
        // 겹치지 않으면 빈 영역.
        let far = (square(100.0, 100.0, 5.0), FillRule::NonZero);
        assert!(clip_region(&[&a, &far], 1.0).is_empty());
    }

    #[test]
    fn clipping_by_a_circle_stays_inside() {
        // 원(290, 60, r 45)으로 사각형(240, 10, 100x100)을 자른다(화면의 배율 몇 가지).
        let circle = |cx: f32, cy: f32, r: f32| {
            let k = 0.552_284_8 * r;
            vec![
                Segment::Move([cx + r, cy]),
                Segment::Cubic([cx + r, cy + k], [cx + k, cy + r], [cx, cy + r]),
                Segment::Cubic([cx - k, cy + r], [cx - r, cy + k], [cx - r, cy]),
                Segment::Cubic([cx - r, cy - k], [cx - k, cy - r], [cx, cy - r]),
                Segment::Cubic([cx + k, cy - r], [cx + r, cy - k], [cx + r, cy]),
                Segment::Close,
            ]
        };
        for scale in [1.0, 1.18, 2.0, 2.36] {
            let clip = (circle(290.0, 60.0, 45.0), FillRule::NonZero);
            let region = clip_region(&[&clip], scale);
            let rect = vec![
                Segment::Move([240.0, 10.0]),
                Segment::Line([340.0, 10.0]),
                Segment::Line([340.0, 110.0]),
                Segment::Line([240.0, 110.0]),
                Segment::Close,
            ];
            let tris = super::fill_triangles(&rect, scale, FillRule::NonZero);
            let clipped = intersect(&tris, &region);
            let want = std::f32::consts::PI * (45.0 * scale).powi(2);
            assert!(
                (area(&clipped) - want).abs() < want * 0.005,
                "{scale}: {} vs {want}",
                area(&clipped)
            );
            // 모든 점이 원 안(허용 오차 1px).
            for p in clipped.iter().flatten() {
                let d = ((p[0] - 290.0 * scale).powi(2) + (p[1] - 60.0 * scale).powi(2)).sqrt();
                assert!(d <= 45.0 * scale + 1.0, "{scale}: {p:?} at {d}");
            }
        }
    }

    #[test]
    fn color_stops_interpolate() {
        let stops = [(0.2, [0.0; 4]), (0.6, [1.0; 4])];
        assert_eq!(color_at(&stops, 0.0), [0.0; 4]);
        assert_eq!(color_at(&stops, 1.0), [1.0; 4]);
        assert!((color_at(&stops, 0.4)[0] - 0.5).abs() < 1e-5);
    }
}
