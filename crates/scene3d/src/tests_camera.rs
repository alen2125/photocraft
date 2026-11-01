use glam::Vec3;

use super::*;

fn cam(position: [f32; 3], target: [f32; 3], roll: f32) -> Camera {
    Camera { position, target, roll, ..Camera::default() }
}

fn assert_orthonormal(c: &Camera) {
    let (r, u, f) = c.axes();
    for v in [r, u, f] {
        assert!((v.length() - 1.0).abs() < 1e-4, "{c:?}: {v:?} is not a unit vector");
    }
    assert!(r.dot(u).abs() < 1e-4 && r.dot(f).abs() < 1e-4 && u.dot(f).abs() < 1e-4, "{c:?}: axes are not at right angles");
}

#[test]
fn axes_are_orthonormal_for_any_roll_and_any_view() {
    for roll in [0.0, 15.0, 90.0, 180.0, -135.0, f32::NAN, f32::INFINITY, 1e9] {
        for (p, t) in [([0.0, 0.0, 5.0], [0.0; 3]), ([0.0, 5.0, 0.0], [0.0; 3]), ([0.0, -5.0, 0.0], [0.0; 3]), ([1.0; 3], [1.0; 3]), ([f32::NAN, 0.0, 0.0], [0.0; 3])] {
            assert_orthonormal(&cam(p, t, roll));
        }
    }
}

#[test]
fn level_camera_keeps_the_horizon_and_roll_banks_about_the_view_axis() {
    let level = cam([0.0, 0.0, 5.0], [0.0; 3], 0.0);
    assert!((level.up_vector() - Vec3::Y).length() < 1e-5);
    // Banked right by 90°: the top of the picture points to the world's right (+X when looking down −Z).
    let right = cam([0.0, 0.0, 5.0], [0.0; 3], 90.0);
    assert!((right.up_vector() - Vec3::X).length() < 1e-5, "{:?}", right.up_vector());
    // The view direction is not changed by the bank.
    assert!((right.axes().2 - level.axes().2).length() < 1e-5);
    let upside_down = cam([0.0, 0.0, 5.0], [0.0; 3], 180.0);
    assert!((upside_down.up_vector() + Vec3::Y).length() < 1e-5);
}

#[test]
fn old_files_without_roll_or_views_load_and_new_ones_round_trip() {
    let old = r#"{"version":1,"camera":{"position":[1,2,3],"target":[0,0,0],"fov":50}}"#;
    let s = Scene::from_json(old).unwrap();
    assert_eq!(s.camera.roll, 0.0);
    assert!(s.views.is_empty());
    assert!(!s.to_json().unwrap().contains("views"), "an unused scene stores no views");

    let mut s = Scene::new();
    s.camera.roll = -30.0;
    assert!(s.save_view(2, &s.camera.clone()));
    let back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(s, back);
    assert_eq!(back.view(2).map(|c| c.roll), Some(-30.0));
    assert!(back.view(0).is_none() && back.view(1).is_none());
}

#[test]
fn views_have_nine_slots_and_hostile_files_cannot_add_more() {
    let mut s = Scene::new();
    let c = Camera::default();
    assert!(s.save_view(MAX_VIEWS - 1, &c));
    assert!(!s.save_view(MAX_VIEWS, &c));
    assert!(!s.save_view(usize::MAX, &c));
    assert!(s.view(MAX_VIEWS).is_none() && s.view(usize::MAX).is_none());
    s.clear_view(MAX_VIEWS - 1);
    s.clear_view(usize::MAX);
    assert!(s.views.is_empty(), "clearing the last slot leaves nothing stored");

    let many = format!(r#"{{"views":[{}]}}"#, vec!["null"; 50].join(","));
    assert_eq!(Scene::from_json(&many).unwrap().views.len(), MAX_VIEWS);
}
