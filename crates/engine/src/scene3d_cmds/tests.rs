use super::*;
use photocraft_scene3d::{Primitive, Transform};

fn session(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
    s
}

fn scene_json() -> Value {
    let mut sc = Scene::new();
    sc.camera.position = [0.0, 0.0, 4.0];
    sc.camera.target = [0.0, 0.0, 0.0];
    sc.add_primitive(Primitive::sphere(), Transform::default()).unwrap();
    serde_json::to_value(&sc).unwrap()
}

fn layer_pixels(s: &Session, id: u64) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(photocraft_doc::LayerId(id)).unwrap().surface().unwrap().read_region(Rect::new(0, 0, 64, 48))
}

#[test]
fn renders_a_transparent_layer_at_every_depth_and_undoes() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let before = s.active().unwrap().doc.layer_count();
        let r = s.execute("scene3d.renderToLayer", json!({"scene": scene_json()})).unwrap();
        assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(64), Some(48)));
        let px = layer_pixels(&s, r["layer"].as_u64().unwrap());
        let alpha: Vec<f32> = px.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        // Sphere in the middle, clear sky at the corner.
        assert!(alpha[(24 * 64 + 32) as usize] > 0.99, "depth {depth}");
        assert!(alpha[0] < 0.01, "depth {depth}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer_count(), before);
    }
}

#[test]
fn opaque_render_fills_the_sky() {
    let mut s = session(8);
    let r = s.execute("scene3d.renderToLayer", json!({"scene": scene_json(), "transparent": false, "width": 32, "height": 32})).unwrap();
    assert_eq!(r["width"], 32);
    assert!(layer_pixels(&s, r["layer"].as_u64().unwrap()).as_chunks::<4>().0.iter().take(32).all(|p| p[3] > 0.99));
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = session(8);
    for p in [
        json!({}),
        json!({"scene": 5}),
        json!({"scene": {"version": 99}}),
        json!({"scene": {"nodes": [{"id": 1, "parent": 1}]}}),
        json!({"scene": scene_json(), "width": 0}),
        json!({"scene": scene_json(), "width": 1e12}),
        json!({"scene": scene_json(), "height": -4}),
        json!({"scene": scene_json(), "width": 16384, "height": 16384, "ssaa": 4}),
        json!({"path": "/definitely/not/here.pcscene"}),
    ] {
        assert!(s.execute("scene3d.renderToLayer", p.clone()).is_err(), "{p}");
    }
    assert!(s.active().unwrap().doc.layer_count() <= 1);
}

#[test]
fn render_needs_a_document() {
    let mut s = Session::new();
    assert!(s.execute("scene3d.renderToLayer", json!({"scene": scene_json()})).is_err());
}

#[test]
fn save_then_open_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pose.pcscene");
    let path = path.to_str().unwrap();
    let mut s = Session::new();
    let saved = s.execute("scene3d.save", json!({"scene": scene_json(), "path": path})).unwrap();
    assert_eq!(saved["nodes"], 2);
    let opened = s.execute("scene3d.open", json!({"path": path})).unwrap();
    assert_eq!(Scene::from_json(&opened["scene"].to_string()).unwrap(), Scene::from_json(&scene_json().to_string()).unwrap());
    // A saved file renders by path too.
    let mut d = session(8);
    d.execute("scene3d.renderToLayer", json!({"path": path})).unwrap();
}

#[test]
fn open_and_save_reject_junk() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.pcscene");
    std::fs::write(&bad, "{ not json").unwrap();
    let mut s = Session::new();
    assert!(s.execute("scene3d.open", json!({"path": bad.to_str().unwrap()})).is_err());
    assert!(s.execute("scene3d.open", json!({})).is_err());
    assert!(s.execute("scene3d.save", json!({"scene": scene_json()})).is_err());
    assert!(s.execute("scene3d.save", json!({"path": dir.path().join("x").to_str().unwrap()})).is_err());
    assert!(!dir.path().join("x").exists());
}
