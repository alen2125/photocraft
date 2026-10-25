//! Dense paths made cheap to draw (fork-local, see MY_CHANGES.md).
//!
//! A traced drawing from a shape library holds tens of thousands of curves, most of them a
//! fraction of a pixel long. The rasterizer cuts every one into pieces fine enough for its own
//! size, so such a shape costs a tenth of a second to fill and more to preview. [`simplify`]
//! flattens the curves to a tolerance that is invisible at the size they are drawn and drops
//! points closer together than a fraction of a pixel; a path that is not dense comes back
//! unchanged.

use photocraft_doc::{Knot, Path, Subpath};
use photocraft_geom::Point;

/// A path with at most this many knots is left as it is.
pub const DENSE: usize = 1500;
/// Most points one curve is cut into.
const MAX_PIECES: usize = 4096;

fn knots_in(path: &Path) -> usize {
    path.subpaths.iter().fold(0usize, |n, s| n.saturating_add(s.knots.len()))
}

fn eval(seg: &[Point; 4], t: f64) -> Point {
    let [p0, c1, c2, p1] = seg;
    let mt = 1.0 - t;
    let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    Point::new(a * p0.x + b * c1.x + c * c2.x + d * p1.x, a * p0.y + b * c1.y + c * c2.y + d * p1.y)
}

/// The points of one cubic after its start, within `tol` of the curve.
fn sample(seg: &[Point; 4], tol: f64, out: &mut Vec<Point>) {
    let [p0, c1, c2, p1] = seg;
    let d1 = (p0.x - 2.0 * c1.x + c2.x).hypot(p0.y - 2.0 * c1.y + c2.y);
    let d2 = (c1.x - 2.0 * c2.x + p1.x).hypot(c1.y - 2.0 * c2.y + p1.y);
    let n = (0.75 * d1.max(d2) / tol.max(1e-9)).sqrt().ceil();
    let n = if n.is_finite() { (n as usize).clamp(1, MAX_PIECES) } else { 1 };
    for i in 1..n {
        out.push(eval(seg, i as f64 / n as f64));
    }
    out.push(*p1);
}

fn polyline(pts: &[Point], closed: bool, op: photocraft_doc::PathOp) -> Subpath {
    Subpath { closed, op, knots: pts.iter().map(|p| Knot::corner(p.x, p.y)).collect() }
}

/// `f` on every subpath, dropping the ones it gives up on, in order. The subpaths of a traced
/// drawing are independent and numerous, so a big path is split over the cores (not on wasm).
pub(crate) fn each_subpath(subpaths: &[Subpath], f: impl Fn(&Subpath) -> Option<Subpath> + Sync + Send) -> Vec<Subpath> {
    #[cfg(not(target_arch = "wasm32"))]
    if subpaths.len() > 256 {
        use rayon::prelude::*;
        return subpaths.par_iter().filter_map(f).collect();
    }
    subpaths.iter().filter_map(f).collect()
}

/// `path` with every curve replaced by straight pieces within `tol` of it: corner knots only.
pub fn flatten(path: &Path, tol: f64) -> Path {
    let subpaths = each_subpath(&path.subpaths, |sp| {
        let first = sp.knots.first()?;
        let mut pts = vec![first.anchor];
        for seg in sp.segments() {
            sample(&seg, tol, &mut pts);
        }
        // The closing piece of a closed subpath ends on its first point.
        if sp.closed && pts.len() > 1 && pts.last() == pts.first() {
            pts.pop();
        }
        Some(polyline(&pts, sp.closed, sp.op))
    });
    Path { subpaths, fill_rule: path.fill_rule, inverted: path.inverted }
}

/// Insert points along straight pieces longer than `max_len`, so a map that bends space bends them.
pub fn subdivide(path: &Path, max_len: f64) -> Path {
    let max_len = if max_len.is_finite() && max_len > 1e-9 { max_len } else { return path.clone() };
    let subpaths = path
        .subpaths
        .iter()
        .map(|sp| {
            let anchors: Vec<Point> = sp.knots.iter().map(|k| k.anchor).collect();
            let mut pts = Vec::with_capacity(anchors.len());
            let pairs = if sp.closed { anchors.len() } else { anchors.len().saturating_sub(1) };
            for i in 0..pairs {
                let (a, b) = (anchors[i], anchors[(i + 1) % anchors.len()]);
                pts.push(a);
                let n = ((b.x - a.x).hypot(b.y - a.y) / max_len).ceil();
                let n = if n.is_finite() { (n as usize).min(MAX_PIECES) } else { 1 };
                for j in 1..n {
                    let t = j as f64 / n as f64;
                    pts.push(Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
                }
            }
            if !sp.closed
                && let Some(last) = anchors.last()
            {
                pts.push(*last);
            }
            polyline(&pts, sp.closed, sp.op)
        })
        .collect();
    Path { subpaths, fill_rule: path.fill_rule, inverted: path.inverted }
}

/// Drop the points of a polyline path that lie within `min_dist` of the last point kept; a closed
/// subpath left with fewer than three points (a speck) is dropped.
pub fn thin(path: &Path, min_dist: f64) -> Path {
    let subpaths = each_subpath(&path.subpaths, |sp| {
        let mut kept: Vec<Point> = Vec::with_capacity(sp.knots.len());
        for k in &sp.knots {
            if kept.last().is_none_or(|l| (k.anchor.x - l.x).hypot(k.anchor.y - l.y) >= min_dist) {
                kept.push(k.anchor);
            }
        }
        // An open line keeps its end.
        if !sp.closed
            && let (Some(end), Some(last)) = (sp.knots.last(), kept.last())
            && end.anchor != *last
        {
            kept.push(end.anchor);
        }
        let enough = if sp.closed { kept.len() >= 3 } else { kept.len() >= 2 };
        enough.then(|| polyline(&kept, sp.closed, sp.op))
    });
    Path { subpaths, fill_rule: path.fill_rule, inverted: path.inverted }
}

/// A dense path (over [`DENSE`] knots) made cheap to draw: curves flattened to `tol` and points
/// closer than `min_dist` merged. Both are in the path's own units, so pass a fraction of a pixel
/// for a path in pixels. Any other path is returned as it is.
pub fn simplify(path: &Path, tol: f64, min_dist: f64) -> Path {
    if knots_in(path) <= DENSE {
        return path.clone();
    }
    thin(&flatten(path, tol), min_dist)
}

/// [`simplify`], then merge points further apart (up to 8 times `min_dist`) until at most
/// `max_points` remain, so the cost of drawing a path has a ceiling whatever it holds.
pub fn simplify_to(path: &Path, tol: f64, min_dist: f64, max_points: usize) -> Path {
    let mut out = simplify(path, tol, min_dist);
    let mut dist = min_dist;
    while knots_in(&out) > max_points && dist < min_dist * 8.0 {
        dist *= 1.5;
        out = thin(&out, dist);
    }
    out
}

#[cfg(test)]
mod tests;
