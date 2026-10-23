//! 확장의 3차원 장면: `g.newScene()`이 돌려주는 `scene`.
//!
//! GPU를 쓰지 않는다. 면과 선을 모아 두었다가 `scene:draw()`에서 카메라로 투영하고, 먼 것부터
//! 2차원 모양(채운 다각형·선)으로 캔버스에 넣는다(화가 알고리즘). 그래서 모양 목록은 2차원 그대로이고,
//! 그리는 쪽(에디터, 나중에 PDF·SVG)은 3차원을 몰라도 된다.
//!
//! 좌표계는 오른손, y가 위다. 면은 바깥에서 볼 때 반시계 방향으로 점을 잇는다. 빛은 면마다 한 번
//! 계산한다(평면 음영: 주변광 + 램버트).

use std::cell::RefCell;
use std::f32::consts::{PI, TAU};
use std::rc::Rc;

use mlua::{Lua, MultiValue, Table, Value};

use super::canvas::{Args, Color, Mode, Point, Recorder, STILL_SHAPE_LIMIT, polyline};

type Vec3 = [f32; 3];

/// `scene` 표에 붙여 콜론 호출을 알아보는 표시.
const SCENE_MARK: &str = "__scene";

/// 장면 함수 이름(오류 안내와 스킬 문서에 쓴다).
pub(super) const FUNCTIONS: &[&str] = &[
    "camera",
    "ortho",
    "light",
    "setColor",
    "push",
    "pop",
    "origin",
    "translate",
    "rotate",
    "rotateX",
    "rotateY",
    "rotateZ",
    "scale",
    "box",
    "sphere",
    "cylinder",
    "cone",
    "torus",
    "plane",
    "polygon",
    "mesh",
    "line",
    "project",
    "draw",
];

/// 카메라에서 이보다 가까운 것은 잘라 낸다(원근 투영).
const NEAR: f32 = 0.01;

/// 채운 면을 바깥으로 넓히는 폭(캔버스 단위). 이웃한 면 사이에 안티에일리어싱 틈이 보이지 않게 한다.
const SEAM: f32 = 0.35;

/// 3×4 아핀 변환(행 우선): 행마다 `[m0, m1, m2, t]`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mat([f32; 12]);

impl Mat {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);

    fn apply(&self, [x, y, z]: Vec3) -> Vec3 {
        let m = &self.0;
        [
            m[0] * x + m[1] * y + m[2] * z + m[3],
            m[4] * x + m[5] * y + m[6] * z + m[7],
            m[8] * x + m[9] * y + m[10] * z + m[11],
        ]
    }

    /// `self` 다음에 `local`(지역 좌표의 변환)을 적용한다.
    fn then(&self, local: Mat) -> Self {
        let (a, b) = (&self.0, &local.0);
        let mut out = [0.0; 12];
        for row in 0..3 {
            for col in 0..4 {
                let mut value = (0..3).map(|k| a[row * 4 + k] * b[k * 4 + col]).sum::<f32>();
                if col == 3 {
                    value += a[row * 4 + 3];
                }
                out[row * 4 + col] = value;
            }
        }
        Self(out)
    }

    fn determinant(&self) -> f32 {
        let m = &self.0;
        m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8])
            + m[2] * (m[4] * m[9] - m[5] * m[8])
    }

    fn translation([x, y, z]: Vec3) -> Self {
        Self([1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z])
    }

    fn scaling([x, y, z]: Vec3) -> Self {
        Self([x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0])
    }

    /// 축 `axis`를 중심으로 `angle`만큼(오른손 법칙) 돌린다.
    fn rotation(axis: Vec3, angle: f32) -> Option<Self> {
        let [x, y, z] = normalize(axis)?;
        let (s, c) = angle.sin_cos();
        let t = 1.0 - c;
        Some(Self([
            t * x * x + c,
            t * x * y - s * z,
            t * x * z + s * y,
            0.0,
            t * x * y + s * z,
            t * y * y + c,
            t * y * z - s * x,
            0.0,
            t * x * z - s * y,
            t * y * z + s * x,
            t * z * z + c,
            0.0,
        ]))
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: Vec3) -> Option<Vec3> {
    let length = dot(v, v).sqrt();
    (length > 1e-12).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

/// 다각형의 법선(Newell). 점을 반시계로 잇는 쪽을 향한다. 길이는 넓이의 두 배다.
fn newell(points: &[Vec3]) -> Vec3 {
    let mut n = [0.0; 3];
    for (ix, a) in points.iter().enumerate() {
        let b = points[(ix + 1) % points.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

fn centroid(points: &[Vec3]) -> Vec3 {
    let n = points.len() as f32;
    let sum = points
        .iter()
        .fold([0.0; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]]);
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

/// 면이 `outward` 쪽을 보도록 점 순서를 맞춘다.
fn facing(mut points: Vec<Vec3>, outward: Vec3) -> Vec<Vec3> {
    if dot(newell(&points), outward) < 0.0 {
        points.reverse();
    }
    points
}

#[derive(Clone, Debug)]
enum Item {
    Face {
        points: Vec<Vec3>,
        mode: Mode,
        color: Color,
        two_sided: bool,
    },
    Line {
        points: Vec<Vec3>,
        color: Color,
    },
}

#[derive(Clone, Copy, Debug)]
enum Projection {
    /// 세로 시야각(라디안).
    Perspective(f32),
    /// 보이는 세로 길이(장면 단위).
    Orthographic(f32),
}

#[derive(Debug)]
struct Scene {
    viewport: [f32; 4],
    eye: Vec3,
    target: Vec3,
    projection: Projection,
    /// 빛이 오는 쪽(정규화).
    light: Vec3,
    ambient: f32,
    color: Color,
    transform: Mat,
    stack: Vec<Mat>,
    items: Vec<Item>,
}

/// 카메라 공간: 오른쪽 `right`, 위 `up`, 앞(보는 쪽) `forward`.
struct Camera {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    projection: Projection,
    focal: f32,
    center: Point,
}

impl Camera {
    fn new(scene: &Scene) -> Self {
        let forward = normalize(sub(scene.target, scene.eye)).unwrap_or([0.0, 0.0, -1.0]);
        let right = normalize(cross(forward, [0.0, 1.0, 0.0]))
            .or_else(|| normalize(cross(forward, [0.0, 0.0, -1.0])))
            .unwrap_or([1.0, 0.0, 0.0]);
        let up = cross(right, forward);
        let [x, y, w, h] = scene.viewport;
        let focal = match scene.projection {
            Projection::Perspective(fov) => h / 2.0 / (fov / 2.0).tan(),
            Projection::Orthographic(height) => h / height,
        };
        Self {
            eye: scene.eye,
            right,
            up,
            forward,
            projection: scene.projection,
            focal,
            center: [x + w / 2.0, y + h / 2.0],
        }
    }

    /// 카메라 공간 `(x, y, 깊이)`.
    fn view(&self, p: Vec3) -> Vec3 {
        let d = sub(p, self.eye);
        [dot(d, self.right), dot(d, self.up), dot(d, self.forward)]
    }

    fn near(&self) -> f32 {
        match self.projection {
            Projection::Perspective(_) => NEAR,
            Projection::Orthographic(_) => f32::NEG_INFINITY,
        }
    }

    fn screen(&self, [x, y, z]: Vec3) -> Point {
        let k = match self.projection {
            Projection::Perspective(_) => self.focal / z,
            Projection::Orthographic(_) => self.focal,
        };
        [self.center[0] + x * k, self.center[1] - y * k]
    }

    /// 면이 카메라 쪽을 보는가(법선 `normal`, 면 위의 점 `at`).
    fn faces(&self, normal: Vec3, at: Vec3) -> bool {
        match self.projection {
            Projection::Perspective(_) => dot(normal, sub(self.eye, at)) > 0.0,
            Projection::Orthographic(_) => dot(normal, self.forward) < 0.0,
        }
    }
}

/// 카메라 공간 다각형을 깊이 `near` 앞쪽만 남긴다(Sutherland–Hodgman).
fn clip_near(points: &[Vec3], near: f32) -> Vec<Vec3> {
    if points.iter().all(|p| p[2] >= near) {
        return points.to_vec();
    }
    let mut out = Vec::new();
    for (ix, &a) in points.iter().enumerate() {
        let b = points[(ix + 1) % points.len()];
        if a[2] >= near {
            out.push(a);
        }
        if (a[2] >= near) != (b[2] >= near) {
            let t = (near - a[2]) / (b[2] - a[2]);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, near]);
        }
    }
    out
}

/// 2차원 다각형을 사각형 `[x, y, w, h]` 안만 남긴다(Sutherland–Hodgman).
fn clip_rect(points: Vec<Point>, [x, y, w, h]: [f32; 4]) -> Vec<Point> {
    let edges: [(usize, f32, bool); 4] = [
        (0, x, true),
        (0, x + w, false),
        (1, y, true),
        (1, y + h, false),
    ];
    let mut points = points;
    for (axis, bound, keep_above) in edges {
        let inside = |p: &Point| {
            if keep_above {
                p[axis] >= bound
            } else {
                p[axis] <= bound
            }
        };
        let mut out = Vec::with_capacity(points.len() + 2);
        for (ix, a) in points.iter().enumerate() {
            let b = points[(ix + 1) % points.len()];
            if inside(a) {
                out.push(*a);
            }
            if inside(a) != inside(&b) {
                let t = (bound - a[axis]) / (b[axis] - a[axis]);
                out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
        points = out;
        if points.is_empty() {
            break;
        }
    }
    points
}

/// 선분을 사각형 안만 남긴다(Liang–Barsky).
fn clip_segment(a: Point, b: Point, [x, y, w, h]: [f32; 4]) -> Option<(Point, Point)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-d[0], a[0] - x),
        (d[0], x + w - a[0]),
        (-d[1], a[1] - y),
        (d[1], y + h - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    (t0 <= t1).then(|| {
        let at = |t: f32| [a[0] + d[0] * t, a[1] + d[1] * t];
        (at(t0), at(t1))
    })
}

/// 투영한 모양 하나(깊이로 줄 세운다).
struct Projected {
    depth: f32,
    mode: Mode,
    color: Color,
    path: Vec<super::canvas::Segment>,
}

impl Scene {
    fn new(viewport: [f32; 4], color: Color) -> Self {
        Self {
            viewport,
            eye: [3.0, 2.0, 4.0],
            target: [0.0, 0.0, 0.0],
            projection: Projection::Perspective(45f32.to_radians()),
            light: normalize([0.4, 1.0, 0.7]).unwrap(),
            ambient: 0.35,
            color,
            transform: Mat::IDENTITY,
            stack: Vec::new(),
            items: Vec::new(),
        }
    }

    fn add(&mut self, args: &Args, item: Item) -> mlua::Result<()> {
        if self.items.len() >= STILL_SHAPE_LIMIT {
            return Err(args.error(format!(
                "too many faces in one scene (limit {STILL_SHAPE_LIMIT}); use fewer segments"
            )));
        }
        self.items.push(item);
        Ok(())
    }

    /// 지역 좌표의 면을 더한다. 거울 변환(행렬식 < 0)이면 순서를 뒤집어 바깥을 지킨다.
    fn face(
        &mut self,
        args: &Args,
        mode: Mode,
        local: &[Vec3],
        two_sided: bool,
    ) -> mlua::Result<()> {
        let mut points: Vec<Vec3> = local.iter().map(|p| self.transform.apply(*p)).collect();
        if self.transform.determinant() < 0.0 {
            points.reverse();
        }
        let color = self.color;
        self.add(
            args,
            Item::Face {
                points,
                mode,
                color,
                two_sided,
            },
        )
    }

    fn faces(&mut self, args: &Args, mode: Mode, faces: Vec<Vec<Vec3>>) -> mlua::Result<()> {
        for face in faces {
            self.face(args, mode, &face, false)?;
        }
        Ok(())
    }

    fn project(&self, camera: &Camera) -> Vec<Projected> {
        let near = camera.near();
        let mut out = Vec::new();
        for item in &self.items {
            match item {
                Item::Face {
                    points,
                    mode,
                    color,
                    two_sided,
                } => {
                    let Some(mut normal) = normalize(newell(points)) else {
                        continue;
                    };
                    if *mode == Mode::Fill && !camera.faces(normal, points[0]) {
                        if !two_sided {
                            continue;
                        }
                        normal = normal.map(|c| -c);
                    }
                    let view: Vec<Vec3> = points.iter().map(|p| camera.view(*p)).collect();
                    let view = clip_near(&view, near);
                    if view.len() < 3 {
                        continue;
                    }
                    let depth = view.iter().map(|p| p[2]).sum::<f32>() / view.len() as f32;
                    let mut screen: Vec<Point> = view.iter().map(|p| camera.screen(*p)).collect();
                    let color = match mode {
                        Mode::Fill => {
                            let light = self.ambient
                                + (1.0 - self.ambient) * dot(normal, self.light).max(0.0);
                            if color[3] >= 1.0 {
                                let n = screen.len() as f32;
                                let c = screen
                                    .iter()
                                    .fold([0.0, 0.0], |s, p| [s[0] + p[0] / n, s[1] + p[1] / n]);
                                for p in &mut screen {
                                    let d = [p[0] - c[0], p[1] - c[1]];
                                    let length = (d[0] * d[0] + d[1] * d[1]).sqrt();
                                    if length > 1e-3 {
                                        p[0] += d[0] / length * SEAM;
                                        p[1] += d[1] / length * SEAM;
                                    }
                                }
                            }
                            [
                                color[0] * light,
                                color[1] * light,
                                color[2] * light,
                                color[3],
                            ]
                        }
                        Mode::Line => *color,
                    };
                    let screen = clip_rect(screen, self.viewport);
                    if screen.len() < 3 {
                        continue;
                    }
                    out.push(Projected {
                        depth,
                        mode: *mode,
                        color,
                        path: polyline(&screen, true),
                    });
                }
                Item::Line { points, color } => {
                    for pair in points.windows(2) {
                        let view = clip_near(&[camera.view(pair[0]), camera.view(pair[1])], near);
                        // 두 점짜리 "다각형"을 자르면 [앞, 잘린 점] 또는 [잘린 점, 앞]이 남는다.
                        let [a, b] = match view.as_slice() {
                            [a, b] | [a, b, _] => [*a, *b],
                            _ => continue,
                        };
                        let Some((sa, sb)) =
                            clip_segment(camera.screen(a), camera.screen(b), self.viewport)
                        else {
                            continue;
                        };
                        out.push(Projected {
                            depth: (a[2] + b[2]) / 2.0,
                            mode: Mode::Line,
                            color: *color,
                            path: polyline(&[sa, sb], false),
                        });
                    }
                }
            }
        }
        // 먼 것부터 그린다.
        out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        out
    }
}

fn box_faces([a, b, c]: Vec3) -> Vec<Vec<Vec3>> {
    let mut faces = Vec::new();
    for axis in 0..3 {
        for sign in [-1.0, 1.0] {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let half = [a, b, c];
            let corner = |su: f32, sv: f32| {
                let mut p = [0.0; 3];
                p[axis] = sign * half[axis];
                p[u] = su * half[u];
                p[v] = sv * half[v];
                p
            };
            let mut outward = [0.0; 3];
            outward[axis] = sign;
            faces.push(facing(
                vec![
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                ],
                outward,
            ));
        }
    }
    faces
}

fn sphere_faces(radius: f32, segments: usize) -> Vec<Vec<Vec3>> {
    let rings = (segments / 2).max(2);
    let at = |ring: usize, seg: usize| {
        let theta = PI * ring as f32 / rings as f32;
        let phi = TAU * seg as f32 / segments as f32;
        [
            radius * theta.sin() * phi.cos(),
            radius * theta.cos(),
            -radius * theta.sin() * phi.sin(),
        ]
    };
    let mut faces = Vec::new();
    for ring in 0..rings {
        for seg in 0..segments {
            let mut points = vec![at(ring, seg)];
            if ring > 0 {
                points.push(at(ring, seg + 1));
            }
            points.push(at(ring + 1, seg + 1));
            if ring + 1 < rings {
                points.push(at(ring + 1, seg));
            }
            let outward = centroid(&points);
            faces.push(facing(points, outward));
        }
    }
    faces
}

/// 원기둥·원뿔: 아래 반지름 `bottom`, 위 반지름 `top`(원뿔은 0), 높이 `height`, 가운데가 원점.
fn frustum_faces(bottom: f32, top: f32, height: f32, segments: usize) -> Vec<Vec<Vec3>> {
    let (y0, y1) = (-height / 2.0, height / 2.0);
    let ring = |r: f32, y: f32, seg: usize| {
        let phi = TAU * seg as f32 / segments as f32;
        [r * phi.cos(), y, -r * phi.sin()]
    };
    let mut faces = Vec::new();
    for seg in 0..segments {
        let mut points = vec![ring(bottom, y0, seg), ring(bottom, y0, seg + 1)];
        if top > 0.0 {
            points.push(ring(top, y1, seg + 1));
            points.push(ring(top, y1, seg));
        } else {
            points.push([0.0, y1, 0.0]);
        }
        let c = centroid(&points);
        faces.push(facing(points, [c[0], 0.0, c[2]]));
    }
    faces.push(facing(
        (0..segments).map(|seg| ring(bottom, y0, seg)).collect(),
        [0.0, -1.0, 0.0],
    ));
    if top > 0.0 {
        faces.push(facing(
            (0..segments).map(|seg| ring(top, y1, seg)).collect(),
            [0.0, 1.0, 0.0],
        ));
    }
    faces
}

fn torus_faces(major: f32, minor: f32, segments: usize, sides: usize) -> Vec<Vec<Vec3>> {
    let at = |seg: usize, side: usize| {
        let phi = TAU * seg as f32 / segments as f32;
        let theta = TAU * side as f32 / sides as f32;
        let r = major + minor * theta.cos();
        [r * phi.cos(), minor * theta.sin(), -r * phi.sin()]
    };
    let mut faces = Vec::new();
    for seg in 0..segments {
        for side in 0..sides {
            let points = vec![
                at(seg, side),
                at(seg + 1, side),
                at(seg + 1, side + 1),
                at(seg, side + 1),
            ];
            let c = centroid(&points);
            let ring = normalize([c[0], 0.0, c[2]]).unwrap_or([1.0, 0.0, 0.0]);
            faces.push(facing(points, sub(c, ring.map(|v| v * major))));
        }
    }
    faces
}

fn vec3(args: &Args, ix: usize) -> mlua::Result<Vec3> {
    Ok([args.number(ix)?, args.number(ix + 1)?, args.number(ix + 2)?])
}

/// `ix`부터의 3차원 점: 숫자 여럿 또는 숫자 표 하나.
fn points3(args: &Args, ix: usize) -> mlua::Result<Vec<Vec3>> {
    let numbers = args.numbers(ix)?;
    if !numbers.len().is_multiple_of(3) {
        return Err(args.error("coordinates must come in x, y, z triples"));
    }
    Ok(numbers.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
}

fn segments(args: &Args, ix: usize, default: usize) -> mlua::Result<usize> {
    Ok(args
        .opt_number(ix)?
        .map_or(default, |n| n.round().clamp(3.0, 64.0) as usize))
}

fn positive(args: &Args, ix: usize) -> mlua::Result<f32> {
    let value = args.number(ix)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(args.error(format!("argument #{} must be greater than 0", ix + 1)))
    }
}

/// `g.newScene([x, y, w, h])`: 캔버스의 그 사각형(기본은 캔버스 전체)에 그리는 장면.
pub(super) fn create(
    lua: &Lua,
    recorder: Rc<RefCell<Recorder>>,
    args: Args,
) -> mlua::Result<Table> {
    let viewport = {
        let rec = recorder.borrow();
        if args.len() == 0 {
            [0.0, 0.0, rec.width, rec.height]
        } else {
            let viewport = [
                args.number(0)?,
                args.number(1)?,
                args.number(2)?,
                args.number(3)?,
            ];
            if viewport[2] <= 0.0 || viewport[3] <= 0.0 {
                return Err(args.error("the scene's width and height must be greater than 0"));
            }
            viewport
        }
    };
    let scene = Rc::new(RefCell::new(Scene::new(viewport, recorder.borrow().color)));
    let table = lua.create_table()?;
    table.raw_set(SCENE_MARK, true)?;

    macro_rules! function {
        ($name:literal, |$scene:ident, $args:ident| $body:expr) => {{
            let scene = scene.clone();
            table.raw_set(
                $name,
                lua.create_function(move |_, values: MultiValue| {
                    let $args = Args::with_self("scene:", SCENE_MARK, $name, values);
                    #[allow(unused_mut)]
                    let mut $scene = scene.borrow_mut();
                    $body
                })?,
            )?;
        }};
    }

    // camera(ex, ey, ez, tx, ty, tz[, fov]). fov는 세로 시야각(도), 기본 45.
    function!("camera", |scene, args| {
        scene.eye = vec3(&args, 0)?;
        scene.target = match args.opt_number(3)? {
            Some(_) => vec3(&args, 3)?,
            None => [0.0, 0.0, 0.0],
        };
        if let Some(fov) = args.opt_number(6)? {
            if !(1.0..=170.0).contains(&fov) {
                return Err(args.error("fov must be between 1 and 170 degrees"));
            }
            scene.projection = Projection::Perspective(fov.to_radians());
        }
        Ok(MultiValue::new())
    });
    // ortho(height): 원근 없이 그린다. `height`는 장면 칸에 보이는 세로 길이(장면 단위).
    function!("ortho", |scene, args| {
        scene.projection = Projection::Orthographic(positive(&args, 0)?);
        Ok(MultiValue::new())
    });
    // light(x, y, z[, ambient]): 빛이 오는 쪽과 주변광(0..1, 기본 0.35).
    function!("light", |scene, args| {
        scene.light = normalize(vec3(&args, 0)?)
            .ok_or_else(|| args.error("the light direction must not be 0, 0, 0"))?;
        if let Some(ambient) = args.opt_number(3)? {
            scene.ambient = ambient.clamp(0.0, 1.0);
        }
        Ok(MultiValue::new())
    });
    function!("setColor", |scene, args| {
        scene.color = args.color(0)?;
        Ok(MultiValue::new())
    });
    function!("push", |scene, args| {
        if scene.stack.len() >= 64 {
            return Err(args.error("too many pushes without pop"));
        }
        let transform = scene.transform;
        scene.stack.push(transform);
        Ok(MultiValue::new())
    });
    function!("pop", |scene, args| {
        scene.transform = scene
            .stack
            .pop()
            .ok_or_else(|| args.error("pop without a matching push"))?;
        Ok(MultiValue::new())
    });
    function!("origin", |scene, args| {
        let _ = args;
        scene.transform = Mat::IDENTITY;
        Ok(MultiValue::new())
    });
    function!("translate", |scene, args| {
        scene.transform = scene.transform.then(Mat::translation(vec3(&args, 0)?));
        Ok(MultiValue::new())
    });
    // rotate(angle, ax, ay, az): 축을 중심으로 오른손 법칙 방향으로 돌린다.
    function!("rotate", |scene, args| {
        let angle = args.number(0)?;
        let rotation = Mat::rotation(vec3(&args, 1)?, angle)
            .ok_or_else(|| args.error("the rotation axis must not be 0, 0, 0"))?;
        scene.transform = scene.transform.then(rotation);
        Ok(MultiValue::new())
    });
    function!("rotateX", |scene, args| {
        let rotation = Mat::rotation([1.0, 0.0, 0.0], args.number(0)?).unwrap();
        scene.transform = scene.transform.then(rotation);
        Ok(MultiValue::new())
    });
    function!("rotateY", |scene, args| {
        let rotation = Mat::rotation([0.0, 1.0, 0.0], args.number(0)?).unwrap();
        scene.transform = scene.transform.then(rotation);
        Ok(MultiValue::new())
    });
    function!("rotateZ", |scene, args| {
        let rotation = Mat::rotation([0.0, 0.0, 1.0], args.number(0)?).unwrap();
        scene.transform = scene.transform.then(rotation);
        Ok(MultiValue::new())
    });
    function!("scale", |scene, args| {
        let sx = args.number(0)?;
        let sy = args.opt_number(1)?.unwrap_or(sx);
        let sz = args.opt_number(2)?.unwrap_or(sx);
        scene.transform = scene.transform.then(Mat::scaling([sx, sy, sz]));
        Ok(MultiValue::new())
    });
    // box(mode, w[, h, d]): 가운데가 원점인 직육면체.
    function!("box", |scene, args| {
        let mode = args.mode(0)?;
        let w = positive(&args, 1)?;
        let h = if args.opt_number(2)?.is_some() {
            positive(&args, 2)?
        } else {
            w
        };
        let d = if args.opt_number(3)?.is_some() {
            positive(&args, 3)?
        } else {
            w
        };
        scene.faces(&args, mode, box_faces([w / 2.0, h / 2.0, d / 2.0]))?;
        Ok(MultiValue::new())
    });
    function!("sphere", |scene, args| {
        let mode = args.mode(0)?;
        let radius = positive(&args, 1)?;
        let segments = segments(&args, 2, 16)?;
        scene.faces(&args, mode, sphere_faces(radius, segments))?;
        Ok(MultiValue::new())
    });
    function!("cylinder", |scene, args| {
        let mode = args.mode(0)?;
        let (radius, height) = (positive(&args, 1)?, positive(&args, 2)?);
        let segments = segments(&args, 3, 24)?;
        scene.faces(&args, mode, frustum_faces(radius, radius, height, segments))?;
        Ok(MultiValue::new())
    });
    function!("cone", |scene, args| {
        let mode = args.mode(0)?;
        let (radius, height) = (positive(&args, 1)?, positive(&args, 2)?);
        let segments = segments(&args, 3, 24)?;
        scene.faces(&args, mode, frustum_faces(radius, 0.0, height, segments))?;
        Ok(MultiValue::new())
    });
    // torus(mode, R, r[, segments, sides]): y축을 둘러싼 고리.
    function!("torus", |scene, args| {
        let mode = args.mode(0)?;
        let (major, minor) = (positive(&args, 1)?, positive(&args, 2)?);
        let segments = segments(&args, 3, 24)?;
        let sides = args
            .opt_number(4)?
            .map_or(12, |n| n.round().clamp(3.0, 64.0) as usize);
        scene.faces(&args, mode, torus_faces(major, minor, segments, sides))?;
        Ok(MultiValue::new())
    });
    // plane(mode, w[, d]): y = 0의 바닥판. 양면이다.
    function!("plane", |scene, args| {
        let mode = args.mode(0)?;
        let w = positive(&args, 1)?;
        let d = if args.opt_number(2)?.is_some() {
            positive(&args, 2)?
        } else {
            w
        };
        let (a, c) = (w / 2.0, d / 2.0);
        let points = [[-a, 0.0, c], [a, 0.0, c], [a, 0.0, -c], [-a, 0.0, -c]];
        scene.face(&args, mode, &points, true)?;
        Ok(MultiValue::new())
    });
    // polygon(mode, x1, y1, z1, ...): 평평한 면 하나. 양면이다.
    function!("polygon", |scene, args| {
        let mode = args.mode(0)?;
        let points = points3(&args, 1)?;
        if points.len() < 3 {
            return Err(args.error("a polygon needs at least three points"));
        }
        scene.face(&args, mode, &points, true)?;
        Ok(MultiValue::new())
    });
    // mesh(mode, vertices, faces): vertices = {{x, y, z}, ...}, faces = {{1, 2, 3}, ...}(1부터). 양면이다.
    function!("mesh", |scene, args| {
        let mode = args.mode(0)?;
        let table = |ix: usize, what: &str| match args.values.get(ix) {
            Some(Value::Table(table)) => Ok(table.clone()),
            _ => Err(args.error(format!("argument #{} must be a table of {what}", ix + 1))),
        };
        let vertices: Vec<Vec3> = table(1, "{x, y, z} vertices")?
            .sequence_values::<Vec<f32>>()
            .map(|v| match v.ok().as_deref() {
                Some(&[x, y, z]) => Ok([x, y, z]),
                _ => Err(args.error("each vertex must be {x, y, z}")),
            })
            .collect::<mlua::Result<_>>()?;
        for face in table(2, "faces (lists of vertex numbers)")?.sequence_values::<Vec<i64>>() {
            let face =
                face.map_err(|_| args.error("each face must be a list of vertex numbers"))?;
            let points = face
                .iter()
                .map(|&ix| {
                    usize::try_from(ix - 1)
                        .ok()
                        .and_then(|ix| vertices.get(ix).copied())
                        .ok_or_else(|| {
                            args.error(format!(
                                "face uses vertex {ix}, but there are {} vertices (numbers start at 1)",
                                vertices.len()
                            ))
                        })
                })
                .collect::<mlua::Result<Vec<_>>>()?;
            if points.len() < 3 {
                return Err(args.error("each face needs at least three vertices"));
            }
            scene.face(&args, mode, &points, true)?;
        }
        Ok(MultiValue::new())
    });
    // line(x1, y1, z1, x2, y2, z2, ...): 3차원 꺾은선. 굵기는 g.setLineWidth를 따른다.
    function!("line", |scene, args| {
        let points: Vec<Vec3> = points3(&args, 0)?
            .into_iter()
            .map(|p| scene.transform.apply(p))
            .collect();
        if points.len() < 2 {
            return Err(args.error("a line needs at least two points"));
        }
        let color = scene.color;
        scene.add(&args, Item::Line { points, color })?;
        Ok(MultiValue::new())
    });
    // project(x, y, z): 지금 변환을 적용한 점이 캔버스의 어디에 그려지는지 `x, y, depth`. 카메라 뒤면 nil.
    function!("project", |scene, args| {
        let camera = Camera::new(&scene);
        let view = camera.view(scene.transform.apply(vec3(&args, 0)?));
        if view[2] < camera.near() {
            return Ok(MultiValue::from_vec(vec![Value::Nil]));
        }
        let [x, y] = camera.screen(view);
        Ok(MultiValue::from_vec(vec![
            Value::Number(f64::from(x)),
            Value::Number(f64::from(y)),
            Value::Number(f64::from(view[2])),
        ]))
    });
    // draw(): 모은 면과 선을 먼 것부터 캔버스에 그린다(지금의 g 변환을 따른다).
    {
        let scene = scene.clone();
        let recorder = recorder.clone();
        table.raw_set(
            "draw",
            lua.create_function(move |_, values: MultiValue| {
                let _ = Args::with_self("scene:", SCENE_MARK, "draw", values);
                let scene = scene.borrow();
                let projected = scene.project(&Camera::new(&scene));
                let mut rec = recorder.borrow_mut();
                for shape in projected {
                    rec.draw_with(shape.mode, &shape.path, shape.color)?;
                }
                Ok(())
            })?,
        )?;
    }

    let meta = lua.create_table()?;
    meta.raw_set(
        "__index",
        lua.create_function(|_, (_, key): (Value, Value)| -> mlua::Result<Value> {
            let key = match &key {
                Value::String(s) => s.to_string_lossy(),
                other => format!("{other:?}"),
            };
            Err(mlua::Error::runtime(format!(
                "scene:{key} does not exist. Available: scene:{}",
                FUNCTIONS.join(", scene:")
            )))
        })?,
    )?;
    table.set_metatable(Some(meta))?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_face_outward() {
        let check = |faces: Vec<Vec<Vec3>>, center: Vec3| {
            for face in faces {
                let n = newell(&face);
                assert!(dot(n, sub(centroid(&face), center)) > 0.0, "{face:?}");
            }
        };
        check(box_faces([1.0, 2.0, 3.0]), [0.0; 3]);
        check(sphere_faces(1.0, 8), [0.0; 3]);
        check(frustum_faces(1.0, 1.0, 2.0, 8), [0.0; 3]);
        check(frustum_faces(1.0, 0.0, 2.0, 8), [0.0; 3]);
    }

    #[test]
    fn rotation_follows_right_hand_rule() {
        // y축으로 90°: x → -z.
        let r = Mat::rotation([0.0, 1.0, 0.0], PI / 2.0).unwrap();
        let p = r.apply([1.0, 0.0, 0.0]);
        assert!(p[0].abs() < 1e-5 && (p[2] + 1.0).abs() < 1e-5, "{p:?}");
        let t = Mat::translation([5.0, 0.0, 0.0]).then(r);
        let p = t.apply([1.0, 0.0, 0.0]);
        assert!(
            (p[0] - 5.0).abs() < 1e-5 && (p[2] + 1.0).abs() < 1e-5,
            "{p:?}"
        );
    }

    #[test]
    fn clips_to_the_viewport() {
        let square = vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]];
        let clipped = clip_rect(square, [0.0, 0.0, 10.0, 10.0]);
        assert!(
            clipped
                .iter()
                .all(|p| (0.0..=5.0).contains(&p[0]) && (0.0..=5.0).contains(&p[1]))
        );
        assert_eq!(
            clip_segment([-5.0, 5.0], [15.0, 5.0], [0.0, 0.0, 10.0, 10.0]),
            Some(([0.0, 5.0], [10.0, 5.0]))
        );
        assert_eq!(
            clip_segment([-5.0, -5.0], [-1.0, -1.0], [0.0, 0.0, 10.0, 10.0]),
            None
        );
    }
}
