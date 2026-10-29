//! Zooming the 3D camera toward the point under the cursor, with no stopping distance: the
//! camera and its target both move toward that point, and once the target is reached the camera
//! keeps gliding forward, so a face or a hand can be filled with the screen.

use photocraft_scene3d::glam::Vec3;

use crate::scene3d_ui::Scene3dEditor;

/// Closest and farthest the camera may sit from its target, in scene units.
pub(crate) const MIN_DISTANCE: f32 = 0.01;
pub(crate) const MAX_DISTANCE: f32 = 500.0;

/// Scales the camera's distance to the point under preview pixel `p` by `factor` (below 1 moves
/// in, above 1 moves out) and keeps that point where it is on the screen.
pub(crate) fn zoom_toward(ed: &mut Scene3dEditor, view: (u32, u32), p: (f32, f32), factor: f32) {
    if !factor.is_finite() || factor <= 0.0 || !p.0.is_finite() || !p.1.is_finite() {
        return;
    }
    let factor = factor.clamp(0.05, 20.0);
    let target = Vec3::from(ed.orbit.target);
    let cam = Vec3::from(ed.scene.camera.position);
    let fwd = (target - cam).normalize_or_zero();
    if fwd == Vec3::ZERO || !target.is_finite() {
        return;
    }
    // The point on the cursor's ray at the depth of the target; the target itself when the ray
    // misses (looking edge-on) or the cursor is off the picture.
    let anchor = photocraft_render3d::pixel_ray(&ed.scene, view.0, view.1, p.0, p.1)
        .and_then(|(origin, dir)| {
            let denom = dir.dot(fwd);
            if denom.abs() < 1e-4 {
                return None;
            }
            let t = (target - origin).dot(fwd) / denom;
            (t > 0.0).then(|| origin + dir * t)
        })
        .filter(|a| a.is_finite())
        .unwrap_or(target);
    let old = ed.orbit.distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
    let new = (old * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
    let applied = new / old;
    let mut moved = anchor + (target - anchor) * applied;
    // Stopped by the minimum distance while zooming in: glide on, the way a person walks forward.
    if factor < 1.0 && applied > factor {
        moved += fwd * (old * (applied - factor));
    }
    if !moved.is_finite() {
        return;
    }
    ed.orbit.target = moved.to_array();
    ed.orbit.distance = new;
    // Close up the near plane has to come in too, or the face would be cut away.
    ed.scene.camera.near = (new * 0.1).clamp(0.005, 0.05);
    ed.scene.camera.far = (new * 4.0).max(200.0);
    ed.orbit.apply(&mut ed.scene.camera);
    ed.touch_keep_world();
    ed.pending_change = true;
}

#[cfg(test)]
mod tests {
    use photocraft_scene3d::glam::Vec3;
    use photocraft_scene3d::{Primitive, Transform};

    use super::*;

    const VIEW: (u32, u32) = (400, 300);

    fn editor() -> Scene3dEditor {
        let mut ed = Scene3dEditor::default();
        ed.scene.add_primitive(Primitive::cube(), Transform::default()).unwrap();
        ed.scene.camera.position = [0.0, 0.0, 5.0];
        ed.scene.camera.target = [0.0, 0.0, 0.0];
        ed.orbit = crate::scene3d_ui::Orbit::from_camera(&ed.scene.camera);
        ed
    }

    fn screen_of(ed: &Scene3dEditor, p: Vec3) -> (f32, f32) {
        photocraft_render3d::project_point(&ed.scene, VIEW.0, VIEW.1, p).unwrap()
    }

    #[test]
    fn the_point_under_the_cursor_stays_under_it_while_zooming() {
        let mut ed = editor();
        let world = Vec3::new(1.0, 0.5, 0.0);
        let at = screen_of(&ed, world);
        for f in [0.5, 0.7, 1.6, 0.8] {
            zoom_toward(&mut ed, VIEW, at, f);
            let now = screen_of(&ed, world);
            assert!((now.0 - at.0).abs() < 1.5 && (now.1 - at.1).abs() < 1.5, "after {f}: {at:?} -> {now:?}");
        }
        assert!(ed.scene.camera.position.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn zooming_in_has_no_stopping_distance_the_camera_keeps_gliding_forward() {
        let mut ed = editor();
        let start = Vec3::from(ed.scene.camera.position);
        let centre = (VIEW.0 as f32 / 2.0, VIEW.1 as f32 / 2.0);
        for _ in 0..120 {
            zoom_toward(&mut ed, VIEW, centre, 0.85);
        }
        assert!((ed.orbit.distance - MIN_DISTANCE).abs() < 1e-4, "the target sits right in front of the camera");
        // The camera passed through the point it started out looking at ...
        let now = Vec3::from(ed.scene.camera.position);
        assert!(start.z - now.z > 5.0, "{start:?} -> {now:?}");
        // ... and every further notch still moves it, however close it is.
        zoom_toward(&mut ed, VIEW, centre, 0.85);
        assert!(Vec3::from(ed.scene.camera.position).z < now.z, "it does not stop");
        assert!(ed.scene.camera.near <= 0.0051, "the near plane came in with it");
    }

    #[test]
    fn zooming_out_is_capped_and_bad_numbers_do_nothing() {
        let mut ed = editor();
        for _ in 0..80 {
            zoom_toward(&mut ed, VIEW, (200.0, 150.0), 1.5);
        }
        assert!((ed.orbit.distance - MAX_DISTANCE).abs() < 1e-2);
        assert!(ed.scene.camera.far >= 4.0 * MAX_DISTANCE - 1.0);
        let before = ed.scene.camera.clone();
        let rev = ed.rev;
        for (f, p) in [(f32::NAN, (1.0, 1.0)), (0.0, (1.0, 1.0)), (-2.0, (1.0, 1.0)), (0.5, (f32::NAN, 1.0)), (f32::INFINITY, (1.0, 1.0))] {
            zoom_toward(&mut ed, VIEW, p, f);
        }
        assert_eq!(ed.scene.camera, before);
        assert_eq!(ed.rev, rev, "nothing changed, nothing to redraw");
    }
}
