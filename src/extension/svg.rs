//! 확장 그림(모양 목록)을 SVG로 쓴다. ePub에 이미지로 넣는다(ADR-0026).
//!
//! - 크기는 캔버스 × `scale:`(CSS px), `viewBox`는 캔버스 좌표다. 캔버스 밖은 잘린다.
//! - 그라디언트는 캔버스 좌표(`userSpaceOnUse`)로, 투명도는 멈춤마다 둔다.
//! - 자르기는 바뀔 때마다 `<g clip-path>`를 닫고 다시 연다(사각형 하나 + 경로들, 겹친 곳).
//! - 글자는 리더의 sans-serif로, 줄 칸 가운데(`dominant-baseline="central"`)에 놓는다.

use std::fmt::Write as _;

use super::Still;
use super::canvas::{
    Align, Blend, Color, Effects, FillRule, LINE_HEIGHT, LineCap, LineJoin, Paint, Segment, Shape,
};

/// 그림 하나를 SVG 문서로.
pub fn to_svg(still: &Still) -> String {
    let (w, h) = still.canvas;
    let mut defs = String::new();
    let mut body = String::new();
    let mut ids = 0usize;
    let mut next_id = |prefix: &str| {
        ids += 1;
        format!("{prefix}{ids}")
    };
    let mut clip_rect: Option<[f32; 4]> = None;
    let mut clip_paths: Vec<(String, FillRule)> = Vec::new();
    let mut open = 0usize;
    // 지금 효과의 속성(섞기 `style`, 흐림·그림자 `filter`). 모양마다 붙인다.
    let mut fx = String::new();
    for shape in still.shapes.iter() {
        match shape {
            Shape::Effects(effects) => fx = effects_attr(effects, (w, h), &mut defs, &mut next_id),
            // 3차원 장면은 깊이 버퍼로 그린 그림(PNG)으로 넣는다.
            Shape::Scene3D(mesh) => {
                use base64::Engine as _;
                let Some(png) = super::raster3d::png(mesh, 3. * still.scale) else {
                    continue;
                };
                let [x, y, sw, sh] = mesh.bounds;
                let data = base64::engine::general_purpose::STANDARD.encode(png);
                let _ = writeln!(
                    body,
                    r#"<image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none"{fx} href="data:image/png;base64,{data}"/>"#,
                    num(x),
                    num(y),
                    num(sw),
                    num(sh)
                );
            }
            Shape::Image {
                image,
                src: [sx, sy, sw, sh],
                transform,
                opacity,
                smooth,
            } => {
                use base64::Engine as _;
                // 원래 파일을 data URI로 넣는다. 안쪽 <svg>가 `src` 밖을 자른다.
                let data = base64::engine::general_purpose::STANDARD.encode(image.encoded.as_slice());
                let matrix: Vec<String> = transform.iter().map(|v| num(*v)).collect();
                let _ = writeln!(
                    body,
                    r#"<g transform="matrix({})" opacity="{}"{fx}><svg x="{}" y="{}" width="{}" height="{}" overflow="hidden"><image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none"{} href="data:image/{};base64,{data}"/></svg></g>"#,
                    matrix.join(" "),
                    num(*opacity),
                    num(*sx),
                    num(*sy),
                    num(*sw),
                    num(*sh),
                    num(-sx),
                    num(-sy),
                    image.width,
                    image.height,
                    if *smooth { "" } else { r#" image-rendering="pixelated""# },
                    image.format
                );
            }
            Shape::Clip(rect) => {
                clip_rect = *rect;
                open = reclip(
                    &mut body,
                    &mut defs,
                    &mut next_id,
                    open,
                    clip_rect,
                    &clip_paths,
                );
            }
            Shape::ClipPath(next) => {
                match next {
                    Some((path, rule)) => clip_paths.push((path_data(path), *rule)),
                    None => clip_paths.clear(),
                }
                open = reclip(
                    &mut body,
                    &mut defs,
                    &mut next_id,
                    open,
                    clip_rect,
                    &clip_paths,
                );
            }
            Shape::Fill { path, paint, rule } => {
                let fill = paint_attr(paint, "fill", &mut defs, &mut next_id);
                let _ = writeln!(
                    body,
                    r#"<path d="{}" {fill} fill-rule="{}"{fx}/>"#,
                    path_data(path),
                    rule_name(*rule)
                );
            }
            Shape::Stroke { path, paint, style } => {
                let stroke = paint_attr(paint, "stroke", &mut defs, &mut next_id);
                let cap = match style.cap {
                    LineCap::Butt => "butt",
                    LineCap::Round => "round",
                    LineCap::Square => "square",
                };
                let join = match style.join {
                    LineJoin::Miter => "miter",
                    LineJoin::Round => "round",
                    LineJoin::Bevel => "bevel",
                };
                let _ = writeln!(
                    body,
                    r#"<path d="{}" fill="none" {stroke} stroke-width="{}" stroke-linecap="{cap}" stroke-linejoin="{join}" stroke-miterlimit="{}"{fx}/>"#,
                    path_data(path),
                    num(style.width),
                    num(style.miter_limit)
                );
            }
            Shape::Text {
                text,
                origin: [x, y],
                size,
                color,
                align,
                width,
                angle,
            } => {
                if text.trim().is_empty() {
                    continue;
                }
                let rotate = if *angle == 0. {
                    String::new()
                } else {
                    format!(
                        r#" transform="rotate({} {} {})""#,
                        num(angle.to_degrees()),
                        num(*x),
                        num(*y)
                    )
                };
                let (tx, anchor) = match (align, width) {
                    (Align::Center, Some(width)) => (x + width / 2., "middle"),
                    (Align::Right, Some(width)) => (x + width, "end"),
                    _ => (*x, "start"),
                };
                let x = tx;
                let _ = writeln!(
                    body,
                    r#"<text x="{}" y="{}" font-size="{}" text-anchor="{anchor}" dominant-baseline="central" {}{rotate}{fx}>{}</text>"#,
                    num(x),
                    num(y + size * LINE_HEIGHT / 2.),
                    num(*size),
                    color_attr("fill", *color),
                    escape(text)
                );
            }
        }
    }
    for _ in 0..open {
        body.push_str("</g>\n");
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}" font-family="sans-serif">
<defs>
{defs}</defs>
{body}</svg>
"#,
        num(w * still.scale),
        num(h * still.scale),
        num(w),
        num(h)
    )
}

/// 효과의 속성. 흐림·그림자는 `defs`에 필터를 더한다(영역은 캔버스 둘레까지 넉넉히).
fn effects_attr(
    effects: &Effects,
    (w, h): (f32, f32),
    defs: &mut String,
    next_id: &mut impl FnMut(&str) -> String,
) -> String {
    let mut attr = String::new();
    if effects.blend != Blend::Normal {
        let name = match effects.blend {
            Blend::Add => "plus-lighter".to_string(),
            other => {
                // colorDodge → color-dodge
                let mut css = String::new();
                for c in other.name().chars() {
                    if c.is_ascii_uppercase() {
                        css.push('-');
                        css.push(c.to_ascii_lowercase());
                    } else {
                        css.push(c);
                    }
                }
                css
            }
        };
        let _ = write!(attr, r#" style="mix-blend-mode:{name}""#);
    }
    if effects.blur > 0. || effects.shadow.is_some() {
        let id = next_id("f");
        let _ = writeln!(
            defs,
            r#"<filter id="{id}" filterUnits="userSpaceOnUse" x="{}" y="{}" width="{}" height="{}">"#,
            num(-w),
            num(-h),
            num(w * 3.),
            num(h * 3.)
        );
        if effects.blur > 0. {
            let _ = writeln!(defs, r#"<feGaussianBlur stdDeviation="{}"/>"#, num(effects.blur));
        }
        if let Some(shadow) = effects.shadow {
            let [r, g, b, a] = shadow.color;
            let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
            let _ = writeln!(
                defs,
                r##"<feDropShadow dx="{}" dy="{}" stdDeviation="{}" flood-color="#{:02x}{:02x}{:02x}" flood-opacity="{}"/>"##,
                num(shadow.offset[0]),
                num(shadow.offset[1]),
                num(shadow.blur),
                byte(r),
                byte(g),
                byte(b),
                num(a)
            );
        }
        defs.push_str("</filter>\n");
        let _ = write!(attr, r#" filter="url(#{id})""#);
    }
    attr
}

/// 열린 자르기 묶음을 닫고, 지금 자르기로 다시 연다. 연 수를 돌려준다.
fn reclip(
    body: &mut String,
    defs: &mut String,
    next_id: &mut impl FnMut(&str) -> String,
    open: usize,
    rect: Option<[f32; 4]>,
    paths: &[(String, FillRule)],
) -> usize {
    for _ in 0..open {
        body.push_str("</g>\n");
    }
    let mut count = 0;
    let mut push = |defs: &mut String, body: &mut String, inner: String| {
        let id = next_id("c");
        let _ = writeln!(defs, r#"<clipPath id="{id}">{inner}</clipPath>"#);
        let _ = writeln!(body, r#"<g clip-path="url(#{id})">"#);
        count += 1;
    };
    if let Some([x, y, w, h]) = rect {
        push(
            defs,
            body,
            format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}"/>"#,
                num(x),
                num(y),
                num(w.max(0.)),
                num(h.max(0.))
            ),
        );
    }
    for (d, rule) in paths {
        push(
            defs,
            body,
            format!(r#"<path d="{d}" clip-rule="{}"/>"#, rule_name(*rule)),
        );
    }
    count
}

fn paint_attr(
    paint: &Paint,
    attr: &str,
    defs: &mut String,
    next_id: &mut impl FnMut(&str) -> String,
) -> String {
    let stops = |stops: &[(f32, Color)]| -> String {
        stops
            .iter()
            .map(|(offset, color)| {
                format!(
                    r#"<stop offset="{}" stop-color="{}" stop-opacity="{}"/>"#,
                    num(offset.clamp(0., 1.)),
                    hex(*color),
                    num(color[3].clamp(0., 1.))
                )
            })
            .collect()
    };
    match paint {
        Paint::Solid(color) => color_attr(attr, *color),
        Paint::Linear { from, to, stops: s } => {
            let id = next_id("g");
            let _ = writeln!(
                defs,
                r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{}" y1="{}" x2="{}" y2="{}">{}</linearGradient>"#,
                num(from[0]),
                num(from[1]),
                num(to[0]),
                num(to[1]),
                stops(s)
            );
            format!(r#"{attr}="url(#{id})""#)
        }
        Paint::Radial {
            center,
            radius,
            stops: s,
        } => {
            let id = next_id("g");
            let _ = writeln!(
                defs,
                r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="{}" cy="{}" r="{}">{}</radialGradient>"#,
                num(center[0]),
                num(center[1]),
                num(*radius),
                stops(s)
            );
            format!(r#"{attr}="url(#{id})""#)
        }
    }
}

fn color_attr(attr: &str, color: Color) -> String {
    let alpha = color[3].clamp(0., 1.);
    if alpha >= 1. {
        format!(r#"{attr}="{}""#, hex(color))
    } else {
        format!(r#"{attr}="{}" {attr}-opacity="{}""#, hex(color), num(alpha))
    }
}

fn hex([r, g, b, _]: Color) -> String {
    let c = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

fn rule_name(rule: FillRule) -> &'static str {
    match rule {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    }
}

fn path_data(segments: &[Segment]) -> String {
    let mut d = String::new();
    for segment in segments {
        let _ = match *segment {
            Segment::Move([x, y]) => write!(d, "M{} {}", num(x), num(y)),
            Segment::Line([x, y]) => write!(d, "L{} {}", num(x), num(y)),
            Segment::Cubic([ax, ay], [bx, by], [x, y]) => write!(
                d,
                "C{} {} {} {} {} {}",
                num(ax),
                num(ay),
                num(bx),
                num(by),
                num(x),
                num(y)
            ),
            Segment::Close => write!(d, "Z"),
        };
    }
    d
}

/// 소수 둘째 자리까지, 끝의 0은 뺀다.
fn num(value: f32) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn writes_effects_and_rotated_text() {
        let still = Still {
            canvas: (100., 50.),
            scale: 1.,
            align: crate::extension::Align::Center,
            shapes: Arc::new(vec![
                Shape::Effects(Effects {
                    blend: Blend::ColorDodge,
                    blur: 2.,
                    shadow: Some(crate::extension::canvas::Shadow {
                        color: [0., 0., 0., 0.5],
                        blur: 3.,
                        offset: [1., 2.],
                    }),
                }),
                Shape::Text {
                    text: "a".into(),
                    origin: [10., 20.],
                    size: 10.,
                    color: [0., 0., 0., 1.],
                    align: Align::Left,
                    width: None,
                    angle: std::f32::consts::FRAC_PI_2,
                },
            ]),
        };
        let svg = to_svg(&still);
        assert!(svg.contains(r#"transform="rotate(90 10 20)""#), "{svg}");
        assert!(svg.contains(r#"style="mix-blend-mode:color-dodge" filter="url(#f1)""#), "{svg}");
        assert!(svg.contains(r#"<feGaussianBlur stdDeviation="2"/>"#), "{svg}");
        assert!(
            svg.contains(r##"<feDropShadow dx="1" dy="2" stdDeviation="3" flood-color="#000000" flood-opacity="0.5"/>"##),
            "{svg}"
        );
    }

    #[test]
    fn writes_paths_gradients_clips_and_text() {
        let square = vec![
            Segment::Move([0., 0.]),
            Segment::Line([10., 0.]),
            Segment::Cubic([10., 5.], [5., 10.], [0., 10.]),
            Segment::Close,
        ];
        let still = Still {
            canvas: (100., 50.),
            scale: 2.,
            align: crate::extension::Align::Center,
            shapes: Arc::new(vec![
                Shape::Clip(Some([0., 0., 50., 50.])),
                Shape::Fill {
                    path: square.clone(),
                    paint: Paint::Linear {
                        from: [0., 0.],
                        to: [10., 0.],
                        stops: vec![(0., [1., 0., 0., 1.]), (1., [0., 0., 1., 0.5])],
                    },
                    rule: FillRule::EvenOdd,
                },
                Shape::Clip(None),
                Shape::Text {
                    text: "a < b".into(),
                    origin: [0., 20.],
                    size: 10.,
                    color: [0., 0., 0., 1.],
                    align: Align::Center,
                    width: Some(100.),
                    angle: 0.,
                },
            ]),
        };
        let svg = to_svg(&still);
        assert!(
            svg.contains(r#"width="200" height="100" viewBox="0 0 100 50""#),
            "{svg}"
        );
        assert!(svg.contains("M0 0L10 0C10 5 5 10 0 10Z"), "{svg}");
        assert!(
            svg.contains(r#"fill="url(#g2)" fill-rule="evenodd""#),
            "{svg}"
        );
        assert!(
            svg.contains(r##"stop-color="#0000ff" stop-opacity="0.5""##),
            "{svg}"
        );
        assert!(svg.contains(r#"<g clip-path="url(#c1)">"#), "{svg}");
        assert_eq!(svg.matches("<g ").count(), svg.matches("</g>").count());
        assert!(svg.contains(r#"x="50" y="26.25""#), "{svg}");
        assert!(svg.contains("a &lt; b"), "{svg}");
    }
}
