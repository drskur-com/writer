//! 3차원 장면(`scene:draw()`)의 픽셀 그리기(ADR-0045): 깊이 버퍼, 픽셀마다 빛, 2×2 슈퍼샘플링.
//!
//! - 장면은 투영을 마친 삼각형 묶음([`Mesh`])으로 모양 목록에 들어간다(`Shape::Scene3D`). 점은 캔버스 좌표이고,
//!   `w`(원근이면 1/깊이)로 속성을 원근에 맞게 보간한다.
//! - 불투명한 것을 먼저 깊이 버퍼로 그리고, 반투명한 것은 먼 것부터 섞는다(깊이는 보되 쓰지 않는다).
//! - 빛은 픽셀마다 주변광 + 램버트다. 평면 음영은 세 꼭짓점의 법선이 같다.

use std::hash::{Hash, Hasher};

use super::canvas::{Color, Point};

/// 슈퍼샘플링을 해도 되는 샘플 수 상한(넘으면 1×1).
const MAX_SAMPLES: usize = 4_000_000;

/// 선이 같은 깊이의 면보다 앞에 보이도록 당기는 정도(상대값).
const LINE_BIAS: f32 = 1e-3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// 캔버스 좌표.
    pub p: Point,
    /// 원근 보간 무게(원근이면 1/깊이, 직교면 1).
    pub w: f32,
    /// 깊이 비교 값: 클수록 가깝다.
    pub depth: f32,
    /// 법선(장면 좌표, 정규화). 빛을 받지 않는 선은 쓰지 않는다.
    pub normal: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tri {
    pub v: [Vertex; 3],
    pub color: Color,
    /// 빛을 받는지(면). 선은 색 그대로다.
    pub lit: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub tris: Vec<Tri>,
    /// 빛이 오는 쪽(정규화)과 주변광.
    pub light: [f32; 3],
    pub ambient: f32,
    /// 장면 칸(캔버스 좌표의 네 꼭짓점, 그때의 2차원 변환을 적용한 것). 이 안만 보인다.
    pub clip: [Point; 4],
    /// `clip`의 테두리 상자 `[x, y, w, h]`.
    pub bounds: [f32; 4],
    /// 모양 목록 열쇠(만들 때 한 번 계산한다).
    pub key: u64,
}

impl Mesh {
    pub fn new(tris: Vec<Tri>, light: [f32; 3], ambient: f32, clip: [Point; 4]) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for [x, y] in clip {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let mut f = |v: f32| v.to_bits().hash(&mut hasher);
        for tri in &tris {
            for v in &tri.v {
                [v.p[0], v.p[1], v.w, v.depth]
                    .into_iter()
                    .chain(v.normal)
                    .for_each(&mut f);
            }
            tri.color.into_iter().for_each(&mut f);
            f(f32::from(u8::from(tri.lit)));
        }
        light.into_iter().chain([ambient]).for_each(&mut f);
        clip.iter().flatten().for_each(|v| f(*v));
        Self {
            tris,
            light,
            ambient,
            clip,
            bounds: [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)],
            key: hasher.finish(),
        }
    }
}

/// 그린 픽셀: 곱하지 않은 RGBA, `bounds`(캔버스 좌표)에 맞춰 놓는다.
pub struct Pixels {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// 캔버스 단위당 픽셀(픽셀 (0, 0)이 `bounds`의 왼쪽 위다).
    pub scale: f32,
}

/// 장면 칸을 PNG로(내보내기에서 그림으로 넣는다). `scale`은 캔버스 단위당 픽셀.
pub fn png(mesh: &Mesh, scale: f32) -> Option<Vec<u8>> {
    let pixels = rasterize(mesh, scale, 4096)?;
    let image = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba)?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// `scale`(캔버스 단위당 픽셀)로 장면 칸을 그린다. 칸이 비었으면 `None`.
pub fn rasterize(mesh: &Mesh, scale: f32, max_side: u32) -> Option<Pixels> {
    let [bx, by, bw, bh] = mesh.bounds;
    let fit = (max_side as f32 / (bw.max(bh) * scale)).min(1.0);
    let scale = scale * fit;
    let width = (bw * scale).ceil() as usize;
    let height = (bh * scale).ceil() as usize;
    if width == 0 || height == 0 {
        return None;
    }
    let ss = if width * height * 4 <= MAX_SAMPLES {
        2
    } else {
        1
    };
    let (sw, sh) = (width * ss, height * ss);
    let k = scale * ss as f32;
    // 샘플 좌표로 옮긴 점.
    let to_sample = |p: Point| [(p[0] - bx) * k, (p[1] - by) * k];
    let mut color = vec![[0f32; 4]; sw * sh];
    let mut depth = vec![f32::NEG_INFINITY; sw * sh];

    let (opaque, mut clear): (Vec<&Tri>, Vec<&Tri>) =
        mesh.tris.iter().partition(|tri| tri.color[3] >= 1.0);
    for tri in opaque {
        draw_tri(mesh, tri, &to_sample, sw, sh, &mut color, &mut depth, true);
    }
    // 반투명: 먼 것(깊이 값이 작은 것)부터.
    let mean = |tri: &Tri| tri.v.iter().map(|v| v.depth).sum::<f32>();
    clear.sort_by(|a, b| mean(a).total_cmp(&mean(b)));
    for tri in clear {
        draw_tri(mesh, tri, &to_sample, sw, sh, &mut color, &mut depth, false);
    }

    // ss × ss 샘플을 한 픽셀로(미리 곱한 값의 평균).
    let mut rgba = vec![0u8; width * height * 4];
    let n = (ss * ss) as f32;
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0f32; 4];
            for dy in 0..ss {
                for dx in 0..ss {
                    let c = color[(y * ss + dy) * sw + x * ss + dx];
                    for i in 0..4 {
                        sum[i] += c[i];
                    }
                }
            }
            let a = sum[3] / n;
            let out = &mut rgba[(y * width + x) * 4..][..4];
            if a > 0.0 {
                for i in 0..3 {
                    out[i] = ((sum[i] / n / a).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                out[3] = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    Some(Pixels {
        rgba,
        width: width as u32,
        height: height as u32,
        scale,
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_tri(
    mesh: &Mesh,
    tri: &Tri,
    to_sample: &impl Fn(Point) -> Point,
    sw: usize,
    sh: usize,
    color: &mut [[f32; 4]],
    depth: &mut [f32],
    write_depth: bool,
) {
    // 반시계(넓이 > 0)로 맞춘다.
    let [mut v0, mut v1, v2] = tri.v;
    let mut area = edge(to_sample(v0.p), to_sample(v1.p), to_sample(v2.p));
    if area < 0.0 {
        std::mem::swap(&mut v0, &mut v1);
        area = -area;
    }
    if area < 1e-9 {
        return;
    }
    let [a, b, c] = [v0, v1, v2].map(|v| to_sample(v.p));
    // 두 삼각형이 나눠 가진 변 위의 샘플은 한쪽만 칠한다(위·왼쪽 변 규칙). 반투명이 두 번 칠해지지 않는다.
    let top_left = |p: Point, q: Point| {
        let d = [q[0] - p[0], q[1] - p[1]];
        (d[1] == 0.0 && d[0] < 0.0) || d[1] > 0.0
    };
    let owns = [top_left(b, c), top_left(c, a), top_left(a, b)];
    let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
    let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
    let x1 = (a[0].max(b[0]).max(c[0]).ceil() as usize).min(sw);
    let y1 = (a[1].max(b[1]).max(c[1]).ceil() as usize).min(sh);
    let bias = if tri.lit { 0.0 } else { LINE_BIAS };
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let p = [px, py];
            let e = [edge(b, c, p), edge(c, a, p), edge(a, b, p)];
            if (0..3).any(|i| e[i] < 0.0 || (e[i] == 0.0 && !owns[i])) {
                continue;
            }
            // 무게중심 좌표.
            let (w0, w1, w2) = (e[0] / area, e[1] / area, e[2] / area);
            let z = w0 * v0.depth + w1 * v1.depth + w2 * v2.depth;
            let z = z + bias * z.abs().max(1e-3);
            let ix = y * sw + x;
            if z <= depth[ix] {
                continue;
            }
            let rgb = if tri.lit {
                // 원근에 맞춘 법선 보간.
                let (p0, p1, p2) = (w0 * v0.w, w1 * v1.w, w2 * v2.w);
                let total = p0 + p1 + p2;
                let n: [f32; 3] = std::array::from_fn(|i| {
                    (p0 * v0.normal[i] + p1 * v1.normal[i] + p2 * v2.normal[i]) / total
                });
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
                let lambert =
                    (n[0] * mesh.light[0] + n[1] * mesh.light[1] + n[2] * mesh.light[2]) / length;
                let light = mesh.ambient + (1.0 - mesh.ambient) * lambert.max(0.0);
                [
                    tri.color[0] * light,
                    tri.color[1] * light,
                    tri.color[2] * light,
                ]
            } else {
                [tri.color[0], tri.color[1], tri.color[2]]
            };
            let alpha = tri.color[3].clamp(0.0, 1.0);
            let dst = &mut color[ix];
            // 미리 곱한 값으로 위에 얹는다.
            for i in 0..3 {
                dst[i] = rgb[i] * alpha + dst[i] * (1.0 - alpha);
            }
            dst[3] = alpha + dst[3] * (1.0 - alpha);
            if write_depth {
                depth[ix] = z;
            }
        }
    }
}

/// `p`가 `a → b`의 왼쪽(화면 y가 아래라 반시계 기준)에 있는 정도(넓이의 두 배).
fn edge(a: Point, b: Point, p: Point) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(p: Point, depth: f32) -> Vertex {
        Vertex {
            p,
            w: 1.0,
            depth,
            normal: [0.0, 0.0, 1.0],
        }
    }

    fn square_tris(x: f32, size: f32, depth: f32, color: Color) -> Vec<Tri> {
        let [a, b, c, d] = [[x, 0.0], [x + size, 0.0], [x + size, size], [x, size]];
        vec![
            Tri {
                v: [vertex(a, depth), vertex(b, depth), vertex(c, depth)],
                color,
                lit: true,
            },
            Tri {
                v: [vertex(a, depth), vertex(c, depth), vertex(d, depth)],
                color,
                lit: true,
            },
        ]
    }

    #[test]
    fn nearer_wins_regardless_of_order() {
        let clip = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let mut tris = square_tris(0.0, 10.0, 2.0, [1.0, 0.0, 0.0, 1.0]);
        // 뒤에 넣었지만 더 먼(깊이 값이 작은) 파랑은 가려진다.
        tris.extend(square_tris(0.0, 10.0, 1.0, [0.0, 0.0, 1.0, 1.0]));
        let mesh = Mesh::new(tris, [0.0, 0.0, 1.0], 0.0, clip);
        let pixels = rasterize(&mesh, 1.0, 1000).unwrap();
        assert_eq!((pixels.width, pixels.height), (10, 10));
        assert_eq!(&pixels.rgba[(5 * 10 + 5) * 4..][..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn translucent_mixes_over_what_is_behind() {
        let clip = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let mut tris = square_tris(0.0, 10.0, 2.0, [0.0, 0.0, 1.0, 0.5]);
        tris.extend(square_tris(0.0, 10.0, 1.0, [1.0, 0.0, 0.0, 1.0]));
        let mesh = Mesh::new(tris, [0.0, 0.0, 1.0], 0.0, clip);
        let pixels = rasterize(&mesh, 1.0, 1000).unwrap();
        let p = &pixels.rgba[(5 * 10 + 5) * 4..][..4];
        assert!(
            p[0] > 110 && p[0] < 145 && p[2] > 110 && p[2] < 145,
            "{p:?}"
        );
        assert_eq!(p[3], 255);
    }
}
