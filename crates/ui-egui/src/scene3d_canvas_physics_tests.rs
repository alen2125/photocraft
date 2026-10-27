use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::{NodeId, Primitive, Transform};
use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app
}

/// A floor and a cube held up in the air (pinned), the mode open on that scene.
fn cube_over_floor(app: &mut PhotocraftApp) -> NodeId {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, "ui.scene3d.open", json!({})).unwrap();
    let ed = &mut app.scene3d;
    ed.scene.nodes.clear();
    ed.scene.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    ed.scene.add_primitive(Primitive::cube(), Transform::at(0.0, 5.0, 0.0)).unwrap()
}

fn y(app: &PhotocraftApp, id: NodeId) -> f32 {
    app.scene3d.scene.world_matrix(id).unwrap().transform_point3(Vec3::ZERO).y
}

fn fall(ed: &mut Scene3dEditor, seconds: f32) {
    for _ in 0..(seconds * 60.0) as usize {
        tick(ed, 1.0 / 60.0);
    }
}

#[test]
fn play_needs_something_unpinned_and_says_so() {
    let mut app = app();
    cube_over_floor(&mut app);
    assert!(play(&mut app.scene3d).unwrap_err().contains("Pinned") || play(&mut app.scene3d).is_err());
    assert!(app.scene3d.physics.is_none() && app.scene3d.play_base.is_none());
}

#[test]
fn unpinned_objects_fall_while_pinned_ones_hold() {
    let mut app = app();
    let cube = cube_over_floor(&mut app);
    let held = app.scene3d.scene.add_primitive(Primitive::sphere(), Transform::at(4.0, 5.0, 0.0)).unwrap();
    app.scene3d.scene.node_mut(cube).unwrap().pinned = false;
    play(&mut app.scene3d).unwrap();
    fall(&mut app.scene3d, 3.0);
    assert!((y(&app, cube) - 0.5).abs() < 0.05, "the cube lay down on the floor: {}", y(&app, cube));
    assert_eq!(y(&app, held), 5.0, "the pinned sphere did not move");
}

#[test]
fn stop_keep_is_one_undo_step_and_revert_brings_everything_back() {
    let mut app = app();
    let cube = cube_over_floor(&mut app);
    app.scene3d.scene.node_mut(cube).unwrap().pinned = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    let history = app.scene3d.history.len();
    // Play, fall, keep.
    play(&mut app.scene3d).unwrap();
    fall(&mut app.scene3d, 3.0);
    assert_eq!(app.scene3d.history.len(), history, "falling adds no steps while it runs");
    stop(&mut app.scene3d, true);
    assert!(app.scene3d.physics.is_none());
    assert!((y(&app, cube) - 0.5).abs() < 0.05);
    assert_eq!(app.scene3d.history.len(), history + 1, "the whole fall is one step");
    // Undo puts the cube back up in the air.
    assert!(step_history(&mut app.scene3d, true));
    assert!((y(&app, cube) - 5.0).abs() < 1e-4);
    // Play again and revert: nothing was kept.
    play(&mut app.scene3d).unwrap();
    fall(&mut app.scene3d, 2.0);
    assert!(y(&app, cube) < 4.0);
    app.scene3d.scene.camera.position = [1.0, 2.0, 3.0];
    stop(&mut app.scene3d, false);
    assert!((y(&app, cube) - 5.0).abs() < 1e-4, "back where it was");
    assert_eq!(app.scene3d.scene.camera.position, [1.0, 2.0, 3.0], "the camera is not part of the revert");
    // Stopping when nothing runs is harmless.
    stop(&mut app.scene3d, true);
    stop(&mut app.scene3d, false);
}

#[test]
fn the_physics_runs_with_the_frames_and_the_pointer_cannot_grab_the_gizmo_meanwhile() {
    let mut app = app();
    let cube = cube_over_floor(&mut app);
    app.scene3d.scene.node_mut(cube).unwrap().pinned = false;
    app.scene3d.selected = Some(cube);
    play(&mut app.scene3d).unwrap();
    let ctx = egui::Context::default();
    let start = y(&app, cube);
    for _ in 0..30 {
        let mut out = ctx.run_ui(Default::default(), |ui| frame(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    assert!(y(&app, cube) < start, "frames advance the physics");
    // A press on the gizmo while running does not start a gizmo drag.
    let g = gizmo_for(&app.scene3d);
    if let Some(g) = g {
        let view = view_size(app.scene3d.doc_size);
        if let Some((px, py)) = photocraft_render3d::project_points(&app.scene3d.scene, view.0, view.1, &[g.origin + Vec3::X * g.size])[0] {
            let (dx, dy) = (f64::from(px) / f64::from(view.0) * 400.0, f64::from(py) / f64::from(view.1) * 300.0);
            pointer(&mut app, ToolEvent::Down { x: dx, y: dy, pressure: 1.0 }, egui::Modifiers::NONE);
            assert!(!matches!(app.scene3d.pointer.as_ref().map(|p| &p.kind), Some(PointerKind::Gizmo(_))));
        }
    }
    // Closing the mode stops the physics and forgets the base.
    close(&mut app.scene3d);
    assert!(app.scene3d.physics.is_none() && app.scene3d.play_base.is_none());
}

#[test]
fn nonsense_time_steps_are_harmless() {
    let mut app = app();
    let cube = cube_over_floor(&mut app);
    app.scene3d.scene.node_mut(cube).unwrap().pinned = false;
    play(&mut app.scene3d).unwrap();
    for dt in [f32::NAN, f32::INFINITY, -1.0, 0.0, 1.0e9] {
        tick(&mut app.scene3d, dt);
    }
    assert!(y(&app, cube).is_finite());
    tick(&mut Scene3dEditor::default(), 0.1);
}

#[test]
fn a_pinned_flag_survives_a_saved_scene_and_defaults_to_pinned() {
    let mut s = photocraft_scene3d::Scene::default();
    let id = s.add_primitive(Primitive::cube(), Transform::default()).unwrap();
    s.node_mut(id).unwrap().pinned = false;
    let back = photocraft_scene3d::Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert!(!back.node(id).unwrap().pinned);
    let old = photocraft_scene3d::Scene::from_json(r#"{"nodes":[{"id":1}]}"#).unwrap();
    assert!(old.nodes[0].pinned, "files from before the flag stay pinned");
    assert!(!s.to_json().unwrap().contains("\"pinned\": true"));
}
