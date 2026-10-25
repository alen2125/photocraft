use photocraft_doc::{Knot, Path, PathOp, Subpath};
use photocraft_geom::Point;

use super::*;

/// A closed ring of `n` knots with smooth handles, radius `r`: a dense, curvy path.
fn ring(n: usize, r: f64) -> Path {
    let knots = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let (c, s) = (a.cos(), a.sin());
            let h = r * std::f64::consts::TAU / n as f64 / 3.0;
            Knot::smooth(Point::new(r * c, r * s), Point::new(r * c + h * s, r * s - h * c), Point::new(r * c - h * s, r * s + h * c))
        })
        .collect();
    Path { subpaths: vec![Subpath { closed: true, op: PathOp::Combine, knots }], ..Default::default() }
}

fn area(p: &Path) -> f64 {
    let sp = &p.subpaths[0];
    let n = sp.knots.len();
    (0..n)
        .map(|i| {
            let (a, b) = (sp.knots[i].anchor, sp.knots[(i + 1) % n].anchor);
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

#[test]
fn a_path_that_is_not_dense_is_returned_as_it_is() {
    let p = ring(200, 50.0);
    assert_eq!(simplify(&p, 0.05, 0.1), p);
}

#[test]
fn a_dense_path_loses_most_of_its_points_and_keeps_its_shape() {
    let p = ring(20_000, 5.0);
    let s = simplify(&p, 0.05, 0.1);
    let kept = s.subpaths[0].knots.len();
    assert!(kept < 600, "kept {kept} of 20000");
    assert!(s.subpaths[0].closed);
    // The circle's area (π r²) is kept to within a fraction of a percent.
    let want = std::f64::consts::PI * 25.0;
    assert!((area(&s) - want).abs() / want < 0.01, "{} vs {want}", area(&s));
    assert!(s.subpaths[0].knots.iter().all(|k| (k.anchor.x.hypot(k.anchor.y) - 5.0).abs() < 0.06));
}

#[test]
fn flatten_follows_the_curves_within_the_tolerance() {
    let p = ring(8, 100.0);
    let f = flatten(&p, 0.05);
    assert!(f.subpaths[0].knots.len() > 8 * 4);
    for k in &f.subpaths[0].knots {
        assert!((k.anchor.x.hypot(k.anchor.y) - 100.0).abs() < 0.4, "{:?}", k.anchor);
        assert!(!k.smooth && k.in_ctrl == k.anchor && k.out_ctrl == k.anchor, "corner knots only");
    }
}

#[test]
fn specks_vanish_and_lines_keep_both_ends() {
    let speck = Subpath::polygon(&[(0.0, 0.0), (0.01, 0.0), (0.0, 0.01)]);
    let line = Subpath::polyline(&[(0.0, 0.0), (0.02, 0.0), (0.04, 0.0), (10.0, 0.0), (10.01, 0.0)]);
    let t = thin(&Path { subpaths: vec![speck, line], ..Default::default() }, 0.1);
    assert_eq!(t.subpaths.len(), 1, "the speck is gone");
    let pts: Vec<f64> = t.subpaths[0].knots.iter().map(|k| k.anchor.x).collect();
    assert_eq!(pts, [0.0, 10.0, 10.01], "the far end of the line is kept");
}

#[test]
fn subdivide_cuts_long_pieces_and_leaves_short_ones() {
    let sq = {
        let mut sp = Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        sp.closed = true;
        Path { subpaths: vec![sp], ..Default::default() }
    };
    assert_eq!(subdivide(&sq, 2.5).subpaths[0].knots.len(), 16);
    assert_eq!(subdivide(&sq, 100.0).subpaths[0].knots.len(), 4);
    assert_eq!(subdivide(&sq, 0.0), sq, "a useless length changes nothing");
    assert_eq!(subdivide(&sq, f64::NAN), sq);
}

#[test]
fn hostile_numbers_do_not_panic_or_blow_up() {
    let mut p = ring(3, 1.0);
    p.subpaths[0].knots[1].out_ctrl = Point::new(f64::NAN, f64::INFINITY);
    p.subpaths[0].knots[2].in_ctrl = Point::new(1e300, -1e300);
    let f = flatten(&p, 0.05);
    assert!(f.subpaths[0].knots.len() < 3 * MAX_PIECES + 3);
    let _ = thin(&f, 0.1);
    let _ = flatten(&p, 0.0);
    let _ = flatten(&p, f64::NAN);
    assert!(flatten(&Path::default(), 0.1).subpaths.is_empty());
}
