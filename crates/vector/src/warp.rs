//! The "leaf" warp of the Custom Shape tool (fork-local, see MY_CHANGES.md): a shape is bent to
//! fit a leaf dragged from point A to point B.
//!
//! A leaf is two sharp corners, A and B, and two smooth side points halfway between them, one on
//! each side of AB. Between A and B each side is two cubic Béziers that share a tangent (parallel
//! to AB) at the side point, so the outline has corners only at A and B.
//!
//! The shape's box (0..1 each way, `u` along AB, `v` across it) is sent onto the leaf by the
//! Coons patch whose left and right edges collapse to A and B and whose other two edges are the
//! leaf's sides. With edges like that the patch is the blend `(1 - v)·side₀(u) + v·side₁(u)`:
//! every point of the shape lands inside the leaf, and the shape's ends are pinched into A and B.

use photocraft_doc::{Knot, Path, Subpath};
use photocraft_geom::Point;

use crate::thin::{flatten, subdivide};

/// How far a curve may stray when flattened before the warp, and the longest straight piece left
/// uncut, both as a share of the shape's box diagonal.
const FLATTEN: f64 = 2.0e-4;
const PIECE: f64 = 1.0 / 96.0;
/// Widest leaf, as a multiple of the length AB.
const MAX_WIDTH: f64 = 4.0;
/// Range of the fullness control.
const FULLNESS: std::ops::RangeInclusive<f64> = 0.1..=3.0;

/// A leaf from A to B.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leaf {
    pub a: Point,
    pub b: Point,
    /// The leaf's width across the middle, as a multiple of the length AB (0 = a line).
    pub width: f64,
    /// How full the sides are: 1 is the default, less is slimmer near the ends, more is rounder.
    pub fullness: f64,
}

type Cubic = [Point; 4];

fn lerp(p: Point, q: Point, t: f64) -> Point {
    Point::new(p.x + (q.x - p.x) * t, p.y + (q.y - p.y) * t)
}

fn cubic(c: &Cubic, t: f64) -> Point {
    let (ab, bc, cd) = (lerp(c[0], c[1], t), lerp(c[1], c[2], t), lerp(c[2], c[3], t));
    lerp(lerp(ab, bc, t), lerp(bc, cd, t), t)
}

impl Leaf {
    /// A leaf for a drag from `start` to `end`: ⇧ (`snap`) turns AB to the nearest multiple of 45°,
    /// ⌥ (`from_center`) takes `start` as the middle of AB instead of A.
    pub fn from_drag(start: Point, end: Point, snap: bool, from_center: bool, width: f64, fullness: f64) -> Leaf {
        let (mut dx, mut dy) = (end.x - start.x, end.y - start.y);
        if snap {
            let angle = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
            let len = dx.hypot(dy);
            (dx, dy) = (len * angle.cos(), len * angle.sin());
        }
        let b = Point::new(start.x + dx, start.y + dy);
        let a = if from_center { Point::new(start.x - dx, start.y - dy) } else { start };
        Leaf { a, b, width, fullness }
    }

    /// The length AB and the unit vectors along and across it; `None` for a leaf that has no
    /// direction or numbers that are not finite.
    fn frame(&self) -> Option<(f64, Point, Point)> {
        let (dx, dy) = (self.b.x - self.a.x, self.b.y - self.a.y);
        let len = dx.hypot(dy);
        let ok = [self.a.x, self.a.y, self.b.x, self.b.y, self.width, self.fullness].iter().all(|v| v.is_finite());
        (ok && len > 1e-9).then(|| (len, Point::new(dx / len, dy / len), Point::new(-dy / len, dx / len)))
    }

    /// One side of the leaf as two cubics, A → side point → B; `side` is +1 or -1.
    fn side(&self, side: f64) -> Option<[Cubic; 2]> {
        let (len, along, across) = self.frame()?;
        let half = self.width.clamp(0.0, MAX_WIDTH) * len / 2.0 * side;
        let f = self.fullness.clamp(*FULLNESS.start(), *FULLNESS.end());
        let at = |s: f64, c: f64| Point::new(self.a.x + along.x * s + across.x * c, self.a.y + along.y * s + across.y * c);
        let top = at(len / 2.0, half);
        // Handles at the side point run parallel to AB (so it is smooth); at A and B they lean
        // out and along, which is what makes the corner pointed.
        let tangent = 0.2761 * len * f;
        let lean = (0.12 * len * f, 0.5523 * half * f);
        let first = [self.a, at(lean.0, lean.1), at(len / 2.0 - tangent, half), top];
        let second = [top, at(len / 2.0 + tangent, half), at(len - lean.0, lean.1), self.b];
        Some([first, second])
    }

    /// The point of the leaf at `u` along AB and `v` across it (both 0..1; `v` 0 is one side, 1
    /// the other). `u` 0 is A and `u` 1 is B whatever `v` is.
    pub fn at(&self, u: f64, v: f64) -> Option<Point> {
        let pick = |side: [Cubic; 2], u: f64| if u <= 0.5 { cubic(&side[0], u * 2.0) } else { cubic(&side[1], u * 2.0 - 1.0) };
        let (s0, s1) = (pick(self.side(-1.0)?, u), pick(self.side(1.0)?, u));
        Some(lerp(s0, s1, v))
    }

    /// The leaf's outline, A → side → B → side → A: corners at A and B, smooth side points.
    pub fn outline(&self) -> Option<Path> {
        let (minus, plus) = (self.side(-1.0)?, self.side(1.0)?);
        let knot = |anchor: Point, in_ctrl: Point, out_ctrl: Point, smooth: bool| Knot { anchor, in_ctrl, out_ctrl, smooth };
        let knots = vec![
            knot(self.a, minus[0][1], plus[0][1], false),
            knot(plus[0][3], plus[0][2], plus[1][1], true),
            knot(self.b, plus[1][2], minus[1][2], false),
            knot(minus[0][3], minus[1][1], minus[0][2], true),
        ];
        Some(Path { subpaths: vec![Subpath { closed: true, knots, ..Default::default() }], ..Default::default() })
    }
}

/// Bend `path` onto `leaf`: its control box becomes the leaf's (u, v) square. `None` for an empty
/// path, a box with no extent in either direction counted as one point, or an unusable leaf (A
/// and B coincide, a number not finite).
pub fn warp_path(path: &Path, leaf: &Leaf) -> Option<Path> {
    leaf.frame()?;
    let (x0, y0, x1, y1) = path.control_bounds().filter(|b| [b.0, b.1, b.2, b.3].iter().all(|v| v.is_finite()))?;
    let (w, h) = ((x1 - x0).max(1e-9), (y1 - y0).max(1e-9));
    let f = |p: Point| leaf.at(((p.x - x0) / w).clamp(0.0, 1.0), ((p.y - y0) / h).clamp(0.0, 1.0)).unwrap_or(p);
    // Points are mapped, not control points: near A and B the map pinches space, and a handle
    // carried across a pinch throws a curve far out of the leaf. Curves are flattened first and
    // long pieces cut so the straight ones bend too.
    let diag = w.hypot(h);
    let flat = subdivide(&flatten(path, diag * FLATTEN), diag * PIECE);
    let subpaths =
        flat.subpaths.into_iter().map(|sp| Subpath { knots: sp.knots.iter().map(|k| Knot::corner(f(k.anchor).x, f(k.anchor).y)).collect(), ..sp }).collect();
    Some(Path { subpaths, fill_rule: path.fill_rule, inverted: path.inverted })
}

#[cfg(test)]
mod tests;
