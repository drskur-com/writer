//! 경로의 불리언 연산(합·차·교·XOR), 선 윤곽(`outline`), 넓히기·좁히기(`offset`). i_overlay로 한다.
//!
//! 곡선은 촘촘한 직선으로 편 뒤 연산한다. 결과의 하위 경로는 닫힌 다각형이고, 바깥은 반시계·구멍은
//! 시계 방향이라 nonzero로 칠하면 구멍이 남는다.

use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{
    LineCap as OverlayCap, LineJoin as OverlayJoin, OutlineStyle, StrokeStyle as OverlayStroke,
};
use i_overlay::mesh::math::MathMode;
use lyon::path::Event;
use lyon::path::iterator::PathIterator as _;

use super::canvas::{FillRule, LineCap, LineJoin, Point, Segment};
use super::path::LuaPath;

/// 곡선을 펴는 허용 오차(지역 좌표).
const FLATTEN_TOLERANCE: f32 = 0.05;

/// 둥근 이음·끝의 조각 각도(라디안 비율). 작을수록 매끄럽다.
const ROUND_STEP: f64 = 0.15;

/// 결과 점의 가장 많은 수. 지나치게 잘게 나뉜 입력을 막는다.
const MAX_POINTS: usize = 200_000;

type Contour = Vec<[f64; 2]>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Op {
    Union,
    Subtract,
    Intersect,
    Xor,
}

/// 하위 경로마다 편 점 목록과 닫혔는지.
fn flatten(segments: &[Segment]) -> Vec<(Contour, bool)> {
    let path = LuaPath::to_lyon(segments);
    let mut out = Vec::new();
    let mut current: Contour = Vec::new();
    let p = |at: lyon::math::Point| [f64::from(at.x), f64::from(at.y)];
    for event in path.iter().flattened(FLATTEN_TOLERANCE) {
        match event {
            Event::Begin { at } => {
                current.clear();
                current.push(p(at));
            }
            Event::Line { to, .. } => current.push(p(to)),
            Event::End { close, .. } => {
                // 닫는 점이 처음과 같으면 뺀다(i_overlay는 스스로 닫는다).
                if current.len() > 1 && current.first() == current.last() {
                    current.pop();
                }
                out.push((std::mem::take(&mut current), close));
            }
            Event::Quadratic { .. } | Event::Cubic { .. } => {}
        }
    }
    out
}

/// 칠할 다각형(열린 하위 경로도 닫아 본다). 점이 셋보다 적은 것은 뺀다.
fn polygons(segments: &[Segment]) -> Vec<Contour> {
    flatten(segments)
        .into_iter()
        .map(|(contour, _)| contour)
        .filter(|contour| contour.len() >= 3)
        .collect()
}

fn to_segments(shapes: Vec<Vec<Contour>>) -> Result<Vec<Segment>, String> {
    let total: usize = shapes.iter().flatten().map(Vec::len).sum();
    if total > MAX_POINTS {
        return Err(format!(
            "the result has too many points ({total}, limit {MAX_POINTS})"
        ));
    }
    let mut out = Vec::with_capacity(total + shapes.len() * 2);
    let pt = |[x, y]: [f64; 2]| -> Point { [x as f32, y as f32] };
    for contour in shapes.into_iter().flatten() {
        let mut points = contour.into_iter();
        let Some(first) = points.next() else {
            continue;
        };
        out.push(Segment::Move(pt(first)));
        out.extend(points.map(|p| Segment::Line(pt(p))));
        out.push(Segment::Close);
    }
    Ok(out)
}

fn overlay_fill(rule: FillRule) -> OverlayFill {
    match rule {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    }
}

/// `a`와 `b`의 불리언 연산. 두 경로 모두 `rule`로 칠한 모양으로 본다.
pub(super) fn boolean(
    a: &[Segment],
    b: &[Segment],
    op: Op,
    rule: FillRule,
) -> Result<Vec<Segment>, String> {
    let (subject, clip) = (polygons(a), polygons(b));
    let rule_op = match op {
        Op::Union => OverlayRule::Union,
        Op::Subtract => OverlayRule::Difference,
        Op::Intersect => OverlayRule::Intersect,
        Op::Xor => OverlayRule::Xor,
    };
    to_segments(subject.overlay(&clip, rule_op, overlay_fill(rule)))
}

/// 선 모양(굵기 `width`, 이음, 끝)으로 그은 선의 윤곽을 칠할 경로로 돌려준다.
pub(super) fn outline(
    segments: &[Segment],
    width: f32,
    join: LineJoin,
    cap: LineCap,
    miter_limit: f32,
) -> Result<Vec<Segment>, String> {
    if width <= 0.0 {
        return Err("the width must be more than 0".into());
    }
    let join = match join {
        LineJoin::Bevel => OverlayJoin::Bevel,
        LineJoin::Round => OverlayJoin::Round(ROUND_STEP),
        // miter 한계 l은 1 / sin(θ/2): 이보다 뾰족한 모서리(θ보다 작은 안쪽 각)는 자른다.
        LineJoin::Miter => OverlayJoin::Miter(2.0 * (1.0 / f64::from(miter_limit.max(1.0))).asin()),
    };
    let cap = match cap {
        LineCap::Butt => OverlayCap::Butt,
        LineCap::Round => OverlayCap::Round(ROUND_STEP),
        LineCap::Square => OverlayCap::Square,
    };
    let style = OverlayStroke::new(f64::from(width))
        .line_join(join)
        .start_cap(cap.clone())
        .end_cap(cap)
        .math(MathMode::Float);
    let mut shapes = Vec::new();
    for (contour, closed) in flatten(segments) {
        if contour.len() < 2 {
            continue;
        }
        shapes.extend(contour.stroke(style.clone(), closed));
    }
    // 하위 경로끼리 겹친 곳을 하나로 합친다.
    let merged: Vec<Contour> = shapes.into_iter().flatten().collect();
    to_segments(merged.overlay(
        &Vec::<Contour>::new(),
        OverlayRule::Subject,
        OverlayFill::NonZero,
    ))
}

/// 칠한 모양을 `distance`만큼 넓히거나(양수) 좁힌다(음수).
pub(super) fn offset(
    segments: &[Segment],
    distance: f32,
    join: LineJoin,
    rule: FillRule,
) -> Result<Vec<Segment>, String> {
    // 먼저 칠 규칙대로 정리해 바깥은 반시계, 구멍은 시계 방향으로 만든다.
    let cleaned = polygons(segments).overlay(
        &Vec::<Contour>::new(),
        OverlayRule::Subject,
        overlay_fill(rule),
    );
    let join = match join {
        LineJoin::Bevel => OverlayJoin::Bevel,
        LineJoin::Round => OverlayJoin::Round(ROUND_STEP),
        LineJoin::Miter => OverlayJoin::Miter(std::f64::consts::PI / 18.0),
    };
    let style = OutlineStyle::new(f64::from(distance)).line_join(join);
    let mut out = Vec::new();
    for shape in cleaned {
        out.extend(shape.outline(&style));
    }
    to_segments(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::path::parse_svg;

    /// 칠한 넓이(nonzero, 바깥 반시계·구멍 시계라 부호 있는 합).
    fn area(segments: &[Segment]) -> f32 {
        polygons(segments)
            .iter()
            .map(|c| {
                let n = c.len();
                (0..n)
                    .map(|i| {
                        let (a, b) = (c[i], c[(i + 1) % n]);
                        a[0] * b[1] - b[0] * a[1]
                    })
                    .sum::<f64>()
                    / 2.0
            })
            .sum::<f64>()
            .abs() as f32
    }

    fn square(x: f32, y: f32, size: f32) -> Vec<Segment> {
        parse_svg(&format!("M{x} {y} h{size} v{size} h-{size} Z")).unwrap()
    }

    #[test]
    fn boolean_operations_have_the_right_area() {
        // 10x10 두 개가 5x10만큼 겹친다.
        let (a, b) = (square(0.0, 0.0, 10.0), square(5.0, 0.0, 10.0));
        let rule = FillRule::NonZero;
        let check = |op, want: f32| {
            let got = area(&boolean(&a, &b, op, rule).unwrap());
            assert!((got - want).abs() < 0.01, "{op:?}: {got}");
        };
        check(Op::Union, 150.0);
        check(Op::Intersect, 50.0);
        check(Op::Subtract, 50.0);
        check(Op::Xor, 100.0);
        // 원에서 원을 빼면 고리(구멍 하나).
        let ring = boolean(
            &parse_svg("M0 0 m-10 0 a10 10 0 1 0 20 0 a10 10 0 1 0 -20 0").unwrap(),
            &parse_svg("M0 0 m-5 0 a5 5 0 1 0 10 0 a5 5 0 1 0 -10 0").unwrap(),
            Op::Subtract,
            rule,
        )
        .unwrap();
        let moves = ring
            .iter()
            .filter(|s| matches!(s, Segment::Move(_)))
            .count();
        assert_eq!(moves, 2);
        let want = std::f32::consts::PI * (100.0 - 25.0);
        assert!((area(&ring) - want).abs() < want * 0.01, "{}", area(&ring));
    }

    #[test]
    fn outline_turns_a_line_into_a_filled_shape() {
        let line = parse_svg("M0 0 L 100 0").unwrap();
        let shape = outline(&line, 10.0, LineJoin::Miter, LineCap::Butt, 10.0).unwrap();
        assert!((area(&shape) - 1000.0).abs() < 1.0, "{}", area(&shape));
        let capped = outline(&line, 10.0, LineJoin::Miter, LineCap::Square, 10.0).unwrap();
        assert!((area(&capped) - 1100.0).abs() < 1.0, "{}", area(&capped));
        assert!(outline(&line, 0.0, LineJoin::Miter, LineCap::Butt, 10.0).is_err());
    }

    #[test]
    fn offset_grows_and_shrinks() {
        let sq = square(0.0, 0.0, 20.0);
        let grown = offset(&sq, 2.0, LineJoin::Miter, FillRule::NonZero).unwrap();
        assert!((area(&grown) - 576.0).abs() < 2.0, "{}", area(&grown));
        let shrunk = offset(&sq, -2.0, LineJoin::Miter, FillRule::NonZero).unwrap();
        assert!((area(&shrunk) - 256.0).abs() < 2.0, "{}", area(&shrunk));
    }
}
