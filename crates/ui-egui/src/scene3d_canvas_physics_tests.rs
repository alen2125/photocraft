use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::{NodeId, Primitive, Transform};
use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    // These tests drive Play by hand; the live physics has its own tests.
    app.scene3d.live = false;
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

#[test]
fn a_model_with_hair_can_be_played_and_an_unpinned_bone_hangs_from_the_one_above() {
    use photocraft_model3d::testgen::skinned_triangle_glb;
    let mut app = app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", json!({})).unwrap();
    // A VRM 0 file with a humanoid (hips, spine) and a spring-bone chain.
    app.scene3d.import_model("hero.vrm", &skinned_triangle_glb(true, 2)).unwrap();
    let find = |app: &PhotocraftApp, n: &str| app.scene3d.scene.nodes.iter().find(|x| x.name == n).unwrap().id;
    let (hips, spine) = (find(&app, "hips"), find(&app, "spine"));
    // Everything is pinned, but the chain swings, so Play is allowed and changes nothing else.
    play(&mut app.scene3d).unwrap();
    fall(&mut app.scene3d, 1.0);
    let hips_at = app.scene3d.scene.world_matrix(hips).unwrap().transform_point3(Vec3::ZERO);
    assert!(hips_at.is_finite());
    stop(&mut app.scene3d, false);
    // Unpinning the spine lets it swing from the hips; revert puts it back.
    let rest = app.scene3d.scene.node(spine).unwrap().transform.clone();
    app.scene3d.scene.node_mut(spine).unwrap().pinned = false;
    play(&mut app.scene3d).unwrap();
    fall(&mut app.scene3d, 2.0);
    assert!(app.scene3d.status.is_empty());
    let hips_after = app.scene3d.scene.world_matrix(hips).unwrap().transform_point3(Vec3::ZERO);
    assert!((hips_after - hips_at).length() < 1e-4, "the hips, pinned, stayed");
    stop(&mut app.scene3d, false);
    assert_eq!(app.scene3d.scene.node(spine).unwrap().transform, rest);
}

#[test]
fn while_the_physics_runs_only_a_node_that_is_on_has_a_gizmo_and_it_sits_at_the_target() {
    use photocraft_model3d::testgen::skinned_triangle_glb;
    let mut app = app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", json!({})).unwrap();
    app.scene3d.import_model("hero.vrm", &skinned_triangle_glb(true, 2)).unwrap();
    let hips = app.scene3d.scene.nodes.iter().find(|x| x.name == "hips").unwrap().id;
    app.scene3d.selected = Some(hips);
    assert!(gizmo_for(&app.scene3d).is_some(), "stopped: any node has a gizmo");
    play(&mut app.scene3d).unwrap();
    assert!(gizmo_for(&app.scene3d).is_none(), "running: an Off node has none");
    app.scene3d.scene.set_hold(hips, true).unwrap();
    let target = Vec3::new(0.3, 0.7, 0.1);
    app.scene3d.scene.node_mut(hips).unwrap().hold = Some(target.to_array());
    let g = gizmo_for(&app.scene3d).expect("an On node has one while it runs");
    assert!(g.origin.distance(target) < 1e-5, "at its target, not at the body: {:?}", g.origin);
}

#[test]
fn a_far_off_cube_is_picked_where_it_is_drawn_not_where_the_body_is() {
    use photocraft_model3d::testgen::skinned_triangle_glb;
    let mut app = app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", json!({})).unwrap();
    app.scene3d.import_model("hero.vrm", &skinned_triangle_glb(true, 2)).unwrap();
    let hips = app.scene3d.scene.nodes.iter().find(|x| x.name == "hips").unwrap().id;
    let view = view_size(app.scene3d.doc_size);
    let body = app.scene3d.scene.world_position(hips).unwrap();
    app.scene3d.scene.set_hold(hips, true).unwrap();
    app.scene3d.scene.node_mut(hips).unwrap().hold = Some((body + Vec3::new(0.6, 0.0, 0.0)).to_array());
    let cubes = crate::scene3d_ui::cube_screen(&app.scene3d.scene, view);
    let bodies = crate::scene3d_ui::bone_screen(&app.scene3d.scene, view);
    let at = |v: &[(NodeId, Option<NodeId>, (f32, f32))]| v.iter().find(|b| b.0 == hips).map(|b| b.2);
    let (c, b) = (at(&cubes), at(&bodies));
    if let (Some(c), Some(b)) = (c, b) {
        assert!((c.0 - b.0).abs() > 5.0, "the cube is drawn away from the body: {c:?} vs {b:?}");
        assert_eq!(crate::scene3d_ui::nearest_bone(&cubes, c.0, c.1, 6.0), Some(hips));
    }
}
