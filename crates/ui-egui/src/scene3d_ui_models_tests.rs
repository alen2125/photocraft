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
    app.scene3d.scene.set_hold_rotation(spine, true).unwrap();
    assert!(app.scene3d.scene.node(spine).unwrap().hold.is_some() && app.scene3d.scene.node(spine).unwrap().hold_rot.is_some());
    for _ in 0..3 {
        let mut out = ctx.run_ui(Default::default(), |ui| crate::scene3d_ui::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    assert!(app.scene3d.status.is_empty(), "{}", app.scene3d.status);
    app.scene3d.scene.set_hold(spine, false).unwrap();
    app.scene3d.scene.set_hold_rotation(spine, false).unwrap();
    assert!(app.scene3d.scene.node(spine).unwrap().hold.is_none() && app.scene3d.scene.node(spine).unwrap().hold_rot.is_none());
}

#[test]
fn the_pose_sliders_panel_lists_a_models_sliders_and_moving_one_poses_the_bone() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 120, "height": 90})).unwrap();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", serde_json::json!({})).unwrap();
    app.scene3d.import_model("hero.vrm", &skinned_triangle_glb(true, 2)).unwrap();
    let asset = app.scene3d.scene.assets[0].id;
    assert_eq!(app.scene3d.scene.model(asset).unwrap().pose_controls.len(), 1);
    let spine = app.scene3d.scene.nodes.iter().find(|n| n.name == "spine").unwrap().id;
    app.scene3d.control_filter = "bend".into();
    for _ in 0..3 {
        let mut out = ctx.run_ui(Default::default(), |ui| crate::scene3d_ui::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    assert!(app.scene3d.status.is_empty(), "{}", app.scene3d.status);
    let before = app.scene3d.scene.node(spine).unwrap().transform.rotation;
    app.scene3d.scene.set_control(asset, "Bend", 1.0).unwrap();
    assert_ne!(app.scene3d.scene.node(spine).unwrap().transform.rotation, before);
    // A saved pose remembers the slider; resetting the pose puts both back.
    let i = app.scene3d.scene.capture_pose("bent");
    app.scene3d.scene.reset_pose();
    assert_eq!(app.scene3d.scene.node(spine).unwrap().transform.rotation, before);
    assert_eq!(app.scene3d.scene.control_value(asset, "Bend"), 0.0);
    app.scene3d.scene.apply_pose(i).unwrap();
    assert_eq!(app.scene3d.scene.control_value(asset, "Bend"), 1.0);
}

#[test]
fn a_pose_slider_changes_the_picture_of_the_real_figure_fresh_from_import_and_from_an_old_scene() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/vam_base_female.vrm");
    let Ok(bytes) = std::fs::read(&path) else { return };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "ui.scene3d.open", serde_json::json!({})).unwrap();
    app.scene3d.import_model("vam_base_female.vrm", &bytes).unwrap();
    let asset = app.scene3d.scene.assets[0].id;
    // Look at the left hand from close by.
    let hand = app.scene3d.scene.nodes.iter().find(|n| n.name == "lHand").unwrap().id;
    let at = app.scene3d.scene.world_position(hand).unwrap();
    app.scene3d.scene.camera.target = at.to_array();
    app.scene3d.scene.camera.position = (at + Vec3::new(0.0, 0.5, 0.6)).to_array();
    let shot = |scene: &photocraft_scene3d::Scene| photocraft_render3d::render(scene, &photocraft_render3d::RenderOptions { ssaa: 1, ..photocraft_render3d::RenderOptions::new(160, 160) }).unwrap().rgba;
    let rest = shot(&app.scene3d.scene);
    app.scene3d.scene.set_control(asset, "CTRLlHandFist", 1.0).unwrap();
    let fist = shot(&app.scene3d.scene);
    let changed = rest.iter().zip(&fist).filter(|(a, b)| a != b).count();
    assert!(changed > 500, "a fist looks different from an open hand: {changed} bytes changed");
    // The same through a scene file written before nodes remembered their model.
    let mut old = app.scene3d.scene.clone();
    old.reset_pose();
    for n in &mut old.nodes {
        n.source = None;
    }
    let mut old = photocraft_scene3d::Scene::from_json(&old.to_json().unwrap()).unwrap();
    old.attach_model(asset, app.scene3d.scene.model(asset).unwrap().clone()).unwrap();
    old.set_control(asset, "CTRLlHandFist", 1.0).unwrap();
    assert_eq!(shot(&old), fist, "the reopened scene poses the same way");
}

/// A real preset from a VaM install lands on the figure (run with `VAM_DIR`, and `PIC_DIR` for a picture).
#[test]
#[ignore]
fn a_real_look_imports_onto_the_figure() {
    let dir = std::env::var("VAM_DIR").unwrap_or_default();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/vam_base_female.vrm");
    let Ok(bytes) = std::fs::read(path) else { return };
    let look_path = std::env::var("VAP").unwrap_or_else(|_| format!("{dir}/Custom/Atom/Person/Appearance/Preset_Anime naked.vap"));
    let Ok(look) = std::fs::read(&look_path) else { return };
    let mut ed = Scene3dEditor::default();
    let ctx = egui::Context::default();
    let figure_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/vam_base_female.vrm");
    ed.import_model(&figure_path.to_string_lossy(), &bytes).unwrap();
    ed.vam.dir = dir;
    crate::scene3d_vam::open_library(&mut ed, &ctx);
    if let Ok(first) = std::env::var("VAP0") {
        let first = std::fs::read(first).unwrap();
        crate::scene3d_vam::import_look(&mut ed, "first.vap", &first).unwrap();
        crate::scene3d_vam::import_look(&mut ed, "look.vap", &look).unwrap();
    }
    let t0 = std::time::Instant::now();
    let summary = crate::scene3d_vam::import_look(&mut ed, "look.vap", &look).unwrap();
    eprintln!("LOOK {summary}");
    eprintln!("LOOK took {:?}", t0.elapsed());
    assert!(ed.scene.morphs.len() > 5, "morph values were set");
    // Isolate what breaks: MODE=morphs keeps only the shape, MODE=skin keeps only the pictures.
    match std::env::var("MODE").as_deref() {
        Ok("morphs") => {
            ed.scene.material_overrides.clear();
            ed.scene.apply_morphs(1).unwrap();
        }
        Ok("skin") => {
            ed.scene.reset_morphs(1).unwrap();
        }
        Ok("bare") => {
            ed.scene.material_overrides.clear();
            ed.scene.reset_morphs(1).unwrap();
        }
        _ => {}
    }
    if std::env::var("PHYS").is_ok() {
        crate::scene3d_canvas::play(&mut ed).unwrap();
        for _ in 0..180 {
            crate::scene3d_canvas::tick(&mut ed, 1.0 / 60.0);
        }
    }
    if std::env::var("DIAG").is_ok() {
        let m = ed.scene.model(1).unwrap().clone();
        let base = ed.scene.morph_cache_base(1).unwrap();
        let (now, rest) = (&m.meshes[0].positions, &base.meshes[0].positions);
        let mut spread = std::collections::HashMap::<u32, (f32, usize)>::new();
        let mut first = std::collections::HashMap::<u32, usize>::new();
        for (j, &b) in m.vertex_base.iter().enumerate() {
            let f = *first.entry(b).or_insert(j);
            let d = (0..3).map(|c| (now[j][c] - now[f][c]).powi(2)).sum::<f32>().sqrt();
            let e = spread.entry(b).or_insert((0.0, j));
            if d > e.0 { *e = (d, j); }
        }
        let mut worst: Vec<_> = spread.iter().filter(|(_, (d, _))| *d > 1e-4).collect();
        worst.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
        eprintln!("DIAG seam copies apart: {} of {}", worst.len(), spread.len());
        for (b, (d, j)) in worst.iter().take(12) { eprintln!("  base {b} apart {d:.4} at y={:.3} z={:.3} x={:.3}", now[*j][1], now[*j][2], now[*j][0]); }
        {
            let me = &m.meshes[0];
            let mut firstw = std::collections::HashMap::<u32, usize>::new();
            let mut differ = 0;
            for (j, &b) in m.vertex_base.iter().enumerate() {
                let f = *firstw.entry(b).or_insert(j);
                if f != j && (me.joints[j] != me.joints[f] || me.weights[j].iter().zip(&me.weights[f]).any(|(x, y)| (x - y).abs() > 1e-4)) {
                    differ += 1;
                    if differ < 6 { eprintln!("  weights differ base {b}: {:?}{:?} vs {:?}{:?} at y={:.3}", me.joints[j], me.weights[j], me.joints[f], me.weights[f], now[j][1]); }
                }
            }
            eprintln!("DIAG copies with other skin weights: {differ}");
        }
        let mut edges: Vec<(f32, [f32; 3], usize, usize)> = Vec::new();
        for t in m.meshes.iter().flat_map(|me| me.indices.chunks(3)) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (a as usize, b as usize);
                let len = |p: &Vec<[f32; 3]>| (0..3).map(|c| (p[a][c] - p[b][c]).powi(2)).sum::<f32>().sqrt();
                let (l0, l1) = (len(rest), len(now));
                if l0 > 1e-5 && l1 > 0.004 { edges.push((l1 / l0, now[a], a, b)); }
            }
        }
        edges.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (r, p, a, b) in edges.iter().take(15) { eprintln!("  stretch x{r:.1} at x={:.3} y={:.3} z={:.3} verts {a},{b} base {},{}", p[0], p[1], p[2], m.vertex_base[*a], m.vertex_base[*b]); }
    }
    if let Ok(out) = std::env::var("PIC_DIR") {
        let mut s = ed.scene.clone();
        s.camera.position = [0.0, 1.0, 3.4];
        s.camera.target = [0.0, 0.95, 0.0];
        s.camera.fov = 35.0;
        if std::env::var("POSE").is_ok() {
            for n in &mut s.nodes {
                match n.humanoid.as_deref() {
                    Some("head") => n.transform.rotation = [25.0, 0.0, 30.0],
                    Some("neck") => n.transform.rotation = [10.0, 0.0, 12.0],
                    _ => {}
                }
            }
        }
        let mut size = (520, 800);
        if std::env::var("CAM").as_deref() == Ok("face") {
            s.camera.position = [0.0, 1.66, 0.75];
            s.camera.target = [0.0, 1.645, 0.0];
            s.camera.fov = 30.0;
            size = (800, 800);
        }
        if std::env::var("CAM").as_deref() == Ok("body") {
            s.camera.position = [0.0, 1.15, 1.7];
            s.camera.target = [0.0, 1.1, 0.0];
            s.camera.fov = 30.0;
            size = (800, 800);
        }
        let img = photocraft_render3d::render(&s, &photocraft_render3d::RenderOptions { ssaa: std::env::var("SSAA").ok().and_then(|v| v.parse().ok()).unwrap_or(2), ..photocraft_render3d::RenderOptions::new(size.0, size.1) }).unwrap();
        std::fs::write(std::path::Path::new(&out).join("look.rgba"), img.rgba).unwrap();
    }
}
