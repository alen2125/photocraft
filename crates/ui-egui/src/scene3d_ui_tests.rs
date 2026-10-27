use serde_json::json;

use super::*;

/// Draws frames until the background renderer has produced a picture.
pub(crate) fn wait_for_preview(app: &mut PhotocraftApp, ctx: &egui::Context) {
    for _ in 0..1000 {
        let mut out = ctx.run_ui(Default::default(), |ui| show(app, ui.ctx()));
        // Nothing paints in a test, so the texture deltas are dropped on purpose.
        out.textures_delta.clear();
        if app.scene3d.preview.is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    app
}

#[test]
fn the_window_command_opens_the_editor() {
    let mut app = app();
    assert!(!app.scene3d.open);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", json!({})).unwrap();
    assert!(app.scene3d.open);
    assert!(crate::menus::is_enabled(&app, "ui.scene3d.open"));
}

#[test]
fn orbit_and_camera_round_trip() {
    for (yaw, pitch, dist) in [(0.0, 0.0, 5.0), (35.0, 20.0, 3.0), (-120.0, -45.0, 12.0), (170.0, 80.0, 0.5)] {
        let mut cam = Camera::default();
        let o = Orbit { target: [1.0, 2.0, 3.0], yaw, pitch, distance: dist };
        o.apply(&mut cam);
        let back = Orbit::from_camera(&cam);
        assert!((back.yaw - yaw).abs() < 0.01 && (back.pitch - pitch).abs() < 0.01 && (back.distance - dist).abs() < 1e-3, "{yaw} {pitch} {dist}: {back:?}");
    }
    // A camera sitting on its target falls back to a usable view instead of NaNs.
    let c = Camera { position: [1.0; 3], target: [1.0; 3], ..Camera::default() };
    let o = Orbit::from_camera(&c);
    assert!(o.distance > 0.0 && o.yaw.is_finite() && o.pitch.is_finite());
}

#[test]
fn editing_helpers_keep_the_scene_valid() {
    let mut ed = Scene3dEditor::default();
    let start = ed.scene.nodes.len();
    ed.add_primitive(Primitive::cube());
    ed.add_light(LightKind::Spot { angle: 40.0 });
    ed.add(Node::default());
    assert_eq!(ed.scene.nodes.len(), start + 3);
    let before = ed.rev;
    ed.selected = ed.scene.nodes.iter().find(|n| matches!(n.kind, NodeKind::Primitive { .. })).map(|n| n.id);
    ed.duplicate_selected();
    assert!(ed.rev > before);
    ed.delete_selected();
    assert!(ed.selected.is_none());
    ed.scene.validate().unwrap();
    assert_eq!(tree_order(&ed.scene).len(), ed.scene.nodes.len());
}

#[test]
fn render_to_layer_adds_a_layer_through_the_engine() {
    let mut app = app();
    let ctx = egui::Context::default();
    let layers = app.session.active().unwrap().doc.layer_count();
    app.scene3d.doc_size = [40, 30];
    run_action(&mut app, &ctx, Action::Bake);
    assert!(app.scene3d.status.is_empty(), "{}", app.scene3d.status);
    assert_eq!(app.session.active().unwrap().doc.layer_count(), layers + 1);
}

#[test]
fn render_without_a_document_reports_instead_of_panicking() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    run_action(&mut app, &ctx, Action::Bake);
    assert!(!app.scene3d.status.is_empty());
}

#[test]
fn loading_a_scene_adopts_its_camera_and_clears_the_selection() {
    let mut ed = Scene3dEditor { selected: Some(1), ..Scene3dEditor::default() };
    let mut s = Scene::new();
    s.camera.position = [0.0, 0.0, 9.0];
    s.camera.target = [0.0, 0.0, 0.0];
    ed.load(s, Some("a.pcscene".into()));
    assert_eq!(ed.selected, None);
    assert!((ed.orbit.distance - 9.0).abs() < 1e-4);
    assert_eq!(ed.file_name.as_deref(), Some("a.pcscene"));
}

#[test]
fn the_window_draws_and_previews_a_scene_headlessly() {
    let mut app = app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", json!({})).unwrap();
    wait_for_preview(&mut app, &ctx);
    assert!(app.scene3d.preview.is_some(), "the viewport should have rendered a preview");
    assert!(app.scene3d.open);
}
