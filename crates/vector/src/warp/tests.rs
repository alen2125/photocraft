use photocraft_doc::{Knot, Path, Subpath};
use photocraft_geom::Point;

use super::*;

fn leaf(width: f64) -> Leaf {
    Leaf { a: Point::new(10.0, 20.0), b: Point::new(110.0, 70.0), width, fullness: 1.0 }
}

fn square() -> Path {
    let mut sp = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    sp.closed = true;
    Path { subpaths: vec![sp], ..Default::default() }
}

fn near(p: Point, q: Point) -> bool {
    (p.x - q.x).abs() < 1e-6 && (p.y - q.y).abs() < 1e-6
}

fn points(p: &Path) -> Vec<Point> {
    p.subpaths.iter().flat_map(|s| s.knots.iter().flat_map(|k| [k.anchor, k.in_ctrl, k.out_ctrl])).collect()
}

#[test]
fn the_ends_of_the_box_land_on_a_and_b_whatever_the_height() {
    let l = leaf(0.5);
    for v in [0.0, 0.3, 0.5, 1.0] {
        assert!(near(l.at(0.0, v).unwrap(), l.a), "u 0, v {v}");
        assert!(near(l.at(1.0, v).unwrap(), l.b), "u 1, v {v}");
    }
}

#[test]
fn a_warped_shape_keeps_its_end_points_at_a_and_b() {
    let l = leaf(0.5);
    let out = warp_path(&square(), &l).unwrap();
    let on_a = points(&out).iter().filter(|p| near(**p, l.a)).count();
    let on_b = points(&out).iter().filter(|p| near(**p, l.b)).count();
    assert!(on_a >= 2 && on_b >= 2, "the left edge collapses into A ({on_a}) and the right into B ({on_b})");
}

#[test]
fn a_warped_shape_stays_inside_the_leaf() {
    let l = leaf(0.5);
    let outline = l.outline().unwrap();
    let (x0, y0, x1, y1) = outline.control_bounds().unwrap();
    let out = warp_path(&square(), &l).unwrap();
    let (a0, b0, a1, b1) = out.control_bounds().unwrap();
    assert!(a0 >= x0 - 1e-6 && b0 >= y0 - 1e-6 && a1 <= x1 + 1e-6 && b1 <= y1 + 1e-6, "{:?} vs {:?}", (a0, b0, a1, b1), (x0, y0, x1, y1));
}

#[test]
fn the_outline_has_corners_only_at_a_and_b_and_smooth_side_points() {
    let l = leaf(0.5);
    let o = l.outline().unwrap();
    let k: &Vec<Knot> = &o.subpaths[0].knots;
    assert_eq!(k.len(), 4);
    assert!(near(k[0].anchor, l.a) && !k[0].smooth);
    assert!(near(k[2].anchor, l.b) && !k[2].smooth);
    let dir = (l.b.x - l.a.x, l.b.y - l.a.y);
    for side in [&k[1], &k[3]] {
        assert!(side.smooth);
        // In and out handles are collinear with each other and parallel to AB.
        let hin = (side.anchor.x - side.in_ctrl.x, side.anchor.y - side.in_ctrl.y);
        let hout = (side.out_ctrl.x - side.anchor.x, side.out_ctrl.y - side.anchor.y);
        let cross = |p: (f64, f64), q: (f64, f64)| p.0 * q.1 - p.1 * q.0;
        assert!(cross(hin, hout).abs() < 1e-6 && cross(hin, dir).abs() < 1e-6, "{hin:?} {hout:?}");
    }
    // The side points are on opposite sides of AB.
    let side_of = |p: Point| (l.b.x - l.a.x) * (p.y - l.a.y) - (l.b.y - l.a.y) * (p.x - l.a.x);
    assert!(side_of(k[1].anchor) * side_of(k[3].anchor) < 0.0);
}

#[test]
fn a_zero_width_leaf_collapses_onto_the_segment_without_nan() {
    let l = leaf(0.0);
    let out = warp_path(&square(), &l).unwrap();
    for p in points(&out) {
        assert!(p.x.is_finite() && p.y.is_finite());
        let cross = (l.b.x - l.a.x) * (p.y - l.a.y) - (l.b.y - l.a.y) * (p.x - l.a.x);
        assert!(cross.abs() < 1e-6, "{p:?} is off the line AB");
    }
}

#[test]
fn an_unusable_leaf_or_path_gives_none() {
    let mut l = leaf(0.5);
    l.b = l.a;
    assert!(warp_path(&square(), &l).is_none(), "A and B coincide");
    assert!(l.outline().is_none() && l.at(0.5, 0.5).is_none());
    for bad in [f64::NAN, f64::INFINITY] {
        let mut l = leaf(0.5);
        l.a.x = bad;
        assert!(warp_path(&square(), &l).is_none());
        let mut l = leaf(0.5);
        l.width = bad;
        assert!(warp_path(&square(), &l).is_none());
        let mut l = leaf(0.5);
        l.fullness = bad;
        assert!(warp_path(&square(), &l).is_none());
    }
    assert!(warp_path(&Path::default(), &leaf(0.5)).is_none(), "an empty path");
}

#[test]
fn no_setting_produces_a_nan() {
    let curvy = {
        let mut sp = Subpath::polygon(&[(0.0, 0.0), (50.0, 30.0), (100.0, 0.0), (60.0, 100.0)]);
        sp.closed = true;
        sp.knots[1] = Knot::smooth(Point::new(50.0, 30.0), Point::new(30.0, 10.0), Point::new(70.0, 50.0));
        Path { subpaths: vec![sp], ..Default::default() }
    };
    for width in [0.0, 1e-12, 0.1, 0.5, 1.0, 4.0, 50.0, -1.0] {
        for fullness in [0.0, 0.1, 1.0, 3.0, 100.0] {
            let l = Leaf { a: Point::new(-5.0, 3.0), b: Point::new(1e-3, 3.0), width, fullness };
            let out = warp_path(&curvy, &l).unwrap();
            assert!(points(&out).iter().all(|p| p.x.is_finite() && p.y.is_finite()), "width {width}, fullness {fullness}");
        }
    }
}

#[test]
fn a_shape_mirrored_across_the_box_is_mirrored_across_ab() {
    let l = leaf(0.5);
    let (a, b) = (l.at(0.3, 0.2).unwrap(), l.at(0.3, 0.8).unwrap());
    let mid = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    // The midpoint of the pair lies on AB.
    let cross = (l.b.x - l.a.x) * (mid.y - l.a.y) - (l.b.y - l.a.y) * (mid.x - l.a.x);
    assert!(cross.abs() < 1e-6, "{cross}");
}

#[test]
fn a_flat_or_single_point_shape_still_warps() {
    let line = Path { subpaths: vec![Subpath::polyline(&[(0.0, 5.0), (40.0, 5.0)])], ..Default::default() };
    let out = warp_path(&line, &leaf(0.5)).unwrap();
    assert!(points(&out).iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    let dot = Path { subpaths: vec![Subpath::polyline(&[(3.0, 3.0), (3.0, 3.0)])], ..Default::default() };
    assert!(warp_path(&dot, &leaf(0.5)).is_some());
}

#[test]
fn from_drag_snaps_to_45_degrees_and_draws_from_the_centre() {
    let (s, e) = (Point::new(0.0, 0.0), Point::new(100.0, 10.0));
    let l = Leaf::from_drag(s, e, false, false, 0.5, 1.0);
    assert!(near(l.a, s) && near(l.b, e));
    let snapped = Leaf::from_drag(s, e, true, false, 0.5, 1.0);
    assert!(snapped.b.y.abs() < 1e-9, "snapped to horizontal: {:?}", snapped.b);
    assert!(((snapped.b.x - snapped.a.x) - 100.0_f64.hypot(10.0)).abs() < 1e-9, "the length is kept");
    let centred = Leaf::from_drag(s, e, false, true, 0.5, 1.0);
    assert!(near(centred.a, Point::new(-100.0, -10.0)) && near(centred.b, e));
}

fn coverage(path: &Path) -> (Vec<f32>, photocraft_geom::Rect) {
    let r = crate::fill_rasterizer(path, crate::DEFAULT_TOLERANCE);
    let area = photocraft_geom::Rect::new(0, 0, 400, 400);
    (r.render(area), area)
}

fn rectangle(w: f64, h: f64) -> Path {
    let mut sp = Subpath::polygon(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]);
    sp.closed = true;
    Path { subpaths: vec![sp], ..Default::default() }
}

#[test]
fn a_zipper_of_a_shape_that_fills_its_box_is_the_leaf_itself() {
    let l = Leaf { a: Point::new(50.0, 200.0), b: Point::new(350.0, 200.0), width: 0.5, fullness: 1.0 };
    let zipper = clip_to_leaf(&rectangle(10.0, 10.0), &l).unwrap();
    let (cov, _) = coverage(&zipper);
    let (leaf_cov, _) = coverage(&l.outline().unwrap());
    let (a, b): (f32, f32) = (cov.iter().sum(), leaf_cov.iter().sum());
    assert!(b > 10_000.0 && (a - b).abs() / b < 0.01, "zipper {a} vs leaf {b}");
    // Never outside the leaf.
    assert!(cov.iter().zip(&leaf_cov).all(|(z, l)| *z <= *l + 0.02));
}

#[test]
fn a_zipper_does_not_bend_the_shape() {
    // A straight vertical bar in the middle of a 10 x 10 box (an open diagonal line, which has no
    // area, only fixes the box). It stays straight and keeps its width.
    let l = Leaf { a: Point::new(50.0, 200.0), b: Point::new(350.0, 200.0), width: 0.5, fullness: 1.0 };
    let mut bar = Subpath::polygon(&[(4.0, 0.0), (6.0, 0.0), (6.0, 10.0), (4.0, 10.0)]);
    bar.closed = true;
    let frame = Subpath::polyline(&[(0.0, 0.0), (10.0, 10.0)]);
    let z = clip_to_leaf(&Path { subpaths: vec![bar, frame], ..Default::default() }, &l).unwrap();
    let (cov, _) = coverage(&z);
    let at = |x: usize, y: usize| cov[y * 400 + x];
    // The box is 300 long, so the bar is 60 px wide (x 170..230) and as tall as the leaf there.
    assert!(at(200, 200) > 0.99 && at(200, 150) > 0.99 && at(200, 250) > 0.99);
    assert!(at(160, 200) < 0.01 && at(240, 200) < 0.01, "the bar's sides are straight");
    assert!(at(172, 150) > 0.99 && at(228, 150) > 0.99 && at(168, 150) < 0.01, "and stay vertical away from the axis");
}

#[test]
fn a_zipper_cuts_the_box_corners_where_the_leaf_is_pointed() {
    let l = Leaf { a: Point::new(50.0, 200.0), b: Point::new(350.0, 200.0), width: 0.5, fullness: 1.0 };
    let (cov, _) = coverage(&clip_to_leaf(&rectangle(10.0, 10.0), &l).unwrap());
    let at = |x: usize, y: usize| cov[y * 400 + x];
    assert!(at(200, 200) > 0.99, "the middle is filled");
    assert!(at(55, 130) < 0.01 && at(345, 130) < 0.01 && at(55, 270) < 0.01 && at(345, 270) < 0.01, "the four corners of the box are cut away");
    assert!(at(200, 130) > 0.99, "the box reaches its full width at the middle");
}

#[test]
fn a_zipper_follows_a_slanted_axis_and_a_flat_leaf_without_nan() {
    let slanted = Leaf { a: Point::new(60.0, 60.0), b: Point::new(340.0, 340.0), width: 0.4, fullness: 1.0 };
    let (cov, _) = coverage(&clip_to_leaf(&rectangle(10.0, 4.0), &slanted).unwrap());
    assert!(cov[200 * 400 + 200] > 0.99);
    assert!(cov[60 * 400 + 340] < 0.01, "off the leaf's axis");
    let flat = Leaf { width: 0.0, ..slanted };
    let z = clip_to_leaf(&rectangle(10.0, 4.0), &flat).unwrap();
    assert!(points(&z).iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    assert!(clip_to_leaf(&rectangle(10.0, 4.0), &Leaf { b: slanted.a, ..slanted }).is_none(), "A and B coincide");
    assert!(clip_to_leaf(&Path::default(), &slanted).is_none(), "an empty path");
}

#[test]
fn box_coords_are_the_inverse_of_the_box() {
    let l = leaf(0.5);
    let (u, v) = l.box_coords(l.a).unwrap();
    assert!((u - 0.0).abs() < 1e-9 && (v - 0.5).abs() < 1e-9);
    let (u, v) = l.box_coords(l.b).unwrap();
    assert!((u - 1.0).abs() < 1e-9 && (v - 0.5).abs() < 1e-9);
    let mid = l.at(0.5, 1.0).unwrap();
    let (u, v) = l.box_coords(mid).unwrap();
    assert!((u - 0.5).abs() < 1e-6 && (v - 1.0).abs() < 1e-6, "{u} {v}");
}
