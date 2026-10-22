//! 확장의 캔버스: Lua가 부르는 그리기 함수(`g.*`)를 모양 목록으로 모은다.
//!
//! 함수 이름과 인자는 LÖVE(`love.graphics`)를 따른다. 모델이 그 코드를 많이 봐서 틀리지 않고 쓴다.
//! 변환(`translate`·`rotate`·`scale`)은 모으면서 점에 바로 적용해, 모양 목록에는 캔버스 좌표만 남는다.
//! 원·호도 3차 베지어로 바꿔 두므로 그리는 쪽(에디터, 나중에 PDF·SVG)은 경로와 글자만 알면 된다.

use std::cell::RefCell;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

use mlua::{Lua, MultiValue, Table, Value};

/// RGBA(0..1).
pub type Color = [f32; 4];

/// 캔버스 좌표의 점.
pub type Point = [f32; 2];

/// 한 프레임에 그릴 수 있는 가장 많은 모양. 끝없이 그리는 실수를 막는다.
const MAX_SHAPES: usize = 20_000;

/// 기본 글자 크기(LÖVE 기본 서체와 같다).
const DEFAULT_FONT_SIZE: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Move(Point),
    Line(Point),
    /// (조절점 1, 조절점 2, 끝점).
    Cubic(Point, Point, Point),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Fill {
        path: Vec<Segment>,
        color: Color,
    },
    Stroke {
        path: Vec<Segment>,
        width: f32,
        color: Color,
    },
    /// 글자 한 줄. `origin`은 글자 상자 왼쪽 위, `width`가 있으면 그 폭 안에서 `align`으로 맞춘다.
    Text {
        text: String,
        origin: Point,
        size: f32,
        color: Color,
        align: Align,
        width: Option<f32>,
    },
}

/// 테마 색. `g.theme`으로 Lua에 넘긴다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub foreground: Color,
    pub background: Color,
    pub muted: Color,
    pub accent: Color,
    pub border: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            foreground: [0.1, 0.1, 0.1, 1.0],
            background: [1.0, 1.0, 1.0, 1.0],
            muted: [0.45, 0.45, 0.45, 1.0],
            accent: [0.2, 0.45, 0.9, 1.0],
            border: [0.85, 0.85, 0.85, 1.0],
        }
    }
}

/// 2차원 아핀 변환 `[a, b, c, d, e, f]`: (x, y) → (a·x + c·y + e, b·x + d·y + f).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Transform([f32; 6]);

impl Transform {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn apply(&self, [x, y]: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        [a * x + c * y + e, b * x + d * y + f]
    }

    /// `self` 다음에 `local`(지역 좌표의 변환)을 적용한다.
    fn then(&self, local: [f32; 6]) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [la, lb, lc, ld, le, lf] = local;
        Self([
            a * la + c * lb,
            b * la + d * lb,
            a * lc + c * ld,
            b * lc + d * ld,
            a * le + c * lf + e,
            b * le + d * lf + f,
        ])
    }

    /// 길이가 늘어나는 비율(선 굵기·글자 크기용).
    fn scale(&self) -> f32 {
        let [a, b, c, d, ..] = self.0;
        (a * d - b * c).abs().sqrt()
    }
}

/// 그리기 상태와 모은 모양.
#[derive(Debug)]
pub(super) struct Recorder {
    pub(super) width: f32,
    pub(super) height: f32,
    color: Color,
    line_width: f32,
    font_size: f32,
    transform: Transform,
    stack: Vec<Transform>,
    pub(super) shapes: Vec<Shape>,
}

impl Recorder {
    pub(super) fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            color: [1.0, 1.0, 1.0, 1.0],
            line_width: 1.0,
            font_size: DEFAULT_FONT_SIZE,
            transform: Transform::IDENTITY,
            stack: Vec::new(),
            shapes: Vec::new(),
        }
    }

    /// 새 프레임: 상태를 처음으로 되돌린다. 기본 색은 테마 글자색이다.
    pub(super) fn reset(&mut self, width: f32, height: f32, color: Color) {
        *self = Self::new(width, height);
        self.color = color;
    }

    fn push(&mut self, shape: Shape) -> mlua::Result<()> {
        if self.shapes.len() >= MAX_SHAPES {
            return Err(mlua::Error::runtime(format!(
                "too many shapes in one frame (limit {MAX_SHAPES})"
            )));
        }
        self.shapes.push(shape);
        Ok(())
    }

    fn path(&self, local: &[Segment]) -> Vec<Segment> {
        local
            .iter()
            .map(|segment| match *segment {
                Segment::Move(p) => Segment::Move(self.transform.apply(p)),
                Segment::Line(p) => Segment::Line(self.transform.apply(p)),
                Segment::Cubic(a, b, p) => Segment::Cubic(
                    self.transform.apply(a),
                    self.transform.apply(b),
                    self.transform.apply(p),
                ),
                Segment::Close => Segment::Close,
            })
            .collect()
    }

    fn draw(&mut self, mode: Mode, local: &[Segment]) -> mlua::Result<()> {
        let path = self.path(local);
        let color = self.color;
        match mode {
            Mode::Fill => self.push(Shape::Fill { path, color }),
            Mode::Line => {
                let width = self.line_width * self.transform.scale();
                self.push(Shape::Stroke { path, width, color })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Fill,
    Line,
}

/// 원·타원 호를 3차 베지어로 잇는다(90°씩 나눈다). `move_to`면 시작점으로 옮기고 시작한다.
fn ellipse_arc(
    out: &mut Vec<Segment>,
    [cx, cy]: Point,
    [rx, ry]: [f32; 2],
    start: f32,
    end: f32,
    move_to: bool,
) {
    let sweep = end - start;
    let parts = ((sweep.abs() / FRAC_PI_2).ceil() as usize).clamp(1, 64);
    let step = sweep / parts as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let at = |angle: f32| [cx + rx * angle.cos(), cy + ry * angle.sin()];
    if move_to {
        out.push(Segment::Move(at(start)));
    }
    for ix in 0..parts {
        let a = start + step * ix as f32;
        let b = a + step;
        let (p0, p3) = (at(a), at(b));
        let c1 = [p0[0] - k * rx * a.sin(), p0[1] + k * ry * a.cos()];
        let c2 = [p3[0] + k * rx * b.sin(), p3[1] - k * ry * b.cos()];
        out.push(Segment::Cubic(c1, c2, p3));
    }
}

fn polyline(points: &[Point], closed: bool) -> Vec<Segment> {
    let mut path = Vec::with_capacity(points.len() + 1);
    for (ix, &point) in points.iter().enumerate() {
        path.push(if ix == 0 {
            Segment::Move(point)
        } else {
            Segment::Line(point)
        });
    }
    if closed {
        path.push(Segment::Close);
    }
    path
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Vec<Segment> {
    let rx = rx.min(w / 2.0).max(0.0);
    let ry = ry.min(h / 2.0).max(0.0);
    if rx == 0.0 || ry == 0.0 {
        return polyline(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]], true);
    }
    let mut path = vec![Segment::Move([x + rx, y]), Segment::Line([x + w - rx, y])];
    ellipse_arc(
        &mut path,
        [x + w - rx, y + ry],
        [rx, ry],
        -FRAC_PI_2,
        0.0,
        false,
    );
    path.push(Segment::Line([x + w, y + h - ry]));
    ellipse_arc(
        &mut path,
        [x + w - rx, y + h - ry],
        [rx, ry],
        0.0,
        FRAC_PI_2,
        false,
    );
    path.push(Segment::Line([x + rx, y + h]));
    ellipse_arc(
        &mut path,
        [x + rx, y + h - ry],
        [rx, ry],
        FRAC_PI_2,
        FRAC_PI_2 * 2.0,
        false,
    );
    path.push(Segment::Line([x, y + ry]));
    ellipse_arc(
        &mut path,
        [x + rx, y + ry],
        [rx, ry],
        FRAC_PI_2 * 2.0,
        FRAC_PI_2 * 3.0,
        false,
    );
    path.push(Segment::Close);
    path
}

/// Lua 인자 읽기. 모델이 잘못 불렀을 때 고칠 수 있게 함수 이름과 자리를 알린다.
struct Args {
    name: &'static str,
    values: Vec<Value>,
}

impl Args {
    /// `g:rectangle(...)`처럼 콜론으로 불러 앞에 `g`가 붙었으면 뗀다.
    fn new(name: &'static str, values: MultiValue) -> Self {
        let mut values: Vec<Value> = values.into_iter().collect();
        if let Some(Value::Table(table)) = values.first()
            && table.raw_get::<bool>(CANVAS_MARK).unwrap_or(false)
        {
            values.remove(0);
        }
        Self { name, values }
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn error(&self, message: impl std::fmt::Display) -> mlua::Error {
        mlua::Error::runtime(format!("g.{}: {message}", self.name))
    }

    fn number(&self, ix: usize) -> mlua::Result<f32> {
        self.opt_number(ix)?
            .ok_or_else(|| self.error(format!("argument #{} must be a number", ix + 1)))
    }

    fn opt_number(&self, ix: usize) -> mlua::Result<Option<f32>> {
        match self.values.get(ix) {
            None | Some(Value::Nil) => Ok(None),
            Some(Value::Integer(n)) => Ok(Some(*n as f32)),
            Some(Value::Number(n)) => Ok(Some(*n as f32)),
            Some(other) => Err(self.error(format!(
                "argument #{} must be a number, got {}",
                ix + 1,
                other.type_name()
            ))),
        }
    }

    fn string(&self, ix: usize) -> mlua::Result<String> {
        match self.values.get(ix) {
            Some(Value::String(s)) => Ok(s.to_string_lossy()),
            Some(Value::Integer(n)) => Ok(n.to_string()),
            Some(Value::Number(n)) => Ok(n.to_string()),
            Some(Value::Boolean(b)) => Ok(b.to_string()),
            _ => Err(self.error(format!("argument #{} must be a string", ix + 1))),
        }
    }

    fn mode(&self, ix: usize) -> mlua::Result<Mode> {
        match self.string(ix)?.as_str() {
            "fill" => Ok(Mode::Fill),
            "line" => Ok(Mode::Line),
            other => Err(self.error(format!(
                "draw mode must be \"fill\" or \"line\", got \"{other}\""
            ))),
        }
    }

    /// `ix`부터의 좌표: 숫자 여럿 또는 숫자 표 하나.
    fn points(&self, ix: usize) -> mlua::Result<Vec<Point>> {
        let numbers: Vec<f32> = match self.values.get(ix) {
            Some(Value::Table(table)) => table
                .sequence_values::<f32>()
                .collect::<mlua::Result<_>>()
                .map_err(|_| self.error("the point table must hold only numbers"))?,
            _ => (ix..self.len())
                .map(|ix| self.number(ix))
                .collect::<mlua::Result<_>>()?,
        };
        if !numbers.len().is_multiple_of(2) {
            return Err(self.error("coordinates must come in x, y pairs"));
        }
        Ok(numbers.chunks(2).map(|pair| [pair[0], pair[1]]).collect())
    }

    /// 색: `r, g, b[, a]`(0..1), `{r, g, b[, a]}`, `"#rrggbb[aa]"`. 1보다 큰 값이 있으면 0..255로 본다.
    fn color(&self, ix: usize) -> mlua::Result<Color> {
        let components: Vec<f32> = match self.values.get(ix) {
            Some(Value::String(s)) => {
                return parse_hex(&s.to_string_lossy())
                    .ok_or_else(|| self.error("a color string must look like \"#rrggbb\""));
            }
            Some(Value::Table(table)) => table
                .sequence_values::<f32>()
                .collect::<mlua::Result<_>>()
                .map_err(|_| self.error("a color table must hold numbers"))?,
            _ => (ix..self.len().min(ix + 4))
                .map(|ix| self.number(ix))
                .collect::<mlua::Result<_>>()?,
        };
        if components.len() < 3 {
            return Err(self.error("a color needs r, g, b"));
        }
        let scale = if components.iter().take(3).any(|c| *c > 1.0) {
            255.0
        } else {
            1.0
        };
        let alpha = components.get(3).map_or(1.0, |a| a / scale);
        Ok([
            (components[0] / scale).clamp(0.0, 1.0),
            (components[1] / scale).clamp(0.0, 1.0),
            (components[2] / scale).clamp(0.0, 1.0),
            alpha.clamp(0.0, 1.0),
        ])
    }
}

fn parse_hex(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    if !matches!(hex.len(), 6 | 8) || !hex.is_ascii() {
        return None;
    }
    let byte = |ix: usize| u8::from_str_radix(&hex[ix..ix + 2], 16).ok();
    let alpha = if hex.len() == 8 { byte(6)? } else { 255 };
    Some([byte(0)?, byte(2)?, byte(4)?, alpha].map(|c| f32::from(c) / 255.0))
}

/// `g` 표에 붙여 콜론 호출을 알아보는 표시.
const CANVAS_MARK: &str = "__canvas";

/// 그리기 함수 이름(오류 안내와 스킬 문서에 쓴다).
pub(super) const FUNCTIONS: &[&str] = &[
    "setColor",
    "getColor",
    "setLineWidth",
    "getLineWidth",
    "setFont",
    "newFont",
    "getWidth",
    "getHeight",
    "getDimensions",
    "clear",
    "rectangle",
    "circle",
    "ellipse",
    "arc",
    "line",
    "polygon",
    "print",
    "printf",
    "push",
    "pop",
    "origin",
    "translate",
    "rotate",
    "scale",
];

fn color_table(lua: &Lua, color: Color) -> mlua::Result<Table> {
    lua.create_sequence_from(color)
}

/// `g` 표를 만든다. 함수들은 `recorder`에 모양을 모은다.
pub(super) fn create(lua: &Lua, recorder: Rc<RefCell<Recorder>>) -> mlua::Result<Table> {
    let g = lua.create_table()?;
    g.raw_set(CANVAS_MARK, true)?;

    macro_rules! function {
        ($name:literal, |$rec:ident, $args:ident| $body:expr) => {{
            let recorder = recorder.clone();
            g.raw_set(
                $name,
                lua.create_function(move |lua, values: MultiValue| {
                    let _ = lua;
                    let $args = Args::new($name, values);
                    #[allow(unused_mut)]
                    let mut $rec = recorder.borrow_mut();
                    $body
                })?,
            )?;
        }};
    }

    function!("setColor", |rec, args| {
        rec.color = args.color(0)?;
        Ok(MultiValue::new())
    });
    function!("getColor", |rec, args| {
        let _ = args;
        Ok(rec
            .color
            .iter()
            .map(|c| Value::Number(f64::from(*c)))
            .collect::<MultiValue>())
    });
    function!("setLineWidth", |rec, args| {
        rec.line_width = args.number(0)?.max(0.0);
        Ok(MultiValue::new())
    });
    function!("getLineWidth", |rec, args| {
        let _ = args;
        Ok(MultiValue::from_vec(vec![Value::Number(f64::from(
            rec.line_width,
        ))]))
    });
    // LÖVE는 서체 객체를 넘기지만, 여기서는 크기(숫자)가 서체다. `g.setFont(g.newFont(20))`도 된다.
    function!("newFont", |rec, args| {
        let _ = rec;
        let size = args
            .opt_number(args.len().saturating_sub(1))?
            .unwrap_or(DEFAULT_FONT_SIZE);
        Ok(MultiValue::from_vec(vec![Value::Number(f64::from(size))]))
    });
    function!("setFont", |rec, args| {
        rec.font_size = args.number(0)?.max(1.0);
        Ok(MultiValue::new())
    });
    function!("getWidth", |rec, args| {
        let _ = args;
        Ok(MultiValue::from_vec(vec![Value::Number(f64::from(
            rec.width,
        ))]))
    });
    function!("getHeight", |rec, args| {
        let _ = args;
        Ok(MultiValue::from_vec(vec![Value::Number(f64::from(
            rec.height,
        ))]))
    });
    function!("getDimensions", |rec, args| {
        let _ = args;
        Ok(MultiValue::from_vec(vec![
            Value::Number(f64::from(rec.width)),
            Value::Number(f64::from(rec.height)),
        ]))
    });
    // 캔버스 전체를 칠한다(변환과 상관없다). 인자가 없으면 지금 색이다.
    function!("clear", |rec, args| {
        let color = if args.len() == 0 {
            rec.color
        } else {
            args.color(0)?
        };
        let (w, h) = (rec.width, rec.height);
        let path = polyline(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], true);
        rec.push(Shape::Fill { path, color })?;
        Ok(MultiValue::new())
    });
    function!("rectangle", |rec, args| {
        let mode = args.mode(0)?;
        let (x, y, w, h) = (
            args.number(1)?,
            args.number(2)?,
            args.number(3)?,
            args.number(4)?,
        );
        let rx = args.opt_number(5)?.unwrap_or(0.0);
        let ry = args.opt_number(6)?.unwrap_or(rx);
        rec.draw(mode, &rounded_rect(x, y, w, h, rx, ry))?;
        Ok(MultiValue::new())
    });
    function!("circle", |rec, args| {
        let mode = args.mode(0)?;
        let (x, y, r) = (args.number(1)?, args.number(2)?, args.number(3)?);
        let mut path = Vec::new();
        ellipse_arc(&mut path, [x, y], [r, r], 0.0, TAU, true);
        path.push(Segment::Close);
        rec.draw(mode, &path)?;
        Ok(MultiValue::new())
    });
    function!("ellipse", |rec, args| {
        let mode = args.mode(0)?;
        let (x, y, rx) = (args.number(1)?, args.number(2)?, args.number(3)?);
        let ry = args.opt_number(4)?.unwrap_or(rx);
        let mut path = Vec::new();
        ellipse_arc(&mut path, [x, y], [rx, ry], 0.0, TAU, true);
        path.push(Segment::Close);
        rec.draw(mode, &path)?;
        Ok(MultiValue::new())
    });
    // arc(mode, [arctype,] x, y, radius, angle1, angle2). arctype: "pie"(기본), "open", "closed".
    function!("arc", |rec, args| {
        let mode = args.mode(0)?;
        let (kind, from) = match args.values.get(1) {
            Some(Value::String(_)) => (args.string(1)?, 2),
            _ => ("pie".to_string(), 1),
        };
        let (x, y, r) = (
            args.number(from)?,
            args.number(from + 1)?,
            args.number(from + 2)?,
        );
        let (a1, a2) = (args.number(from + 3)?, args.number(from + 4)?);
        let mut path = Vec::new();
        match kind.as_str() {
            "pie" => {
                path.push(Segment::Move([x, y]));
                path.push(Segment::Line([x + r * a1.cos(), y + r * a1.sin()]));
                ellipse_arc(&mut path, [x, y], [r, r], a1, a2, false);
                path.push(Segment::Close);
            }
            "open" | "closed" => {
                ellipse_arc(&mut path, [x, y], [r, r], a1, a2, true);
                if kind == "closed" || mode == Mode::Fill {
                    path.push(Segment::Close);
                }
            }
            other => {
                return Err(args.error(format!(
                    "arc type must be \"pie\", \"open\" or \"closed\", got \"{other}\""
                )));
            }
        }
        rec.draw(mode, &path)?;
        Ok(MultiValue::new())
    });
    function!("line", |rec, args| {
        let points = args.points(0)?;
        if points.len() < 2 {
            return Err(args.error("a line needs at least two points"));
        }
        rec.draw(Mode::Line, &polyline(&points, false))?;
        Ok(MultiValue::new())
    });
    function!("polygon", |rec, args| {
        let mode = args.mode(0)?;
        let points = args.points(1)?;
        if points.len() < 3 {
            return Err(args.error("a polygon needs at least three points"));
        }
        rec.draw(mode, &polyline(&points, true))?;
        Ok(MultiValue::new())
    });
    // print(text, x, y[, r, sx, sy]). 글자는 돌리지 않는다(위치와 크기만 변환을 따른다).
    function!("print", |rec, args| {
        let text = args.string(0)?;
        let x = args.opt_number(1)?.unwrap_or(0.0);
        let y = args.opt_number(2)?.unwrap_or(0.0);
        let sx = args.opt_number(4)?.unwrap_or(1.0);
        print(&mut rec, &text, [x, y], sx, None, Align::Left)
    });
    // printf(text, x, y, limit[, align]). align: "left"(기본), "center", "right". 줄바꿈은 `\n`만 한다.
    function!("printf", |rec, args| {
        let text = args.string(0)?;
        let (x, y, limit) = (args.number(1)?, args.number(2)?, args.number(3)?);
        let align = match args.values.get(4) {
            None | Some(Value::Nil) => Align::Left,
            _ => match args.string(4)?.as_str() {
                "left" | "justify" => Align::Left,
                "center" => Align::Center,
                "right" => Align::Right,
                other => {
                    return Err(args.error(format!(
                        "align must be \"left\", \"center\" or \"right\", got \"{other}\""
                    )));
                }
            },
        };
        print(&mut rec, &text, [x, y], 1.0, Some(limit), align)
    });
    function!("push", |rec, args| {
        let _ = args;
        if rec.stack.len() >= 64 {
            return Err(args.error("too many pushes without pop"));
        }
        let transform = rec.transform;
        rec.stack.push(transform);
        Ok(MultiValue::new())
    });
    function!("pop", |rec, args| {
        rec.transform = rec
            .stack
            .pop()
            .ok_or_else(|| args.error("pop without a matching push"))?;
        Ok(MultiValue::new())
    });
    function!("origin", |rec, args| {
        let _ = args;
        rec.transform = Transform::IDENTITY;
        Ok(MultiValue::new())
    });
    function!("translate", |rec, args| {
        let (dx, dy) = (args.number(0)?, args.number(1)?);
        rec.transform = rec.transform.then([1.0, 0.0, 0.0, 1.0, dx, dy]);
        Ok(MultiValue::new())
    });
    function!("rotate", |rec, args| {
        let angle = args.number(0)?;
        let (sin, cos) = angle.sin_cos();
        rec.transform = rec.transform.then([cos, sin, -sin, cos, 0.0, 0.0]);
        Ok(MultiValue::new())
    });
    function!("scale", |rec, args| {
        let sx = args.number(0)?;
        let sy = args.opt_number(1)?.unwrap_or(sx);
        rec.transform = rec.transform.then([sx, 0.0, 0.0, sy, 0.0, 0.0]);
        Ok(MultiValue::new())
    });

    // 없는 함수를 부르면 쓸 수 있는 목록을 알린다.
    let meta = lua.create_table()?;
    meta.raw_set(
        "__index",
        lua.create_function(|_, (_, key): (Value, Value)| -> mlua::Result<Value> {
            let key = match &key {
                Value::String(s) => s.to_string_lossy(),
                other => format!("{other:?}"),
            };
            Err(mlua::Error::runtime(format!(
                "g.{key} does not exist. Available: g.{}, g.theme",
                FUNCTIONS.join(", g.")
            )))
        })?,
    )?;
    g.set_metatable(Some(meta))?;
    Ok(g)
}

fn print(
    rec: &mut Recorder,
    text: &str,
    at: Point,
    scale: f32,
    width: Option<f32>,
    align: Align,
) -> mlua::Result<MultiValue> {
    let factor = rec.transform.scale() * scale;
    let size = rec.font_size * factor;
    let color = rec.color;
    for (ix, line) in text.split('\n').enumerate() {
        let origin = rec.transform.apply([
            at[0],
            at[1] + ix as f32 * rec.font_size * scale * LINE_HEIGHT,
        ]);
        rec.push(Shape::Text {
            text: line.to_string(),
            origin,
            size,
            color,
            align,
            width: width.map(|width| width * factor),
        })?;
    }
    Ok(MultiValue::new())
}

/// 글자 줄 높이(글자 크기 대비).
pub const LINE_HEIGHT: f32 = 1.25;

/// `g.theme` 표.
pub(super) fn theme_table(lua: &Lua, theme: &Theme) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.raw_set("foreground", color_table(lua, theme.foreground)?)?;
    table.raw_set("background", color_table(lua, theme.background)?)?;
    table.raw_set("muted", color_table(lua, theme.muted)?)?;
    table.raw_set("accent", color_table(lua, theme.accent)?)?;
    table.raw_set("border", color_table(lua, theme.border)?)?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_composes_like_love() {
        // translate 후 rotate: 지역 (1, 0)이 90° 돌아 (0, 1)이 되고, 그다음 (10, 0)만큼 옮겨진다.
        let t = Transform::IDENTITY
            .then([1.0, 0.0, 0.0, 1.0, 10.0, 0.0])
            .then([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
        let [x, y] = t.apply([1.0, 0.0]);
        assert!(
            (x - 10.0).abs() < 1e-5 && (y - 1.0).abs() < 1e-5,
            "{x}, {y}"
        );
        let s = Transform::IDENTITY.then([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        assert!((s.scale() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn circle_arc_ends_where_it_started() {
        let mut path = Vec::new();
        ellipse_arc(&mut path, [0.0, 0.0], [10.0, 10.0], 0.0, TAU, true);
        assert_eq!(path.len(), 5);
        let Segment::Cubic(_, _, end) = path[4] else {
            panic!()
        };
        assert!((end[0] - 10.0).abs() < 1e-3 && end[1].abs() < 1e-3);
    }

    #[test]
    fn parses_hex_colors() {
        assert_eq!(parse_hex("#ff0000"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(
            parse_hex("#00000080").map(|c| (c[3] * 255.0).round()),
            Some(128.0)
        );
        assert_eq!(parse_hex("red"), None);
    }
}
