//! 확장의 벡터 경로(`g.newPath`)와 그라디언트(`g.newLinearGradient`·`g.newRadialGradient`).
//!
//! 경로는 지역 좌표의 선분 목록이다. 그릴 때(`g.fill`·`g.stroke`) 그때의 변환을 적용해 모양 목록에 넣는다.
//! 그래서 한 번 만든 경로를 여러 자리에 여러 번 그릴 수 있다. 바꾸는 메서드(`translate`·`withPoints`·
//! `trim` 등)는 새 경로를 돌려주고, 만드는 메서드(`moveTo`·`curveTo` 등)는 그 경로에 더하고 자신을 돌려준다.
//!
//! 길이·위치·자르기·점선은 lyon의 경로 측정으로 한다. 곡선은 잘라도 곡선으로 남는다.

use std::f32::consts::TAU;

use lyon::algorithms::aabb::bounding_box;
use lyon::algorithms::measure::{PathMeasurements, SampleType};
use lyon::math::point;
use lyon::path::{Event, Path as LyonPath};
use mlua::{AnyUserData, MultiValue, UserData, UserDataMethods, Value};

use super::canvas::{Args, Color, Point, Segment, ellipse_arc, polyline, rounded_rect};

/// 길이를 잴 때 곡선을 펴는 허용 오차(지역 좌표).
const MEASURE_TOLERANCE: f32 = 0.01;

/// SVG 경로 문자열의 가장 긴 길이. 끝없는 입력을 막는다.
const MAX_SVG_LEN: usize = 200_000;

/// Lua의 경로 값.
#[derive(Clone, Debug, Default)]
pub(super) struct LuaPath {
    pub(super) segments: Vec<Segment>,
}

/// 경로의 마지막 점과 지금 하위 경로의 시작점.
fn cursor(segments: &[Segment]) -> (Option<Point>, Option<Point>) {
    let mut current = None;
    let mut start = None;
    for segment in segments {
        match *segment {
            Segment::Move(p) => {
                current = Some(p);
                start = Some(p);
            }
            Segment::Line(p) | Segment::Cubic(_, _, p) => {
                if start.is_none() {
                    start = Some(p);
                }
                current = Some(p);
            }
            Segment::Close => current = start,
        }
    }
    (current, start)
}

/// 2차 베지어를 3차로 바꾼다.
fn quad_to_cubic(from: Point, ctrl: Point, to: Point) -> Segment {
    let lerp = |a: Point, b: Point, t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    Segment::Cubic(lerp(from, ctrl, 2.0 / 3.0), lerp(to, ctrl, 2.0 / 3.0), to)
}

impl LuaPath {
    fn move_to(&mut self, p: Point) {
        self.segments.push(Segment::Move(p));
    }

    /// 지금 점이 없으면 `p`에서 시작한다(캔버스 API와 같다).
    fn line_to(&mut self, p: Point) {
        match cursor(&self.segments).0 {
            Some(_) => self.segments.push(Segment::Line(p)),
            None => self.move_to(p),
        }
    }

    fn cubic_to(&mut self, c1: Point, c2: Point, p: Point) {
        if cursor(&self.segments).0.is_none() {
            self.move_to(c1);
        }
        self.segments.push(Segment::Cubic(c1, c2, p));
    }

    fn quad_to(&mut self, ctrl: Point, p: Point) {
        let from = match cursor(&self.segments).0 {
            Some(from) => from,
            None => {
                self.move_to(ctrl);
                ctrl
            }
        };
        self.segments.push(quad_to_cubic(from, ctrl, p));
    }

    fn close(&mut self) {
        if !matches!(self.segments.last(), None | Some(Segment::Close)) {
            self.segments.push(Segment::Close);
        }
    }

    /// 원호. 지금 점이 있으면 호의 시작점까지 선으로 잇는다(캔버스 `arc`와 같다).
    fn arc(&mut self, center: Point, radius: f32, a1: f32, a2: f32) {
        let start = [center[0] + radius * a1.cos(), center[1] + radius * a1.sin()];
        if cursor(&self.segments).0.is_some() {
            self.segments.push(Segment::Line(start));
        } else {
            self.move_to(start);
        }
        ellipse_arc(&mut self.segments, center, [radius, radius], a1, a2, false);
    }

    /// 지역 좌표의 lyon 경로.
    pub(super) fn to_lyon(segments: &[Segment]) -> LyonPath {
        let mut builder = LyonPath::builder();
        let mut open = false;
        let mut start = None;
        let pt = |p: Point| point(p[0], p[1]);
        for segment in segments {
            match *segment {
                Segment::Move(p) => {
                    if open {
                        builder.end(false);
                    }
                    builder.begin(pt(p));
                    open = true;
                    start = Some(p);
                }
                Segment::Line(p) => {
                    if !open {
                        builder.begin(pt(start.unwrap_or(p)));
                        open = true;
                    }
                    builder.line_to(pt(p));
                }
                Segment::Cubic(a, b, p) => {
                    if !open {
                        builder.begin(pt(start.unwrap_or(a)));
                        open = true;
                    }
                    builder.cubic_bezier_to(pt(a), pt(b), pt(p));
                }
                Segment::Close => {
                    if open {
                        builder.end(true);
                        open = false;
                    }
                }
            }
        }
        if open {
            builder.end(false);
        }
        builder.build()
    }

    pub(super) fn from_lyon(path: &LyonPath) -> Vec<Segment> {
        let p = |p: lyon::math::Point| [p.x, p.y];
        let mut out = Vec::new();
        for event in path.iter() {
            match event {
                Event::Begin { at } => out.push(Segment::Move(p(at))),
                Event::Line { to, .. } => out.push(Segment::Line(p(to))),
                Event::Quadratic { from, ctrl, to } => {
                    out.push(quad_to_cubic(p(from), p(ctrl), p(to)))
                }
                Event::Cubic {
                    ctrl1, ctrl2, to, ..
                } => out.push(Segment::Cubic(p(ctrl1), p(ctrl2), p(to))),
                Event::End { close, .. } => {
                    if close {
                        out.push(Segment::Close);
                    }
                }
            }
        }
        out
    }

    /// 모든 점(앵커와 조절점)을 차례로: Move·Line은 x, y, Cubic은 c1x, c1y, c2x, c2y, x, y.
    fn points(&self) -> Vec<f32> {
        let mut out = Vec::new();
        for segment in &self.segments {
            match *segment {
                Segment::Move(p) | Segment::Line(p) => out.extend(p),
                Segment::Cubic(a, b, p) => {
                    out.extend(a);
                    out.extend(b);
                    out.extend(p);
                }
                Segment::Close => {}
            }
        }
        out
    }

    /// 구조는 그대로 두고 점만 바꾼다.
    fn with_points(&self, values: &[f32]) -> Result<Self, String> {
        let want = self.points().len();
        if values.len() != want {
            return Err(format!(
                "expected {want} numbers (the same count as path:points()), got {}",
                values.len()
            ));
        }
        let mut ix = 0;
        let mut next = || {
            let p = [values[ix], values[ix + 1]];
            ix += 2;
            p
        };
        let segments = self
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Move(_) => Segment::Move(next()),
                Segment::Line(_) => Segment::Line(next()),
                Segment::Cubic(..) => {
                    let (a, b, p) = (next(), next(), next());
                    Segment::Cubic(a, b, p)
                }
                Segment::Close => Segment::Close,
            })
            .collect();
        Ok(Self { segments })
    }

    fn map_points(&self, f: impl Fn(Point) -> Point) -> Self {
        let segments = self
            .segments
            .iter()
            .map(|segment| match *segment {
                Segment::Move(p) => Segment::Move(f(p)),
                Segment::Line(p) => Segment::Line(f(p)),
                Segment::Cubic(a, b, p) => Segment::Cubic(f(a), f(b), f(p)),
                Segment::Close => Segment::Close,
            })
            .collect();
        Self { segments }
    }

    /// 구조가 같은 두 경로 사이(`t` = 0이면 `self`, 1이면 `other`).
    pub(super) fn lerp(&self, other: &Self, t: f32) -> Result<Self, String> {
        let same = self.segments.len() == other.segments.len()
            && self
                .segments
                .iter()
                .zip(&other.segments)
                .all(|(a, b)| std::mem::discriminant(a) == std::mem::discriminant(b));
        if !same {
            return Err("both paths must have the same structure (the same commands in the same order); build them with the same calls and change only the numbers".into());
        }
        let (a, b) = (self.points(), other.points());
        let mixed: Vec<f32> = a.iter().zip(&b).map(|(a, b)| a + (b - a) * t).collect();
        self.with_points(&mixed)
    }

    pub(super) fn length(segments: &[Segment]) -> f32 {
        let path = Self::to_lyon(segments);
        let measurements = PathMeasurements::from_path(&path, MEASURE_TOLERANCE);
        measurements.length()
    }

    /// 길이의 `t`(0..1) 자리의 점과 그 자리의 방향(라디안).
    fn point_at(&self, t: f32) -> Option<(Point, f32)> {
        let path = Self::to_lyon(&self.segments);
        let measurements = PathMeasurements::from_path(&path, MEASURE_TOLERANCE);
        if measurements.length() <= 0.0 {
            return None;
        }
        let mut sampler = measurements.create_sampler(&path, SampleType::Normalized);
        let sample = sampler.sample(t.clamp(0.0, 1.0));
        let (at, tangent) = (sample.position(), sample.tangent());
        Some(([at.x, at.y], tangent.y.atan2(tangent.x)))
    }

    /// 길이의 `from`..`to`(0..1) 부분. 곡선은 잘라도 곡선이다.
    pub(super) fn trim(segments: &[Segment], from: f32, to: f32) -> Vec<Segment> {
        let (from, to) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0));
        if to <= from {
            return Vec::new();
        }
        let path = Self::to_lyon(segments);
        let measurements = PathMeasurements::from_path(&path, MEASURE_TOLERANCE);
        if measurements.length() <= 0.0 {
            return Vec::new();
        }
        let mut sampler = measurements.create_sampler(&path, SampleType::Normalized);
        let mut builder = LyonPath::builder();
        sampler.split_range(from..to, &mut builder);
        Self::from_lyon(&builder.build())
    }

    /// 점선: `pattern`(그림, 빈칸, …)을 `offset`만큼 밀어 경로를 나눈다.
    pub(super) fn dash(segments: &[Segment], pattern: &[f32], offset: f32) -> Vec<Segment> {
        let period: f32 = pattern.iter().sum();
        if period <= 0.0 {
            return segments.to_vec();
        }
        let path = Self::to_lyon(segments);
        let measurements = PathMeasurements::from_path(&path, MEASURE_TOLERANCE);
        let total = measurements.length();
        if total <= 0.0 {
            return Vec::new();
        }
        let mut sampler = measurements.create_sampler(&path, SampleType::Distance);
        let mut builder = LyonPath::builder();
        // 시작 자리를 offset만큼 앞으로 당긴다(양수 offset은 무늬를 앞쪽으로 민다).
        let mut pos = -offset.rem_euclid(period);
        let mut ix = 0;
        // 너무 잘게 나뉘지 않게 상한을 둔다.
        let mut guard = 0;
        while pos < total && guard < 20_000 {
            let len = pattern[ix % pattern.len()];
            let end = pos + len;
            if ix % 2 == 0 && end > 0.0 {
                sampler.split_range(pos.max(0.0)..end.min(total), &mut builder);
            }
            pos = end;
            ix += 1;
            guard += 1;
        }
        Self::from_lyon(&builder.build())
    }

    /// (x, y, w, h). 비었으면 `None`.
    fn bounds(&self) -> Option<[f32; 4]> {
        if self.segments.iter().all(|s| matches!(s, Segment::Close)) {
            return None;
        }
        let path = Self::to_lyon(&self.segments);
        let b = bounding_box(path.iter());
        Some([b.min.x, b.min.y, b.width(), b.height()])
    }
}

/// SVG 경로 문자열(`d` 속성)을 선분 목록으로 읽는다. M L H V C S Q T A Z와 소문자(상대 좌표)를 안다.
pub(super) fn parse_svg(d: &str) -> Result<Vec<Segment>, String> {
    if d.len() > MAX_SVG_LEN {
        return Err(format!("the path string is too long (limit {MAX_SVG_LEN})"));
    }
    let mut lexer = Lexer {
        bytes: d.as_bytes(),
        pos: 0,
    };
    let mut path = LuaPath::default();
    let mut command: Option<u8> = None;
    let mut current: Point = [0.0, 0.0];
    let mut start: Point = [0.0, 0.0];
    // S·T가 이어 쓰는 앞 조절점.
    let mut last_cubic_ctrl: Option<Point> = None;
    let mut last_quad_ctrl: Option<Point> = None;
    loop {
        lexer.skip_separators();
        let Some(&byte) = lexer.bytes.get(lexer.pos) else {
            break;
        };
        if byte.is_ascii_alphabetic() {
            if !b"MLHVCSQTAZmlhvcsqtaz".contains(&byte) {
                return Err(format!(
                    "unknown path command '{}' at position {}",
                    byte as char,
                    lexer.pos + 1
                ));
            }
            lexer.pos += 1;
            command = Some(byte);
            if matches!(byte, b'Z' | b'z') {
                path.close();
                current = start;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                command = None;
            }
            continue;
        }
        let Some(cmd) = command else {
            return Err(format!(
                "expected a command letter at position {}",
                lexer.pos + 1
            ));
        };
        let relative = cmd.is_ascii_lowercase();
        let base = if relative { current } else { [0.0, 0.0] };
        let abs = |p: Point| [p[0] + base[0], p[1] + base[1]];
        let mut cubic_ctrl = None;
        let mut quad_ctrl = None;
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let p = abs(lexer.pair()?);
                path.move_to(p);
                current = p;
                start = p;
                // 이어지는 좌표 쌍은 L(l)이다.
                command = Some(if relative { b'l' } else { b'L' });
            }
            b'L' => {
                let p = abs(lexer.pair()?);
                path.line_to(p);
                current = p;
            }
            b'H' => {
                let x = lexer.number()? + base[0];
                let p = [x, current[1]];
                path.line_to(p);
                current = p;
            }
            b'V' => {
                let y = lexer.number()? + base[1];
                let p = [current[0], y];
                path.line_to(p);
                current = p;
            }
            b'C' => {
                let (c1, c2, p) = (abs(lexer.pair()?), abs(lexer.pair()?), abs(lexer.pair()?));
                path.cubic_to(c1, c2, p);
                cubic_ctrl = Some(c2);
                current = p;
            }
            b'S' => {
                let c1 = match last_cubic_ctrl {
                    Some(c) => [2.0 * current[0] - c[0], 2.0 * current[1] - c[1]],
                    None => current,
                };
                let (c2, p) = (abs(lexer.pair()?), abs(lexer.pair()?));
                path.cubic_to(c1, c2, p);
                cubic_ctrl = Some(c2);
                current = p;
            }
            b'Q' => {
                let (c, p) = (abs(lexer.pair()?), abs(lexer.pair()?));
                path.quad_to(c, p);
                quad_ctrl = Some(c);
                current = p;
            }
            b'T' => {
                let c = match last_quad_ctrl {
                    Some(c) => [2.0 * current[0] - c[0], 2.0 * current[1] - c[1]],
                    None => current,
                };
                let p = abs(lexer.pair()?);
                path.quad_to(c, p);
                quad_ctrl = Some(c);
                current = p;
            }
            b'A' => {
                let (rx, ry) = (lexer.number()?, lexer.number()?);
                let rotation = lexer.number()?;
                let large = lexer.flag()?;
                let sweep = lexer.flag()?;
                let p = abs(lexer.pair()?);
                if cursor(&path.segments).0.is_none() {
                    path.move_to(current);
                }
                svg_arc(
                    &mut path.segments,
                    current,
                    rx,
                    ry,
                    rotation,
                    large,
                    sweep,
                    p,
                );
                current = p;
            }
            other => {
                return Err(format!(
                    "unknown path command '{}' at position {}",
                    other as char, lexer.pos
                ));
            }
        }
        last_cubic_ctrl = cubic_ctrl;
        last_quad_ctrl = quad_ctrl;
    }
    Ok(path.segments)
}

struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Lexer<'_> {
    fn skip_separators(&mut self) {
        while let Some(b) = self.bytes.get(self.pos) {
            if b.is_ascii_whitespace() || *b == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn number(&mut self) -> Result<f32, String> {
        self.skip_separators();
        let start = self.pos;
        let bytes = self.bytes;
        let mut end = start;
        if matches!(bytes.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        let mut seen_dot = false;
        let mut digits = 0;
        while let Some(&b) = bytes.get(end) {
            if b.is_ascii_digit() {
                digits += 1;
                end += 1;
            } else if b == b'.' && !seen_dot {
                seen_dot = true;
                end += 1;
            } else {
                break;
            }
        }
        if digits > 0 && matches!(bytes.get(end), Some(b'e' | b'E')) {
            let mut exp = end + 1;
            if matches!(bytes.get(exp), Some(b'+' | b'-')) {
                exp += 1;
            }
            if bytes.get(exp).is_some_and(u8::is_ascii_digit) {
                while bytes.get(exp).is_some_and(u8::is_ascii_digit) {
                    exp += 1;
                }
                end = exp;
            }
        }
        if digits == 0 {
            return Err(format!("expected a number at position {}", start + 1));
        }
        self.pos = end;
        std::str::from_utf8(&bytes[start..end])
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| format!("bad number at position {}", start + 1))
    }

    fn pair(&mut self) -> Result<Point, String> {
        Ok([self.number()?, self.number()?])
    }

    /// 호의 깃발(0 또는 1). 붙여 써도 된다(`a1 1 0 01 2 3`).
    fn flag(&mut self) -> Result<bool, String> {
        self.skip_separators();
        match self.bytes.get(self.pos) {
            Some(b'0') => {
                self.pos += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err(format!(
                "expected an arc flag (0 or 1) at position {}",
                self.pos + 1
            )),
        }
    }
}

/// SVG의 끝점 호를 중심 호로 바꿔 3차 베지어로 잇는다(SVG 1.1 부록 F.6).
#[allow(clippy::too_many_arguments)]
fn svg_arc(
    out: &mut Vec<Segment>,
    from: Point,
    rx: f32,
    ry: f32,
    rotation_deg: f32,
    large: bool,
    sweep: bool,
    to: Point,
) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if from == to {
        return;
    }
    if rx == 0.0 || ry == 0.0 {
        out.push(Segment::Line(to));
        return;
    }
    let phi = rotation_deg.to_radians();
    let (sin, cos) = phi.sin_cos();
    let dx = (from[0] - to[0]) / 2.0;
    let dy = (from[1] - to[1]) / 2.0;
    let x1 = cos * dx + sin * dy;
    let y1 = -sin * dx + cos * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = (num / den).max(0.0).sqrt();
    if large == sweep {
        coef = -coef;
    }
    let cx1 = coef * rx * y1 / ry;
    let cy1 = -coef * ry * x1 / rx;
    let cx = cos * cx1 - sin * cy1 + (from[0] + to[0]) / 2.0;
    let cy = sin * cx1 + cos * cy1 + (from[1] + to[1]) / 2.0;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let theta1 = angle(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut delta = angle(
        (x1 - cx1) / rx,
        (y1 - cy1) / ry,
        (-x1 - cx1) / rx,
        (-y1 - cy1) / ry,
    );
    if !sweep && delta > 0.0 {
        delta -= TAU;
    } else if sweep && delta < 0.0 {
        delta += TAU;
    }
    // 축에 맞춘 타원 호를 만든 뒤 돌린다.
    let mut arc = Vec::new();
    ellipse_arc(
        &mut arc,
        [0.0, 0.0],
        [rx, ry],
        theta1,
        theta1 + delta,
        false,
    );
    let place = |[x, y]: Point| [cos * x - sin * y + cx, sin * x + cos * y + cy];
    for segment in arc {
        if let Segment::Cubic(a, b, p) = segment {
            out.push(Segment::Cubic(place(a), place(b), place(p)));
        }
    }
    // 끝점은 정확히 맞춘다(부동소수 오차).
    if let Some(Segment::Cubic(_, _, end)) = out.last_mut() {
        *end = to;
    }
}

/// 함수 인자에서 경로를 읽는다: 경로 값 또는 SVG 경로 문자열.
pub(super) fn path_arg(args: &Args, ix: usize) -> mlua::Result<Vec<Segment>> {
    match args.values.get(ix) {
        Some(Value::UserData(ud)) => {
            let path = ud
                .borrow::<LuaPath>()
                .map_err(|_| args.error(format!("argument #{} must be a path", ix + 1)))?;
            Ok(path.segments.clone())
        }
        Some(Value::String(s)) => parse_svg(&s.to_string_lossy()).map_err(|err| args.error(err)),
        _ => Err(args.error(format!(
            "argument #{} must be a path (g.newPath()) or an SVG path string",
            ix + 1
        ))),
    }
}

/// 경로 메서드 이름(오류 안내와 스킬 문서에 쓴다).
pub(super) const PATH_METHODS: &[&str] = &[
    "moveTo",
    "lineTo",
    "curveTo",
    "quadTo",
    "arc",
    "rect",
    "circle",
    "ellipse",
    "polygon",
    "svg",
    "close",
    "clone",
    "length",
    "pointAt",
    "trim",
    "bounds",
    "points",
    "withPoints",
    "lerp",
    "translate",
    "rotate",
    "scale",
];

fn new_path(lua: &mlua::Lua, path: LuaPath) -> mlua::Result<AnyUserData> {
    lua.create_userdata(path)
}

fn numbers_out(values: impl IntoIterator<Item = f32>) -> MultiValue {
    values
        .into_iter()
        .map(|v| Value::Number(f64::from(v)))
        .collect()
}

impl UserData for LuaPath {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // 만드는 메서드: 경로에 더하고 자신을 돌려준다(이어 부를 수 있다).
        macro_rules! build {
            ($name:literal, |$path:ident, $args:ident| $body:expr) => {
                methods.add_function(
                    $name,
                    |_, (ud, values): (AnyUserData, MultiValue)| -> mlua::Result<AnyUserData> {
                        let $args = Args::method("path:", $name, values);
                        {
                            let mut $path = ud.borrow_mut::<LuaPath>()?;
                            $body;
                        }
                        Ok(ud)
                    },
                );
            };
        }
        build!("moveTo", |path, args| path
            .move_to([args.number(0)?, args.number(1)?]));
        build!("lineTo", |path, args| path
            .line_to([args.number(0)?, args.number(1)?]));
        build!("curveTo", |path, args| path.cubic_to(
            [args.number(0)?, args.number(1)?],
            [args.number(2)?, args.number(3)?],
            [args.number(4)?, args.number(5)?],
        ));
        build!("quadTo", |path, args| path.quad_to(
            [args.number(0)?, args.number(1)?],
            [args.number(2)?, args.number(3)?],
        ));
        build!("arc", |path, args| path.arc(
            [args.number(0)?, args.number(1)?],
            args.number(2)?,
            args.number(3)?,
            args.number(4)?,
        ));
        build!("rect", |path, args| {
            let (x, y, w, h) = (
                args.number(0)?,
                args.number(1)?,
                args.number(2)?,
                args.number(3)?,
            );
            let rx = args.opt_number(4)?.unwrap_or(0.0);
            let ry = args.opt_number(5)?.unwrap_or(rx);
            path.segments.extend(rounded_rect(x, y, w, h, rx, ry));
        });
        build!("circle", |path, args| {
            let (x, y, r) = (args.number(0)?, args.number(1)?, args.number(2)?);
            ellipse_arc(&mut path.segments, [x, y], [r, r], 0.0, TAU, true);
            path.segments.push(Segment::Close);
        });
        build!("ellipse", |path, args| {
            let (x, y, rx) = (args.number(0)?, args.number(1)?, args.number(2)?);
            let ry = args.opt_number(3)?.unwrap_or(rx);
            ellipse_arc(&mut path.segments, [x, y], [rx, ry], 0.0, TAU, true);
            path.segments.push(Segment::Close);
        });
        build!("polygon", |path, args| {
            let points = args.points(0)?;
            if points.len() < 2 {
                return Err(args.error("a polygon needs at least two points"));
            }
            path.segments.extend(polyline(&points, true));
        });
        build!("svg", |path, args| {
            let segments = parse_svg(&args.string(0)?).map_err(|err| args.error(err))?;
            path.segments.extend(segments);
        });
        build!("close", |path, args| {
            let _ = &args;
            path.close();
        });

        // 묻는 메서드와 새 경로를 돌려주는 메서드.
        methods.add_function("clone", |lua, ud: AnyUserData| {
            let path = ud.borrow::<LuaPath>()?.clone();
            new_path(lua, path)
        });
        methods.add_function("length", |_, ud: AnyUserData| {
            Ok(LuaPath::length(&ud.borrow::<LuaPath>()?.segments))
        });
        methods.add_function("pointAt", |_, (ud, values): (AnyUserData, MultiValue)| {
            let args = Args::method("path:", "pointAt", values);
            let t = args.number(0)?;
            let path = ud.borrow::<LuaPath>()?;
            let (at, angle) = path
                .point_at(t)
                .ok_or_else(|| args.error("the path has no length"))?;
            Ok(numbers_out([at[0], at[1], angle]))
        });
        methods.add_function("trim", |lua, (ud, values): (AnyUserData, MultiValue)| {
            let args = Args::method("path:", "trim", values);
            let (from, to) = (args.number(0)?, args.number(1)?);
            let segments = LuaPath::trim(&ud.borrow::<LuaPath>()?.segments, from, to);
            new_path(lua, LuaPath { segments })
        });
        methods.add_function("bounds", |_, ud: AnyUserData| {
            Ok(match ud.borrow::<LuaPath>()?.bounds() {
                Some(b) => numbers_out(b),
                None => numbers_out([0.0; 4]),
            })
        });
        methods.add_function("points", |lua, ud: AnyUserData| {
            lua.create_sequence_from(ud.borrow::<LuaPath>()?.points())
        });
        methods.add_function(
            "withPoints",
            |lua, (ud, values): (AnyUserData, MultiValue)| {
                let args = Args::method("path:", "withPoints", values);
                let numbers = args.numbers(0)?;
                let path = ud
                    .borrow::<LuaPath>()?
                    .with_points(&numbers)
                    .map_err(|err| args.error(err))?;
                new_path(lua, path)
            },
        );
        methods.add_function("lerp", |lua, (ud, values): (AnyUserData, MultiValue)| {
            let args = Args::method("path:", "lerp", values);
            let other = path_arg(&args, 0)?;
            let t = args.number(1)?;
            let path = ud
                .borrow::<LuaPath>()?
                .lerp(&LuaPath { segments: other }, t)
                .map_err(|err| args.error(err))?;
            new_path(lua, path)
        });
        methods.add_function(
            "translate",
            |lua, (ud, values): (AnyUserData, MultiValue)| {
                let args = Args::method("path:", "translate", values);
                let (dx, dy) = (args.number(0)?, args.number(1)?);
                let path = ud
                    .borrow::<LuaPath>()?
                    .map_points(|[x, y]| [x + dx, y + dy]);
                new_path(lua, path)
            },
        );
        // rotate(angle[, cx, cy]): (cx, cy)(기본 원점)를 중심으로 돌린다.
        methods.add_function("rotate", |lua, (ud, values): (AnyUserData, MultiValue)| {
            let args = Args::method("path:", "rotate", values);
            let angle = args.number(0)?;
            let (cx, cy) = (
                args.opt_number(1)?.unwrap_or(0.0),
                args.opt_number(2)?.unwrap_or(0.0),
            );
            let (sin, cos) = angle.sin_cos();
            let path = ud.borrow::<LuaPath>()?.map_points(|[x, y]| {
                let (dx, dy) = (x - cx, y - cy);
                [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos]
            });
            new_path(lua, path)
        });
        // scale(sx[, sy, cx, cy]): (cx, cy)(기본 원점)를 중심으로 키운다.
        methods.add_function("scale", |lua, (ud, values): (AnyUserData, MultiValue)| {
            let args = Args::method("path:", "scale", values);
            let sx = args.number(0)?;
            let sy = args.opt_number(1)?.unwrap_or(sx);
            let (cx, cy) = (
                args.opt_number(2)?.unwrap_or(0.0),
                args.opt_number(3)?.unwrap_or(0.0),
            );
            let path = ud
                .borrow::<LuaPath>()?
                .map_points(|[x, y]| [cx + (x - cx) * sx, cy + (y - cy) * sy]);
            new_path(lua, path)
        });

        // 없는 메서드를 부르면 쓸 수 있는 목록을 알린다.
        methods.add_meta_function(
            mlua::MetaMethod::Index,
            |_, (_, key): (AnyUserData, Value)| -> mlua::Result<Value> {
                let key = match &key {
                    Value::String(s) => s.to_string_lossy(),
                    other => format!("{other:?}"),
                };
                Err(mlua::Error::runtime(format!(
                    "path:{key} does not exist. Available: path:{}",
                    PATH_METHODS.join(", path:")
                )))
            },
        );
    }
}

/// 그라디언트의 모양(지역 좌표).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum GradientShape {
    Linear { from: Point, to: Point },
    Radial { center: Point, radius: f32 },
}

/// Lua의 그라디언트 값. 색 자리(`g.setColor`)에 넣는다.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LuaGradient {
    pub(super) shape: GradientShape,
    /// (자리 0..1, 색). 자리순.
    pub(super) stops: Vec<(f32, Color)>,
}

impl UserData for LuaGradient {}

/// 색 멈춤 목록: `{{0, color}, {1, color}}` 또는 `{color, color, …}`(고르게 나눈다).
pub(super) fn parse_stops(args: &Args, ix: usize) -> mlua::Result<Vec<(f32, Color)>> {
    let Some(Value::Table(table)) = args.values.get(ix) else {
        return Err(args.error(format!(
            "argument #{} must be a list of color stops, e.g. {{{{0, \"#ffffff\"}}, {{1, \"#000000\"}}}}",
            ix + 1
        )));
    };
    let entries: Vec<Value> = table
        .sequence_values::<Value>()
        .collect::<mlua::Result<_>>()?;
    if entries.is_empty() {
        return Err(args.error("a gradient needs at least one color stop"));
    }
    let color_of = |value: Value| args.with_values(vec![value]).color(0);
    let count = entries.len();
    let mut stops = Vec::with_capacity(count);
    for (n, entry) in entries.into_iter().enumerate() {
        let even = if count == 1 {
            0.0
        } else {
            n as f32 / (count - 1) as f32
        };
        // {pos, color}: 첫 값이 숫자이고 둘째 값이 색(문자열이나 표)이면 자리가 있는 멈춤이다.
        if let Value::Table(stop) = &entry {
            let first: Value = stop.raw_get(1)?;
            let second: Value = stop.raw_get(2)?;
            if matches!(first, Value::Integer(_) | Value::Number(_))
                && matches!(second, Value::String(_) | Value::Table(_))
            {
                let pos = match first {
                    Value::Integer(n) => n as f32,
                    Value::Number(n) => n as f32,
                    _ => unreachable!(),
                };
                stops.push((pos.clamp(0.0, 1.0), color_of(second)?));
                continue;
            }
        }
        stops.push((even, color_of(entry)?));
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(stops)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn parses_svg_commands() {
        let segments = parse_svg("M10 10 h 20 v20 H10 z").unwrap();
        assert_eq!(
            segments,
            vec![
                Segment::Move([10.0, 10.0]),
                Segment::Line([30.0, 10.0]),
                Segment::Line([30.0, 30.0]),
                Segment::Line([10.0, 30.0]),
                Segment::Close,
            ]
        );
        // 이어지는 좌표 쌍은 L, 붙여 쓴 음수와 지수, 상대 곡선.
        let segments = parse_svg("m0,0 10-5 1e1,0c0 10 10 10 10 0").unwrap();
        assert_eq!(segments[1], Segment::Line([10.0, -5.0]));
        assert_eq!(segments[2], Segment::Line([20.0, -5.0]));
        assert_eq!(
            segments[3],
            Segment::Cubic([20.0, 5.0], [30.0, 5.0], [30.0, -5.0])
        );
        assert!(parse_svg("M0 0 X 1").is_err());
        assert!(parse_svg("10 10").is_err());
    }

    #[test]
    fn svg_smooth_and_quadratic_curves() {
        let segments =
            parse_svg("M0 0 C 0 10 10 10 10 0 S 20 -10 20 0 Q 25 10 30 0 T 40 0").unwrap();
        // S의 첫 조절점은 앞 곡선 둘째 조절점의 반사.
        let Segment::Cubic(c1, _, _) = segments[2] else {
            panic!()
        };
        assert!(close(c1, [10.0, -10.0]));
        assert_eq!(segments.len(), 5);
        let Segment::Cubic(_, _, end) = segments[4] else {
            panic!()
        };
        assert!(close(end, [40.0, 0.0]));
    }

    #[test]
    fn svg_arc_reaches_its_end_point() {
        // 반원: (0, 0) → (20, 0), 반지름 10.
        let segments = parse_svg("M0 0 A10 10 0 0 1 20 0").unwrap();
        let Segment::Cubic(_, _, end) = *segments.last().unwrap() else {
            panic!()
        };
        assert!(close(end, [20.0, 0.0]));
        let length = LuaPath::length(&segments);
        assert!(
            (length - std::f32::consts::PI * 10.0).abs() < 0.05,
            "{length}"
        );
        // 붙여 쓴 깃발.
        assert!(parse_svg("M0 0a10 10 0 1120 0").is_ok());
    }

    #[test]
    fn measures_trims_and_dashes() {
        let line = polyline(&[[0.0, 0.0], [100.0, 0.0]], false);
        assert!((LuaPath::length(&line) - 100.0).abs() < 1e-3);
        let half = LuaPath::trim(&line, 0.25, 0.75);
        assert!((LuaPath::length(&half) - 50.0).abs() < 1e-3);
        let path = LuaPath {
            segments: line.clone(),
        };
        let (at, angle) = path.point_at(0.5).unwrap();
        assert!(close(at, [50.0, 0.0]) && angle.abs() < 1e-5);
        // 10 그리고 10 쉬면 다섯 조각, 합 50.
        let dashed = LuaPath::dash(&line, &[10.0, 10.0], 0.0);
        let moves = dashed
            .iter()
            .filter(|s| matches!(s, Segment::Move(_)))
            .count();
        assert_eq!(moves, 5);
        assert!((LuaPath::length(&dashed) - 50.0).abs() < 1e-2);
        // offset 5: 첫 조각은 5만 남는다.
        let shifted = LuaPath::dash(&line, &[10.0, 10.0], 5.0);
        let Segment::Line(first_end) = shifted[1] else {
            panic!()
        };
        assert!(close(first_end, [5.0, 0.0]), "{first_end:?}");
    }

    #[test]
    fn lerps_paths_with_the_same_structure() {
        let a = LuaPath {
            segments: parse_svg("M0 0 C 0 10 10 10 10 0 Z").unwrap(),
        };
        let b = LuaPath {
            segments: parse_svg("M0 0 C 0 20 10 20 10 0 Z").unwrap(),
        };
        let mid = a.lerp(&b, 0.5).unwrap();
        assert_eq!(mid.points()[3], 15.0);
        let other = LuaPath {
            segments: parse_svg("M0 0 L 10 0").unwrap(),
        };
        assert!(a.lerp(&other, 0.5).is_err());
        assert!(a.with_points(&[1.0]).is_err());
    }

    #[test]
    fn line_after_close_starts_at_the_subpath_start() {
        let segments = parse_svg("M0 0 L10 0 L10 10 Z L 0 10").unwrap();
        let lyon = LuaPath::to_lyon(&segments);
        let back = LuaPath::from_lyon(&lyon);
        assert_eq!(back[4], Segment::Move([0.0, 0.0]));
        assert_eq!(back[5], Segment::Line([0.0, 10.0]));
    }
}
