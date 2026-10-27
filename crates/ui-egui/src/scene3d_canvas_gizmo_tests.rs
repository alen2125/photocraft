use photocraft_model3d::testgen::skinned_triangle_glb;
use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::{GizmoMode, NodeId, Primitive};
use serde_json::json;

use super::*;

const NONE: egui::Modifiers = egui::Modifiers::NONE;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app
}

fn open(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, "ui.scene3d.open", json!({})).unwrap();
}

fn down(app: &mut PhotocraftApp, x: f64, y: f64) {
    pointer(app, ToolEvent::Down { x, y, pressure: 1.0 }, NONE);
}
fn mv(app: &mut PhotocraftApp, x: f64, y: f64) {
    pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }, NONE);
}
fn up(app: &mut PhotocraftApp, x: f64, y: f64) {
    pointer(app, ToolEvent::Up { x, y }, NONE);
}

/// A cube at the origin, selected, with the camera 6 units away looking straight at it.
fn cube_scene(app: &mut PhotocraftApp) -> NodeId {
    open(app);
    app.scene3d.add_primitive(Primitive::cube());
    let id = app.scene3d.selected.unwrap();
    app.scene3d.scene.node_mut(id).unwrap().transform.translation = [0.0; 3];
    app.scene3d.scene.camera.position = [0.0, 0.0, 6.0];
    app.scene3d.scene.camera.target = [0.0; 3];
    app.scene3d.orbit = crate::scene3d_ui::Orbit::from_camera(&app.scene3d.scene.camera);
    app.scene3d.show_bones = false;
    id
}

/// The document point where a world point is seen.
fn doc_of(app: &PhotocraftApp, p: Vec3) -> (f64, f64) {
    let ed = &app.scene3d;
    let view = view_size(ed.doc_size);
    let (x, y) = photocraft_render3d::project_points(&ed.scene, view.0, view.1, &[p])[0].unwrap();
    (f64::from(x) / f64::from(view.0) * f64::from(ed.doc_size[0]), f64::from(y) / f64::from(view.1) * f64::from(ed.doc_size[1]))
}

fn drag(app: &mut PhotocraftApp, from: Vec3, to: Vec3) {
    let (a, b) = (doc_of(app, from), doc_of(app, to));
    down(app, a.0, a.1);
    mv(app, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    mv(app, b.0, b.1);
    up(app, b.0, b.1);
}

fn pos(app: &PhotocraftApp, id: NodeId) -> Vec3 {
    app.scene3d.scene.bone_positions().iter().find(|b| b.0 == id).unwrap().2
}

#[test]
fn the_gizmo_belongs_to_the_selection_and_keeps_its_screen_size() {
    let mut app = app();
    let id = cube_scene(&mut app);
    let near = gizmo_for(&app.scene3d).unwrap();
    app.scene3d.scene.camera.position = [0.0, 0.0, 12.0];
    let far = gizmo_for(&app.scene3d).unwrap();
    assert!((far.size / near.size - 2.0).abs() < 1e-3, "twice as far, twice as big in the world");
    app.scene3d.selected = None;
    assert!(gizmo_for(&app.scene3d).is_none());
    // Local axes follow the node's turn; world axes do not.
    app.scene3d.selected = Some(id);
    app.scene3d.scene.node_mut(id).unwrap().transform.rotation = [0.0, 0.0, 90.0];
    assert!((gizmo_for(&app.scene3d).unwrap().axes[0] - Vec3::X).length() < 1e-4);
    app.scene3d.gizmo_local = true;
    assert!((gizmo_for(&app.scene3d).unwrap().axes[0] - Vec3::Y).length() < 1e-4);
}

#[test]
fn dragging_an_arrow_moves_the_object_along_that_axis_and_returning_restores_it() {
    let mut app = app();
    let id = cube_scene(&mut app);
    let g = gizmo_for(&app.scene3d).unwrap();
    drag(&mut app, g.origin + Vec3::X * g.size * 0.5, g.origin + Vec3::X * g.size * 1.5);
    let t = app.scene3d.scene.node(id).unwrap().transform.translation;
    assert!((t[0] - g.size).abs() < 0.12, "moved one gizmo-length right: {t:?} (size {})", g.size);
    assert!(t[1].abs() < 0.05 && t[2].abs() < 0.05, "and only along X: {t:?}");
    assert!(app.scene3d.pointer.is_none());
    // Dragging out and back to the start leaves it where it was.
    let before = app.scene3d.scene.node(id).unwrap().transform.clone();
    let g = gizmo_for(&app.scene3d).unwrap();
    let (a, far) = (doc_of(&app, g.origin + Vec3::X * g.size * 0.5), doc_of(&app, g.origin + Vec3::X * g.size * 2.5));
    down(&mut app, a.0, a.1);
    mv(&mut app, far.0, far.1);
    assert_ne!(app.scene3d.scene.node(id).unwrap().transform, before);
    mv(&mut app, a.0, a.1);
    up(&mut app, a.0, a.1);
    let after = app.scene3d.scene.node(id).unwrap().transform.clone();
    assert!(after.translation.iter().zip(before.translation).all(|(x, y)| (x - y).abs() < 1e-3), "{after:?} vs {before:?}");
}

#[test]
fn dragging_a_ring_turns_the_object_and_a_tip_scales_it() {
    let mut app = app();
    let id = cube_scene(&mut app);
    app.scene3d.gizmo_mode = GizmoMode::Rotate;
    let g = gizmo_for(&app.scene3d).unwrap();
    drag(&mut app, g.origin + Vec3::X * g.size, g.origin + Vec3::Y * g.size);
    let r = app.scene3d.scene.node(id).unwrap().transform.rotation;
    assert!((r[2] - 90.0).abs() < 8.0 && r[0].abs() < 5.0 && r[1].abs() < 5.0, "a quarter turn about Z: {r:?}");

    app.scene3d.scene.node_mut(id).unwrap().transform.rotation = [0.0; 3];
    app.scene3d.gizmo_mode = GizmoMode::Scale;
    let g = gizmo_for(&app.scene3d).unwrap();
    drag(&mut app, g.origin + Vec3::X * g.size * 0.95, g.origin + Vec3::X * g.size * 1.9);
    let s = app.scene3d.scene.node(id).unwrap().transform.scale;
    assert!((s[0] - 2.0).abs() < 0.25 && (s[1] - 1.0).abs() < 1e-3 && (s[2] - 1.0).abs() < 1e-3, "doubled on X only: {s:?}");
}

#[test]
fn a_press_that_misses_the_gizmo_still_turns_the_camera() {
    let mut app = app();
    cube_scene(&mut app);
    let yaw = app.scene3d.orbit.yaw;
    down(&mut app, 20.0, 20.0);
    mv(&mut app, 80.0, 20.0);
    up(&mut app, 80.0, 20.0);
    assert_ne!(app.scene3d.orbit.yaw, yaw);
}

#[test]
fn a_limb_bone_dragged_by_its_gizmo_follows_by_turning_the_bones_above() {
    let mut app = app();
    open(&mut app);
    app.scene3d.import_model("hero.vrm", &skinned_triangle_glb(false, 2)).unwrap();
    let find = |app: &PhotocraftApp, name: &str| app.scene3d.scene.nodes.iter().find(|n| n.name == name).unwrap().id;
    let (hips, spine) = (find(&app, "hips"), find(&app, "spine"));
    app.scene3d.selected = Some(spine);
    app.scene3d.show_bones = false;
    let (hips_before, spine_before) = (pos(&app, hips), pos(&app, spine));
    let g = gizmo_for(&app.scene3d).unwrap();
    drag(&mut app, g.origin + Vec3::X * g.size * 0.5, g.origin + Vec3::X * g.size * 1.5);
    let (hips_after, spine_after) = (pos(&app, hips), pos(&app, spine));
    assert!(spine_after.distance(spine_before) > 0.05, "the bone moved");
    assert!(hips_after.distance(hips_before) < 1e-4, "the hips stayed");
    assert!((spine_after.distance(hips_after) - spine_before.distance(hips_before)).abs() < 1e-3, "the bone kept its length");
    // Bones have no scale gizmo.
    app.scene3d.gizmo_mode = GizmoMode::Scale;
    assert!(gizmo_for(&app.scene3d).is_none());
}

fn key(ctx: &egui::Context, app: &mut PhotocraftApp, key: egui::Key, modifiers: egui::Modifiers) -> bool {
    let input = egui::RawInput { events: vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }], ..Default::default() };
    let mut used = false;
    let mut out = ctx.run_ui(input, |ui| used = keys(app, ui.ctx()));
    out.textures_delta.clear();
    used
}

#[test]
fn w_e_r_pick_the_tool_and_ctrl_z_steps_back_through_scene_edits_but_not_the_camera() {
    let mut app = app();
    let id = cube_scene(&mut app);
    let ctx = egui::Context::default();
    for (k, mode) in [(egui::Key::E, GizmoMode::Rotate), (egui::Key::R, GizmoMode::Scale), (egui::Key::W, GizmoMode::Translate)] {
        assert!(key(&ctx, &mut app, k, NONE));
        assert_eq!(app.scene3d.gizmo_mode, mode);
    }
    frame(&mut app, &ctx);
    let start = app.scene3d.scene.node(id).unwrap().transform.translation;
    // Orbiting is not an edit.
    down(&mut app, 20.0, 20.0);
    mv(&mut app, 90.0, 20.0);
    up(&mut app, 90.0, 20.0);
    frame(&mut app, &ctx);
    assert!(app.scene3d.history.is_empty(), "the camera is not in the history");
    // A gizmo drag is one step, however many moves it had.
    let g = gizmo_for(&app.scene3d).unwrap();
    let (a, b) = (doc_of(&app, g.origin + Vec3::X * g.size * 0.5), doc_of(&app, g.origin + Vec3::X * g.size * 1.5));
    down(&mut app, a.0, a.1);
    for i in 1..=5 {
        let k = f64::from(i) / 5.0;
        mv(&mut app, a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k);
    }
    up(&mut app, b.0, b.1);
    frame(&mut app, &ctx);
    assert_eq!(app.scene3d.history.len(), 1);
    let moved = app.scene3d.scene.node(id).unwrap().transform.translation;
    assert_ne!(moved, start);
    let cam = app.scene3d.scene.camera.position;
    let ctrl = egui::Modifiers { command: true, ctrl: true, ..NONE };
    assert!(key(&ctx, &mut app, egui::Key::Z, ctrl));
    assert_eq!(app.scene3d.scene.node(id).unwrap().transform.translation, start);
    assert_eq!(app.scene3d.scene.camera.position, cam, "undo leaves the camera alone");
    let redo = egui::Modifiers { shift: true, ..ctrl };
    assert!(key(&ctx, &mut app, egui::Key::Z, redo));
    assert_eq!(app.scene3d.scene.node(id).unwrap().transform.translation, moved);
    // Nothing left to redo, and a new edit clears the redo stack.
    assert!(!step_history(&mut app.scene3d, false));
    assert!(key(&ctx, &mut app, egui::Key::Z, ctrl));
    app.scene3d.add_primitive(Primitive::sphere());
    frame(&mut app, &ctx);
    assert!(app.scene3d.future.is_empty());
}

#[test]
fn undo_with_nothing_to_undo_is_harmless_and_deleted_selections_are_dropped() {
    let mut app = app();
    let id = cube_scene(&mut app);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    assert!(!step_history(&mut app.scene3d, true));
    app.scene3d.add_primitive(Primitive::sphere());
    frame(&mut app, &ctx);
    let sphere = app.scene3d.selected.unwrap();
    assert!(step_history(&mut app.scene3d, true), "undo the add");
    assert!(app.scene3d.scene.node(sphere).is_none() && app.scene3d.selected.is_none());
    assert!(app.scene3d.scene.node(id).is_some());
}
