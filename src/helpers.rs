#[cfg(feature = "tracing")]
use tracing::instrument;

use crate::EdgeSide;

#[cfg_attr(feature = "tracing", instrument(skip_all))]
#[inline(always)]
pub(crate) fn on_side(point: [f32; 2], i: [[f32; 2]; 2]) -> EdgeSide {
    match (point[1] - i[0][1]) * (i[1][0] - i[0][0]) - (point[0] - i[0][0]) * (i[1][1] - i[0][1]) {
        x if x.abs() < 1e-2 => EdgeSide::Edge,
        x if x < 0.0 => EdgeSide::Right,
        _ => EdgeSide::Left,
    }
}

#[cfg_attr(feature = "tracing", instrument(skip_all))]
#[inline(always)]
pub(crate) fn on_segment(point: [f32; 2], i: [[f32; 2]; 2]) -> bool {
    (i[0][0].min(i[1][0])..=i[0][0].max(i[1][0])).contains(&point[0])
        && (i[0][1].min(i[1][1])..=i[0][1].max(i[1][1])).contains(&point[1])
        && (on_side(point, i) == EdgeSide::Edge)
}

// i should be counterclockwise from r
#[cfg_attr(feature = "tracing", instrument(skip_all))]
#[inline(always)]
pub(crate) fn heuristic(r: [f32; 2], to: [f32; 2], i: [[f32; 2]; 2]) -> f32 {
    let to = if on_side(r, i) == on_side(to, i) {
        mirror(to, i)
    } else {
        to
    };
    if r == i[0] || r == i[1] {
        distance_between(r, to)
    } else {
        let lroot = [r[0] - i[0][0], r[1] - i[0][1]];
        let lgoal = [to[0] - i[0][0], to[1] - i[0][1]];
        let rootgoal = [to[0] - r[0], to[1] - r[1]];
        let lr = [i[1][0] - i[0][0], i[1][1] - i[0][1]];
        let lr_num = lgoal[0] * lroot[1] - lgoal[1] * lroot[0];
        let denom = rootgoal[0] * lr[1] - rootgoal[1] * lr[0];
        match lr_num / denom {
            x if x < 0.0 => distance_between(r, i[0]) + distance_between(i[0], to),
            x if x > 1.0 => distance_between(r, i[1]) + distance_between(i[1], to),
            _ => distance_between(r, to),
        }
    }
}

#[cfg_attr(feature = "tracing", instrument(skip_all))]
pub(crate) fn turning_on(r: [f32; 2], to: [f32; 2], i: [[f32; 2]; 2]) -> Option<[f32; 2]> {
    let to = if on_side(r, i) == on_side(to, i) {
        mirror(to, i)
    } else {
        to
    };
    if r == i[0] {
        None
    } else if on_side(to, [r, i[0]]) == EdgeSide::Right {
        Some(i[0])
    } else if on_side(to, [r, i[1]]) == EdgeSide::Left {
        Some(i[1])
    } else {
        None
    }
}

#[cfg_attr(feature = "tracing", instrument(skip_all))]
#[inline(always)]
pub(crate) fn mirror(p: [f32; 2], i: [[f32; 2]; 2]) -> [f32; 2] {
    let dx = i[1][0] - i[0][0];
    let dy = i[1][1] - i[0][1];

    let a = (dx * dx - dy * dy) / (dx * dx + dy * dy);
    let b = 2.0 * dx * dy / (dx * dx + dy * dy);

    let x2 = a * (p[0] - i[0][0]) + b * (p[1] - i[0][1]) + i[0][0];
    let y2 = b * (p[0] - i[0][0]) - a * (p[1] - i[0][1]) + i[0][1];

    [x2, y2]
}

#[cfg_attr(feature = "tracing", instrument(skip_all))]
#[inline(always)]
pub(crate) fn distance_between(from: [f32; 2], to: [f32; 2]) -> f32 {
    ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt()
}

#[cfg_attr(feature = "tracing", instrument(skip_all))]
#[inline(always)]
pub(crate) fn line_intersect_segment(
    line: [[f32; 2]; 2],
    segment: [[f32; 2]; 2],
) -> Option<[f32; 2]> {
    let u = ((line[0][0] - segment[0][0]) * (line[0][1] - line[1][1])
        - (line[0][1] - segment[0][1]) * (line[0][0] - line[1][0]))
        / ((line[0][0] - line[1][0]) * (segment[0][1] - segment[1][1])
            - (line[0][1] - line[1][1]) * (segment[0][0] - segment[1][0]));

    if !(0.0..=1.0).contains(&u) || u.is_nan() {
        None
    } else {
        Some([
            (segment[0][0] + u * (segment[1][0] - segment[0][0])),
            (segment[0][1] + u * (segment[1][1] - segment[0][1])),
        ])
    }
}

#[cfg(test)]
mod tests {
    use crate::EdgeSide;

    use super::{heuristic, line_intersect_segment, mirror, on_side};

    #[test]
    fn test_on_side() {
        assert_eq!(
            on_side([0.0, 0.5], [[0.0, 0.0], [1.0, 0.0]]),
            EdgeSide::Left
        );
        assert_eq!(
            on_side([0.0, -0.5], [[0.0, 0.0], [1.0, 0.0]]),
            EdgeSide::Right
        );
        assert_eq!(
            on_side([1.0, 0.0], [[0.0, 0.0], [1.0, 1.0]]),
            EdgeSide::Right
        );
        assert_eq!(
            on_side([0.0, 1.0], [[0.0, 0.0], [1.0, 1.0]]),
            EdgeSide::Left
        );
        assert_eq!(
            on_side([2.0, 2.0], [[0.0, 0.0], [1.0, 1.0]]),
            EdgeSide::Edge
        );
    }

    #[test]
    fn test_mirror() {
        assert_eq!(mirror([1.0, 0.0], [[0.0, 0.0], [0.0, 1.0]]), [-1.0, 0.0]);
        assert_eq!(mirror([-1.0, 0.0], [[0.0, 0.0], [0.0, 1.0]]), [1.0, 0.0]);
    }

    #[test]
    fn test_heuristic() {
        assert_eq!(
            heuristic([0.0, 0.0], [1.0, 1.0], [[1.0, 0.0], [0.0, 1.0]]),
            2.0_f32.sqrt()
        );
        assert_eq!(
            heuristic([0.0, 0.0], [2.0, -1.0], [[1.0, 0.0], [0.0, 1.0]]),
            1.0 + 2.0_f32.sqrt()
        );
        assert_eq!(
            heuristic([0.0, 0.0], [-1.0, 2.0], [[1.0, 0.0], [0.0, 1.0]]),
            1.0 + 2.0_f32.sqrt()
        );
        assert_eq!(
            heuristic([0.0, 0.0], [1.0, -1.0], [[1.0, 0.0], [0.0, 1.0]]),
            2.0
        );
    }

    #[test]
    fn test_line_intersect() {
        assert_eq!(
            line_intersect_segment([[0.0, 0.5], [0.5, 0.5]], [[1.0, 0.0], [1.0, 1.0]]),
            Some([1.0, 0.5])
        );
        assert_eq!(
            line_intersect_segment([[0.0, 0.0], [0.5, 0.8]], [[1.0, 0.0], [1.0, 0.2]]),
            None
        );
        assert_eq!(
            line_intersect_segment([[0.0, 0.8], [0.5, 0.0]], [[1.0, 0.0], [1.0, 0.2]]),
            None
        );
    }

    #[test]
    fn test_line_intersect_colinear() {
        assert_eq!(
            line_intersect_segment([[0.0, 0.5], [0.5, 0.5]], [[-1.0, 0.5], [1.0, 0.5]]),
            None
        );
    }
}
