use glam::{Quat, Vec3};

use super::*;

/// A ray straight down -Z through the point (x, y): what an orthographic view over the gizmo gives.
fn at(x: f32, y: f32) -> Ray {
    Ray { origin: Vec3::new(x, y, 10.0), dir: Vec3::NEG_Z }
}

fn unit() -> Gizmo {
    Gizmo { origin: Vec3::ZERO, axes: [Vec3::X, Vec3::Y, Vec3::Z], size: 1.0 }
}

const UP: Vec3 = Vec3::Y;

#[test]
fn translate_picks_the_axis_the_ray_passes_nearest() {
    let g = unit();
    assert_eq!(g.pick(GizmoMode::Translate, at(0.5, 0.02)), Some(Handle::Axis(0)));
    assert_eq!(g.pick(GizmoMode::Translate, at(0.02, 0.5)), Some(Handle::Axis(1)));
    assert_eq!(g.pick(GizmoMode::Translate, at(3.0, 3.0)), None);
    assert_eq!(g.pick(GizmoMode::Translate, at(0.7, 0.7)), None, "between the axes");
    // The handles are easy to hit: a ray a fifth of a gizmo away from an arrow still takes it.
    assert_eq!(g.pick(GizmoMode::Translate, at(0.6, 0.2)), Some(Handle::Axis(0)));
}

#[test]
fn translate_drag_moves_along_the_axis_only() {
    let g = unit();
    let d = g.begin(GizmoMode::Translate, Handle::Axis(0), at(0.5, 0.0), UP).unwrap();
    let Some(Delta::Translate(v)) = d.update(at(0.9, 0.3)) else { panic!("not a translation") };
    assert!((v - Vec3::new(0.4, 0.0, 0.0)).length() < 1e-5, "{v:?}");
    // Dragging back to where it started is no movement.
    let Some(Delta::Translate(v)) = d.update(at(0.5, -2.0)) else { panic!() };
    assert!(v.length() < 1e-5);
}

#[test]
fn rotate_picks_a_ring_and_turns_by_the_angle_swept() {
    let g = unit();
    // Looking down -Z the ring about Z is the circle of radius 1; the X and Y rings are edge-on.
    assert_eq!(g.pick(GizmoMode::Rotate, at(1.0, 0.0)), Some(Handle::Axis(2)));
    assert_eq!(g.pick(GizmoMode::Rotate, at(0.05, 0.05)), None, "the middle is not a ring");
    let d = g.begin(GizmoMode::Rotate, Handle::Axis(2), at(1.0, 0.0), UP).unwrap();
    let Some(Delta::Rotate(q)) = d.update(at(0.0, 1.0)) else { panic!("not a rotation") };
    assert!((q * Vec3::X - Vec3::Y).length() < 1e-4, "a quarter turn takes X to Y");
    let Some(Delta::Rotate(q)) = d.update(at(0.0, -1.0)) else { panic!() };
    assert!((q * Vec3::X + Vec3::Y).length() < 1e-4, "the other way takes X to -Y");
    let Some(Delta::Rotate(q)) = d.update(at(2.0, 0.0)) else { panic!() };
    assert!(q.angle_between(Quat::IDENTITY) < 1e-4, "same direction from the centre: no turn");
}

#[test]
fn scale_grabs_the_tips_and_the_centre_and_scales_by_the_ratio() {
    let g = unit();
    assert_eq!(g.pick(GizmoMode::Scale, at(0.9, 0.01)), Some(Handle::Axis(0)));
    assert_eq!(g.pick(GizmoMode::Scale, at(0.3, 0.3)), None, "the shaft is not a scale handle");
    assert_eq!(g.pick(GizmoMode::Scale, at(0.05, 0.03)), Some(Handle::Uniform));
    let d = g.begin(GizmoMode::Scale, Handle::Axis(0), at(1.0, 0.0), UP).unwrap();
    let Some(Delta::Scale(f)) = d.update(at(2.0, 0.5)) else { panic!("not a scale") };
    assert!((f - Vec3::new(2.0, 1.0, 1.0)).length() < 1e-4, "{f:?}");
    let Some(Delta::Scale(f)) = d.update(at(0.5, 0.0)) else { panic!() };
    assert!((f.x - 0.5).abs() < 1e-4);
    // Pulled through the centre it stays positive and tiny, never zero or negative.
    let Some(Delta::Scale(f)) = d.update(at(-3.0, 0.0)) else { panic!() };
    assert!(f.x > 0.0);

    let u = g.begin(GizmoMode::Scale, Handle::Uniform, at(0.0, 0.0), UP).unwrap();
    let k = |y| match u.update(at(0.0, y)) {
        Some(Delta::Scale(f)) => {
            assert!((f.x - f.y).abs() < 1e-6 && (f.y - f.z).abs() < 1e-6);
            f.x
        }
        _ => panic!(),
    };
    assert!((k(0.0) - 1.0).abs() < 1e-5, "no movement, no change");
    assert!(k(0.5) > 2.0 && k(-0.5) < 0.5, "up grows, down shrinks");
}

#[test]
fn a_turned_gizmo_uses_its_own_axes() {
    let g = Gizmo { axes: [Vec3::Y, Vec3::NEG_X, Vec3::Z], ..unit() };
    assert_eq!(g.pick(GizmoMode::Translate, at(0.02, 0.5)), Some(Handle::Axis(0)));
    let d = g.begin(GizmoMode::Translate, Handle::Axis(0), at(0.0, 0.5), UP).unwrap();
    let Some(Delta::Translate(v)) = d.update(at(0.0, 1.5)) else { panic!() };
    assert!((v - Vec3::Y).length() < 1e-5);
}

#[test]
fn degenerate_input_is_refused_not_trusted() {
    let g = unit();
    let nan = Ray { origin: Vec3::NAN, dir: Vec3::NEG_Z };
    for mode in [GizmoMode::Translate, GizmoMode::Rotate, GizmoMode::Scale] {
        assert_eq!(g.pick(mode, nan), None);
        assert_eq!(g.pick(mode, Ray { origin: Vec3::ZERO, dir: Vec3::NAN }), None);
        assert!(g.begin(mode, Handle::Axis(0), nan, UP).is_none());
        assert!(g.begin(mode, Handle::Axis(0), at(0.5, 0.0), Vec3::NAN).is_none());
        assert!(g.begin(mode, Handle::Axis(7), at(0.5, 0.0), UP).is_none(), "no such axis");
    }
    let bad = [Gizmo { size: 0.0, ..g }, Gizmo { size: f32::NAN, ..g }, Gizmo { origin: Vec3::INFINITY, ..g }, Gizmo { axes: [Vec3::ZERO; 3], ..g }];
    for b in bad {
        assert!(!b.is_valid());
        assert_eq!(b.pick(GizmoMode::Translate, at(0.5, 0.0)), None);
        assert!(b.begin(GizmoMode::Translate, Handle::Axis(0), at(0.5, 0.0), UP).is_none());
    }
    // A view straight down an axis cannot drag along it: the ray never leaves the line.
    assert!(g.begin(GizmoMode::Translate, Handle::Axis(2), at(0.0, 0.0), UP).is_none());
    // A plane behind the eye or parallel to the ray is not hit.
    assert_eq!(ray_plane(Ray { origin: Vec3::ZERO, dir: Vec3::Z }, Vec3::new(0.0, 0.0, -5.0), Vec3::Z), None);
    assert_eq!(ray_plane(at(0.0, 0.0), Vec3::ZERO, Vec3::X), None);
    assert!(closest_on_line(Vec3::ZERO, Vec3::Z, at(0.0, 0.0)).is_none());
}

#[test]
fn arrows_and_rings_together_tell_a_move_from_a_turn() {
    let g = unit();
    // Out along an arrow: move that axis.
    assert_eq!(g.pick_both(at(0.7, 0.02)), Some((GizmoMode::Translate, Handle::Axis(0))));
    assert_eq!(g.pick_both(at(0.02, 0.7)), Some((GizmoMode::Translate, Handle::Axis(1))));
    // On the ring about Z (radius 1 in the XY plane), away from every arrow: turn about Z.
    let on_ring = at(std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
    assert_eq!(g.pick_both(on_ring), Some((GizmoMode::Rotate, Handle::Axis(2))));
    // Between the arrows and inside the rings nothing is grabbed, and far away neither.
    assert_eq!(g.pick_both(at(0.5, 0.5)), None);
    assert_eq!(g.pick_both(at(3.0, 3.0)), None);
    // `pick` in this mode names the handle; garbage never grabs anything.
    assert_eq!(g.pick(GizmoMode::Both, at(0.7, 0.02)), Some(Handle::Axis(0)));
    assert_eq!(g.pick_both(Ray { origin: Vec3::NAN, dir: Vec3::NEG_Z }), None);
    // Beginning a drag with the resolved mode works for both kinds.
    let (m, h) = g.pick_both(on_ring).unwrap();
    assert!(g.begin(m, h, on_ring, UP).is_some());
    let (m, h) = g.pick_both(at(0.7, 0.02)).unwrap();
    assert!(g.begin(m, h, at(0.7, 0.02), UP).is_some());
}
