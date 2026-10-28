use photocraft_model3d::testgen::skinned_triangle_glb;

use super::tests::wait_for_preview;
use super::*;

fn bone(ed: &Scene3dEditor, name: &str) -> NodeId {
    ed.scene.nodes.iter().find(|n| n.name == name).unwrap().id
}

fn bone_pos(ed: &Scene3dEditor, id: NodeId) -> Vec3 {
    ed.scene.bone_positions().iter().find(|b| b.0 == id).unwrap().2
}

fn with_model() -> Scene3dEditor {
    let mut ed = Scene3dEditor::default();
    ed.import_model("hero.vrm", &skinned_triangle_glb(false, 2)).unwrap();
    ed
}

#[test]
fn importing_a_model_adds_bones_selects_it_and_frames_the_camera() {
    let ed = with_model();
    assert!(ed.scene.nodes.iter().any(|n| matches!(n.kind, NodeKind::Bone) && n.name == "spine"));
    assert_eq!(ed.scene.assets.len(), 1);
    // A bare name (the browser) has no path to reload from; a desktop path is kept.
    assert_eq!(ed.scene.assets[0].path, None);
    assert!(ed.selected.is_some());
    // The skeleton spans one unit; the camera now looks at its middle from a fitting distance.
    assert!((ed.scene.camera.target[1] - 0.5).abs() < 1e-3, "{:?}", ed.scene.camera.target);
    assert!(ed.orbit.distance > 1.0 && ed.orbit.distance < 5.0);
}

#[test]
fn an_unreadable_model_reports_and_changes_nothing() {
    let mut ed = Scene3dEditor::default();
    let before = ed.scene.nodes.len();
    assert!(ed.import_model("junk.glb", b"definitely not a model").unwrap_err().contains("junk.glb"));
    assert_eq!(ed.scene.nodes.len(), before);
    assert!(ed.scene.assets.is_empty());
}

#[test]
fn the_nearest_bone_within_the_radius_is_grabbed() {
    let bones = vec![(1, None, (10.0, 10.0)), (2, Some(1), (20.0, 10.0)), (3, Some(2), (100.0, 100.0))];
    assert_eq!(nearest_bone(&bones, 11.0, 10.0, 12.0), Some(1));
    assert_eq!(nearest_bone(&bones, 18.0, 10.0, 12.0), Some(2), "closer of two in range");
    assert_eq!(nearest_bone(&bones, 60.0, 60.0, 12.0), None);
    assert_eq!(nearest_bone(&[], 0.0, 0.0, 12.0), None);
}

#[test]
fn dragging_a_bone_in_ik_mode_follows_the_pointer_on_the_view_plane() {
    let mut ed = with_model();
    ed.scene.camera.position = [0.0, 0.5, 4.0];
    ed.scene.camera.target = [0.0, 0.5, 0.0];
    let size = (200, 200);
    let spine = bone(&ed, "spine");
    let start = bone_pos(&ed, spine);
    let (px, py) = photocraft_render3d::project(&ed.scene, size.0, size.1, spine).unwrap();
    ed.pose_mode = PoseMode::Ik;
    let drag = BoneDrag { id: spine, plane_point: start };
    // The spine hangs from the hips, which are fixed, so it can swing on a circle round them.
    drag_bone(&mut ed, drag, size, (px + 30.0, py), (30.0, 0.0));
    let now = bone_pos(&ed, spine);
    assert!(now.x > start.x + 0.1, "moved right: {start:?} -> {now:?}");
    let hips = bone_pos(&ed, bone(&ed, "hips"));
    assert!((now.distance(hips) - start.distance(hips)).abs() < 1e-3, "bone length is kept");
}

#[test]
fn dragging_the_top_bone_slides_it() {
    let mut ed = with_model();
    ed.scene.camera.position = [0.0, 0.5, 4.0];
    ed.scene.camera.target = [0.0, 0.5, 0.0];
    let size = (200, 200);
    let hips = bone(&ed, "hips");
    let start = bone_pos(&ed, hips);
    let (px, py) = photocraft_render3d::project(&ed.scene, size.0, size.1, hips).unwrap();
    drag_bone(&mut ed, BoneDrag { id: hips, plane_point: start }, size, (px + 40.0, py), (40.0, 0.0));
    assert!(bone_pos(&ed, hips).x > start.x + 0.1);
}

#[test]
fn dragging_in_rotate_mode_turns_the_bone() {
    let mut ed = with_model();
    ed.pose_mode = PoseMode::Rotate;
    let spine = bone(&ed, "spine");
    let before = ed.scene.node(spine).unwrap().transform.rotation;
    drag_bone(&mut ed, BoneDrag { id: spine, plane_point: Vec3::ZERO }, (100, 100), (50.0, 50.0), (20.0, 0.0));
    assert_ne!(ed.scene.node(spine).unwrap().transform.rotation, before);
    assert!(ed.status.is_empty());
}

#[test]
fn a_drag_with_no_usable_pointer_ray_is_ignored() {
    let mut ed = with_model();
    let spine = bone(&ed, "spine");
    let before = ed.scene.clone();
    ed.pose_mode = PoseMode::Ik;
    // A camera looking along the view plane's own normal never meets it from the side; a NaN
    // pointer has no ray at all.
    drag_bone(&mut ed, BoneDrag { id: spine, plane_point: Vec3::ZERO }, (100, 100), (f32::NAN, 5.0), (0.0, 0.0));
    assert_eq!(ed.scene.nodes, before.nodes);
    // A bone that was deleted mid-drag is an error message, not a crash.
    drag_bone(&mut ed, BoneDrag { id: 9999, plane_point: Vec3::ZERO }, (100, 100), (50.0, 50.0), (3.0, 3.0));
}

#[test]
fn camera_axes_are_orthonormal_even_looking_straight_down() {
    for (pos, target) in [([0.0, 0.0, 5.0], [0.0; 3]), ([0.0, 5.0, 0.0], [0.0; 3]), ([1.0; 3], [1.0; 3])] {
        let c = Camera { position: pos, target, ..Camera::default() };
        let (r, u, f) = camera_axes(&c);
        assert!((r.length() - 1.0).abs() < 1e-4 && (u.length() - 1.0).abs() < 1e-4 && (f.length() - 1.0).abs() < 1e-4);
        assert!(r.dot(u).abs() < 1e-4 && r.dot(f).abs() < 1e-4 && u.dot(f).abs() < 1e-4);
    }
}

#[test]
fn saved_scenes_reload_their_models_from_the_paths_they_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hero.vrm");
    std::fs::write(&file, skinned_triangle_glb(false, 2)).unwrap();
    let mut ed = Scene3dEditor::default();
    let model = photocraft_scene3d::model3d::load("hero.vrm", &skinned_triangle_glb(false, 2)).unwrap();
    ed.scene.add_model(Arc::new(model), Some(file.to_str().unwrap().to_string())).unwrap();
    let mut back = Scene::from_json(&ed.scene.to_json().unwrap()).unwrap();
    assert_eq!(back.missing_assets().len(), 1);
    assert!(load_missing_assets(&mut back).is_empty());
    assert!(back.missing_assets().is_empty());
    // A model file that has gone is reported by its path; the scene still opens.
    drop(dir);
    let mut gone = Scene::from_json(&ed.scene.to_json().unwrap()).unwrap();
    let errors = load_missing_assets(&mut gone);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("hero.vrm"));
}

#[test]
fn the_window_draws_with_a_model_and_its_skeleton() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 120, "height": 90})).unwrap();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", serde_json::json!({})).unwrap();
    app.scene3d.import_model("hero.vrm", &skinned_triangle_glb(false, 2)).unwrap();
    wait_for_preview(&mut app, &ctx);
    assert!(app.scene3d.preview.is_some());
    assert!(app.scene3d.status.is_empty(), "{}", app.scene3d.status);
}

#[test]
fn the_inspector_shows_a_bones_three_modes_and_its_anatomical_limit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 120, "height": 90})).unwrap();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", serde_json::json!({})).unwrap();
    app.scene3d.import_model("hero.vrm", &skinned_triangle_glb(false, 2)).unwrap();
    let spine = app.scene3d.scene.nodes.iter().find(|n| n.name == "spine").unwrap().id;
    app.scene3d.selected = Some(spine);
    // A limit on the bone brings its switch into the inspector and the panel.
    app.scene3d.scene.node_mut(spine).unwrap().joint = Some(photocraft_scene3d::JointLimit::default());
    // Holding it records where it is; releasing clears that.
    app.scene3d.scene.set_hold(spine, true).unwrap();
    assert!(app.scene3d.scene.node(spine).unwrap().hold.is_some());
    for _ in 0..3 {
        let mut out = ctx.run_ui(Default::default(), |ui| crate::scene3d_ui::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    assert!(app.scene3d.status.is_empty(), "{}", app.scene3d.status);
    app.scene3d.scene.set_hold(spine, false).unwrap();
    assert!(app.scene3d.scene.node(spine).unwrap().hold.is_none());
}
