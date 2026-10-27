use photocraft_model3d::testgen::skinned_triangle_glb;
use photocraft_scene3d::NodeKind;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 8})).unwrap();
    s
}

/// A model file on disk; the temp dir lives as long as the returned guard.
fn model_file(vrm0: bool) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hero.vrm");
    std::fs::write(&path, skinned_triangle_glb(vrm0, 2)).unwrap();
    let p = path.to_str().unwrap().to_string();
    (dir, p)
}

#[test]
fn add_model_returns_a_scene_with_bones_and_a_mesh() {
    let (_dir, path) = model_file(true);
    let mut s = Session::new();
    let r = s.execute("scene3d.addModel", json!({"model": path})).unwrap();
    assert_eq!((r["bones"].as_u64(), r["meshes"].as_u64()), (Some(2), Some(1)));
    let scene = Scene::from_json(&r["scene"].to_string()).unwrap();
    assert_eq!(scene.assets.len(), 1);
    assert_eq!(scene.assets[0].path.as_deref(), Some(path.as_str()));
    assert!(scene.nodes.iter().any(|n| matches!(n.kind, NodeKind::Bone) && n.name == "spine"));
    assert!(scene.nodes.iter().any(|n| matches!(n.kind, NodeKind::Mesh(_))));
}

#[test]
fn a_scene_with_a_model_renders_and_reloads_the_model_from_its_path() {
    let (_dir, path) = model_file(false);
    let mut s = session();
    let added = s.execute("scene3d.addModel", json!({"model": path})).unwrap();
    // The JSON drops the geometry; the render must load it again from `path`.
    let r = s.execute("scene3d.renderToLayer", json!({"scene": added["scene"]})).unwrap();
    assert_eq!(r["width"], 64);
    // Posing then adding a second model into the same scene keeps both.
    let twice = s.execute("scene3d.addModel", json!({"model": path, "scene": added["scene"]})).unwrap();
    assert_eq!(twice["scene"]["assets"].as_array().unwrap().len(), 2);
    s.execute("scene3d.renderToLayer", json!({"scene": twice["scene"]})).unwrap();
}

#[test]
fn a_saved_scene_file_with_a_model_renders_by_path() {
    let (dir, path) = model_file(false);
    let mut s = session();
    let added = s.execute("scene3d.addModel", json!({"model": path})).unwrap();
    let file = dir.path().join("pose.pcscene");
    s.execute("scene3d.save", json!({"scene": added["scene"], "path": file.to_str().unwrap()})).unwrap();
    s.execute("scene3d.renderToLayer", json!({"path": file.to_str().unwrap()})).unwrap();
}

#[test]
fn model_problems_are_errors_not_panics() {
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("junk.glb");
    std::fs::write(&junk, b"not a model").unwrap();
    let mut s = session();
    for p in [
        json!({}),
        json!({"model": ""}),
        json!({"model": "/no/such/file.glb"}),
        json!({"model": junk.to_str().unwrap()}),
        json!({"model": junk.to_str().unwrap(), "scene": 5}),
    ] {
        assert!(s.execute("scene3d.addModel", p.clone()).is_err(), "{p}");
    }
    // A scene whose model file went missing says so instead of drawing nothing.
    let (dir2, path) = model_file(false);
    let added = s.execute("scene3d.addModel", json!({"model": path})).unwrap();
    drop(dir2);
    let err = s.execute("scene3d.renderToLayer", json!({"scene": added["scene"]})).unwrap_err().to_string();
    assert!(err.contains("hero.vrm"), "{err}");
    // An asset without a path cannot be found again.
    let mut scene = Scene::from_json(&added["scene"].to_string()).unwrap();
    scene.assets[0].path = None;
    let err = s.execute("scene3d.renderToLayer", json!({"scene": serde_json::to_value(&scene).unwrap()})).unwrap_err().to_string();
    assert!(err.contains("no file path"), "{err}");
}

#[test]
fn the_same_model_file_is_decoded_once() {
    let (_dir, path) = model_file(false);
    let a = load_model("t", &path).unwrap();
    let b = load_model("t", &path).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
}
