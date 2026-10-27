use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::{GizmoMode, Light, LightKind, Node, NodeId, NodeKind, Primitive};
use serde_json::json;

use super::*;

const NONE: egui::Modifiers = egui::Modifiers::NONE;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app
}

/// A sphere ("face") at the origin and a spot light linked to it from the front, selected.
fn face_and_sun(app: &mut PhotocraftApp) -> (NodeId, NodeId) {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, "ui.scene3d.open", json!({})).unwrap();
    let ed = &mut app.scene3d;
    ed.scene.nodes.clear();
    ed.scene.camera.position = [0.0, 0.0, 12.0];
    ed.scene.camera.target = [0.0; 3];
    ed.orbit = crate::scene3d_ui::Orbit::from_camera(&ed.scene.camera);
    ed.show_bones = false;
    let face = ed.scene.add_primitive(Primitive::sphere(), photocraft_scene3d::Transform::default()).unwrap();
    let sun = ed.scene.add(Node { kind: NodeKind::Light(Light { kind: LightKind::Spot { angle: 40.0 }, ..Light::default() }), ..Node::default() }).unwrap();
    ed.scene.node_mut(sun).unwrap().transform.translation = [0.0, 0.0, 3.0];
    ed.scene.link_to(sun, face).unwrap();
    ed.selected = Some(sun);
    (face, sun)
}

fn doc_of(app: &PhotocraftApp, p: Vec3) -> (f64, f64) {
    let ed = &app.scene3d;
    let view = view_size(ed.doc_size);
    let (x, y) = photocraft_render3d::project_points(&ed.scene, view.0, view.1, &[p])[0].unwrap();
    (f64::from(x) / f64::from(view.0) * f64::from(ed.doc_size[0]), f64::from(y) / f64::from(view.1) * f64::from(ed.doc_size[1]))
}

fn drag(app: &mut PhotocraftApp, from: Vec3, to: Vec3) {
    let (a, b) = (doc_of(app, from), doc_of(app, to));
    pointer(app, ToolEvent::Down { x: a.0, y: a.1, pressure: 1.0 }, NONE);
    pointer(app, ToolEvent::Move { x: (a.0 + b.0) / 2.0, y: (a.1 + b.1) / 2.0, pressure: 1.0 }, NONE);
    pointer(app, ToolEvent::Move { x: b.0, y: b.1, pressure: 1.0 }, NONE);
    pointer(app, ToolEvent::Up { x: b.0, y: b.1 }, NONE);
}

fn pos(app: &PhotocraftApp, id: NodeId) -> Vec3 {
    app.scene3d.scene.world_matrix(id).unwrap().transform_point3(Vec3::ZERO)
}

#[test]
fn moving_a_linked_light_changes_the_link_and_the_light_keeps_aiming() {
    let mut app = app();
    let (face, sun) = face_and_sun(&mut app);
    let before = app.scene3d.scene.node(sun).unwrap().link.clone().unwrap();
    assert!((before.distance - 3.0).abs() < 1e-3);
    let g = gizmo_for(&app.scene3d).unwrap();
    // Drag along X: the light slides sideways, so the angle round the face changes.
    drag(&mut app, g.origin + Vec3::X * g.size * 0.5, g.origin + Vec3::X * g.size * 1.5);
    let after = app.scene3d.scene.node(sun).unwrap().link.clone().unwrap();
    assert!((after.yaw - before.yaw).abs() > 3.0, "{before:?} -> {after:?}");
    assert_eq!(after.target, face);
    assert!(app.scene3d.scene.node(sun).unwrap().transform.translation == [0.0, 0.0, 3.0], "its own transform is not what moved");
    // Still aimed at the face.
    let w = app.scene3d.scene.world_matrix(sun).unwrap();
    let aim = w.transform_vector3(Vec3::NEG_Z).normalize();
    assert!((aim - (-pos(&app, sun)).normalize()).length() < 1e-3);
}

#[test]
fn turning_a_linked_light_swings_it_round_the_target_and_scaling_changes_the_distance() {
    let mut app = app();
    let (face, sun) = face_and_sun(&mut app);
    // The light stands to the right of the face, so a turn about the axis towards the viewer
    // carries it round.
    app.scene3d.scene.unlink(sun).unwrap();
    app.scene3d.scene.node_mut(sun).unwrap().transform.translation = [3.0, 0.0, 0.0];
    app.scene3d.scene.link_to(sun, face).unwrap();
    app.scene3d.gizmo_mode = GizmoMode::Rotate;
    let g = gizmo_for(&app.scene3d).unwrap();
    let before = pos(&app, sun);
    drag(&mut app, g.origin + Vec3::X * g.size, g.origin + Vec3::Y * g.size);
    let after = pos(&app, sun);
    assert!(after.distance(Vec3::new(0.0, 3.0, 0.0)) < 0.35, "a quarter turn about the face carries it from the right to the top: {before:?} -> {after:?}");
    assert!((after.length() - 3.0).abs() < 0.01, "at the same distance from the face: {}", after.length());

    app.scene3d.gizmo_mode = GizmoMode::Scale;
    let g = gizmo_for(&app.scene3d).unwrap();
    drag(&mut app, g.origin + Vec3::X * g.size * 0.95, g.origin + Vec3::X * g.size * 1.9);
    let d = app.scene3d.scene.node(sun).unwrap().link.as_ref().unwrap().distance;
    assert!(d > 5.0, "pulled out to about twice the distance: {d}");
}

#[test]
fn the_inspector_links_and_unlinks_without_moving_the_node() {
    let mut app = app();
    let (face, sun) = face_and_sun(&mut app);
    app.scene3d.scene.unlink(sun).unwrap();
    let placed = pos(&app, sun);
    // What the inspector does when a target is picked, and when "(none)" is.
    app.scene3d.scene.link_to(sun, face).unwrap();
    assert!((pos(&app, sun) - placed).length() < 1e-4);
    app.scene3d.scene.unlink(sun).unwrap();
    assert!((pos(&app, sun) - placed).length() < 1e-4);
    // Drawn with a selected linked node, the window shows the link controls without trouble.
    app.scene3d.scene.link_to(sun, face).unwrap();
    app.scene3d.selected = Some(sun);
    let ctx = egui::Context::default();
    crate::scene3d_ui::tests::wait_for_preview(&mut app, &ctx);
    assert!(app.scene3d.status.is_empty(), "{}", app.scene3d.status);
}

#[test]
fn links_are_drawn_on_the_canvas() {
    let mut app = app();
    face_and_sun(&mut app);
    let ctx = egui::Context::default();
    let xf = ViewXform { rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)), zoom: 1.5, center: [200.0, 150.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let painter = ui.painter().clone();
        draw_overlay(&app, &painter, &xf);
    });
    out.textures_delta.clear();
    // The dashes are shapes in the frame: a link adds more than a plain selection does.
    assert!(!out.shapes.is_empty());
}

#[test]
fn the_factor_that_scales_a_link_is_the_one_that_changed() {
    assert_eq!(farthest_from_one(Vec3::new(2.0, 1.0, 1.0)), 2.0);
    assert_eq!(farthest_from_one(Vec3::new(1.0, 0.25, 1.0)), 0.25);
    assert_eq!(farthest_from_one(Vec3::splat(1.5)), 1.5);
    assert_eq!(farthest_from_one(Vec3::ONE), 1.0);
}
