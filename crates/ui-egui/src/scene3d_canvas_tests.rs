use photocraft_model3d::testgen::skinned_triangle_glb;
use photocraft_scene3d::Primitive;
use serde_json::json;

use super::*;
use crate::scene3d_ui::tests::wait_for_preview;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app
}

fn open(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, "ui.scene3d.open", json!({})).unwrap();
}

fn down(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    pointer(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
}
fn mv(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
}
fn up(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    pointer(app, ToolEvent::Up { x, y }, mods);
}

const NONE: egui::Modifiers = egui::Modifiers::NONE;

#[test]
fn the_preview_has_the_documents_shape_and_stays_small() {
    assert_eq!(view_size([400, 300]), (400, 300));
    assert_eq!(view_size([1920, 960]), (960, 480));
    let (w, h) = view_size([2480, 3508]);
    assert!(h == 960 && (600..=700).contains(&w), "{w}x{h}");
    assert_eq!(view_size([0, 0]), (8, 8));
    assert!(view_size([1, 100_000]).0 >= 8);
}

#[test]
fn the_mode_needs_a_document_and_remembers_which_one() {
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    assert!(crate::menus::invoke(&mut empty, &ctx, "ui.scene3d.open", json!({})).is_err());
    assert!(!empty.scene3d.open);
    let mut app = app();
    open(&mut app);
    assert!(app.scene3d.open && app.scene3d.owner.is_some());
    assert_eq!(app.scene3d.doc_size, [400, 300]);
}

#[test]
fn switching_or_closing_the_document_ends_the_mode_but_keeps_the_scene() {
    let mut app = app();
    open(&mut app);
    app.scene3d.add_primitive(Primitive::cube());
    let nodes = app.scene3d.scene.nodes.len();
    let ctx = egui::Context::default();
    // Another document becomes active: the mode closes.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    frame(&mut app, &ctx);
    assert!(!app.scene3d.open && app.scene3d.preview.is_none());
    assert_eq!(app.scene3d.scene.nodes.len(), nodes, "the scene is still there");
    // Opened again on this document, the scene is as it was and the size follows the document.
    open(&mut app);
    assert_eq!(app.scene3d.doc_size, [100, 100]);
    assert_eq!(app.scene3d.scene.nodes.len(), nodes);
}

#[test]
fn the_preview_is_rendered_in_the_background_over_the_documents_shape() {
    let mut app = app();
    open(&mut app);
    app.scene3d.add_primitive(Primitive::cube());
    let ctx = egui::Context::default();
    wait_for_preview(&mut app, &ctx);
    let p = app.scene3d.preview.as_ref().expect("a preview");
    // Coarse first (half size) while the scene was just touched, then full once it settles.
    assert_eq!(p.texture.size()[0] as f32 / p.texture.size()[1] as f32, 400.0 / 300.0);
}

#[test]
fn dragging_empty_space_orbits_and_a_click_selects() {
    let mut app = app();
    open(&mut app);
    let cube = {
        app.scene3d.add_primitive(Primitive::cube());
        app.scene3d.selected.unwrap()
    };
    app.scene3d.selected = None;
    let yaw = app.scene3d.orbit.yaw;
    // A drag that starts on empty space turns the camera.
    down(&mut app, 10.0, 10.0, NONE);
    mv(&mut app, 60.0, 10.0, NONE);
    up(&mut app, 60.0, 10.0, NONE);
    assert!(app.scene3d.orbit.yaw < yaw, "dragging right turns the view");
    assert_eq!(app.scene3d.selected, None, "a drag is not a click");
    // A click on the cube, with the camera aimed at it, selects it.
    app.scene3d.scene.camera.position = [0.0, 0.5, 4.0];
    app.scene3d.scene.camera.target = [0.0, 0.5, 0.0];
    app.scene3d.show_bones = false;
    let (cx, cy) = (200.0, 150.0);
    down(&mut app, cx, cy, NONE);
    up(&mut app, cx, cy, NONE);
    assert_eq!(app.scene3d.selected, Some(cube));
    // A click on nothing deselects.
    down(&mut app, 3.0, 3.0, NONE);
    up(&mut app, 3.0, 3.0, NONE);
    assert_eq!(app.scene3d.selected, None);
}

#[test]
fn shift_pans_and_ctrl_dollies_the_camera() {
    let mut app = app();
    open(&mut app);
    let (target, distance) = (app.scene3d.orbit.target, app.scene3d.orbit.distance);
    let shift = egui::Modifiers { shift: true, ..NONE };
    down(&mut app, 100.0, 100.0, shift);
    mv(&mut app, 160.0, 120.0, shift);
    up(&mut app, 160.0, 120.0, shift);
    assert_ne!(app.scene3d.orbit.target, target);
    assert_eq!(app.scene3d.orbit.distance, distance);
    let ctrl = egui::Modifiers { command: true, ctrl: true, ..NONE };
    down(&mut app, 100.0, 100.0, ctrl);
    mv(&mut app, 100.0, 60.0, ctrl);
    up(&mut app, 100.0, 60.0, ctrl);
    assert!(app.scene3d.orbit.distance < distance, "dragging up moves in");
    // The camera really moved with its orbit.
    let c = &app.scene3d.scene.camera;
    let d = photocraft_scene3d::glam::Vec3::from(c.position).distance(photocraft_scene3d::glam::Vec3::from(c.target));
    assert!((d - app.scene3d.orbit.distance).abs() < 1e-3);
}

fn with_model(app: &mut PhotocraftApp) -> photocraft_scene3d::NodeId {
    app.scene3d.import_model("hero.vrm", &skinned_triangle_glb(false, 2)).unwrap();
    app.scene3d.scene.nodes.iter().find(|n| n.name == "spine").unwrap().id
}

/// The document point where a bone's dot is drawn.
fn dot(app: &PhotocraftApp, bone: photocraft_scene3d::NodeId) -> (f64, f64) {
    let ed = &app.scene3d;
    let view = view_size(ed.doc_size);
    let (_, _, (x, y)) = *bone_screen(&ed.scene, view).iter().find(|b| b.0 == bone).unwrap();
    (f64::from(x) / f64::from(view.0) * f64::from(ed.doc_size[0]), f64::from(y) / f64::from(view.1) * f64::from(ed.doc_size[1]))
}

#[test]
fn pressing_a_bone_dot_poses_it_instead_of_turning_the_camera() {
    let mut app = app();
    open(&mut app);
    let spine = with_model(&mut app);
    let before = app.scene3d.scene.bone_positions().iter().find(|b| b.0 == spine).unwrap().2;
    let yaw = app.scene3d.orbit.yaw;
    let (x, y) = dot(&app, spine);
    down(&mut app, x, y, NONE);
    assert_eq!(app.scene3d.selected, Some(spine));
    mv(&mut app, x + 40.0, y, NONE);
    mv(&mut app, x + 60.0, y, NONE);
    up(&mut app, x + 60.0, y, NONE);
    let after = app.scene3d.scene.bone_positions().iter().find(|b| b.0 == spine).unwrap().2;
    assert!(after.distance(before) > 0.05, "the bone moved: {before:?} -> {after:?}");
    assert_eq!(app.scene3d.orbit.yaw, yaw, "the camera did not");
    // Alt always means the camera, even on a bone.
    let alt = egui::Modifiers { alt: true, ..NONE };
    let (x, y) = dot(&app, spine);
    down(&mut app, x, y, alt);
    mv(&mut app, x + 50.0, y, alt);
    up(&mut app, x + 50.0, y, alt);
    assert_ne!(app.scene3d.orbit.yaw, yaw);
}

#[test]
fn pointer_events_without_a_press_are_ignored() {
    let mut app = app();
    open(&mut app);
    mv(&mut app, 50.0, 50.0, NONE);
    up(&mut app, 50.0, 50.0, NONE);
    assert!(app.scene3d.pointer.is_none() && app.scene3d.selected.is_none());
}

#[test]
fn enter_bakes_to_a_layer_at_the_documents_origin_and_esc_just_closes() {
    let mut app = app();
    open(&mut app);
    app.scene3d.add_primitive(Primitive::cube());
    app.scene3d.scene.camera.position = [0.0, 0.5, 4.0];
    app.scene3d.scene.camera.target = [0.0, 0.5, 0.0];
    let layers = app.session.active().unwrap().doc.layer_count();
    let key = |key| egui::RawInput {
        events: vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: NONE }],
        ..Default::default()
    };
    let ctx = egui::Context::default();
    // Esc: no layer, the scene stays for next time.
    let mut used = false;
    let mut out = ctx.run_ui(key(egui::Key::Escape), |ui| used = keys(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert!(used && !app.scene3d.open);
    assert_eq!(app.session.active().unwrap().doc.layer_count(), layers);
    assert!(app.scene3d.scene.nodes.len() >= 2);
    // Enter: a pixel layer the size of the document, and the mode closes.
    open(&mut app);
    let mut out = ctx.run_ui(key(egui::Key::Enter), |ui| used = keys(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert!(used && !app.scene3d.open && app.scene3d.status.is_empty(), "{}", app.scene3d.status);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer_count(), layers + 1);
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let alpha = |x, y| layer.surface().unwrap().rgba(x, y)[3];
    assert!(alpha(200, 150) > 0.9, "the cube is where the canvas showed it");
    assert!(alpha(2, 2) < 0.01, "the sky is clear");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer_count(), layers, "one undo step");
}

#[test]
fn a_failed_bake_keeps_the_mode_open_and_says_why() {
    let mut app = app();
    open(&mut app);
    // A model whose file cannot be found again.
    app.scene3d.scene.assets.push(photocraft_scene3d::Asset { id: 5, name: "gone.vrm".into(), path: Some("/no/such/gone.vrm".into()) });
    let ctx = egui::Context::default();
    bake(&mut app, &ctx);
    assert!(app.scene3d.open, "the mode stays open so nothing is lost");
    assert!(app.scene3d.status.contains("gone.vrm"), "{}", app.scene3d.status);
}

#[test]
fn smoothing_is_lowered_for_big_documents_to_fit_the_renderers_budget() {
    assert_eq!(limited_ssaa(1000, 1000, 4), 4);
    assert_eq!(limited_ssaa(2480, 3508, 2), 1, "8.7 MP at 2x2 is over the limit");
    assert_eq!(limited_ssaa(4000, 4000, 4), 1);
    assert_eq!(limited_ssaa(4000, 2000, 2), 2, "32 MP of samples fits");
    assert_eq!(limited_ssaa(5000, 2000, 2), 1, "40 MP does not");
    assert_eq!(limited_ssaa(100, 100, 0), 1);
}

#[test]
fn the_picture_and_skeleton_draw_on_the_canvas_without_trouble() {
    let mut app = app();
    open(&mut app);
    with_model(&mut app);
    let ctx = egui::Context::default();
    wait_for_preview(&mut app, &ctx);
    let xf = ViewXform { rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)), zoom: 1.5, center: [200.0, 150.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let painter = ui.painter().clone();
        draw_content(&app, &painter, &xf);
        draw_overlay(&app, &painter, &xf);
    });
    out.textures_delta.clear();
    // Hidden skeleton, no selection, no preview: still nothing to trip over.
    app.scene3d.show_bones = false;
    app.scene3d.selected = None;
    app.scene3d.preview = None;
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let painter = ui.painter().clone();
        draw_content(&app, &painter, &xf);
        draw_overlay(&app, &painter, &xf);
    });
    out.textures_delta.clear();
}
