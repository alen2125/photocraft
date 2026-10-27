use super::*;

#[test]
fn json_round_trip_keeps_everything() {
    let mut s = Scene::new();
    let a = s.add_primitive(Primitive::cube(), Transform::at(1.0, 2.0, 3.0)).unwrap();
    let b = s.add_primitive(Primitive::sphere(), Transform::default()).unwrap();
    s.set_parent(b, Some(a)).unwrap();
    s.add(Node { kind: NodeKind::Light(Light { kind: LightKind::Spot { angle: 40.0 }, ..Light::default() }), ..Node::default() }).unwrap();
    let text = s.to_json().unwrap();
    let back = Scene::from_json(&text).unwrap();
    assert_eq!(s, back);
    // New ids after a load never collide with loaded ones.
    let mut back = back;
    let c = back.add_primitive(Primitive::cone(), Transform::default()).unwrap();
    assert!(back.nodes.iter().filter(|n| n.id == c).count() == 1);
}

#[test]
fn world_matrix_composes_parents() {
    let mut s = Scene::default();
    let a = s.add_primitive(Primitive::cube(), Transform::at(1.0, 0.0, 0.0)).unwrap();
    let b = s.add_primitive(Primitive::cube(), Transform::at(0.0, 2.0, 0.0)).unwrap();
    s.set_parent(b, Some(a)).unwrap();
    let p = s.world_matrix(b).unwrap().transform_point3(glam::Vec3::ZERO);
    assert!((p - glam::Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);
}

#[test]
fn cycles_are_refused() {
    let mut s = Scene::default();
    let a = s.add_primitive(Primitive::cube(), Transform::default()).unwrap();
    let b = s.add_primitive(Primitive::cube(), Transform::default()).unwrap();
    s.set_parent(b, Some(a)).unwrap();
    assert_eq!(s.set_parent(a, Some(b)), Err(SceneError::Cycle));
    assert_eq!(s.set_parent(a, Some(a)), Err(SceneError::Cycle));
}

#[test]
fn hostile_files_are_errors_not_panics() {
    // Duplicate ids, dangling parent, parent loop, newer version, garbage.
    let dup = r#"{"nodes":[{"id":1},{"id":1}]}"#;
    assert_eq!(Scene::from_json(dup), Err(SceneError::DuplicateId(1)));
    let dangling = r#"{"nodes":[{"id":1,"parent":9}]}"#;
    assert_eq!(Scene::from_json(dangling), Err(SceneError::MissingParent(1)));
    let looped = r#"{"nodes":[{"id":1,"parent":2},{"id":2,"parent":1}]}"#;
    assert_eq!(Scene::from_json(looped), Err(SceneError::Cycle));
    assert_eq!(Scene::from_json(r#"{"version":99}"#), Err(SceneError::TooNew(99)));
    assert!(matches!(Scene::from_json("not json"), Err(SceneError::Parse(_))));
    assert!(matches!(Scene::from_json(r#"{"nodes":[{"kind":{"type":"nope"}}]}"#), Err(SceneError::Parse(_))));
}

#[test]
fn node_limit_is_enforced() {
    let mut s = Scene::default();
    for _ in 0..MAX_NODES {
        s.add(Node::default()).unwrap();
    }
    assert_eq!(s.add(Node::default()), Err(SceneError::TooManyNodes));
}

#[test]
fn removing_a_node_removes_its_subtree() {
    let mut s = Scene::default();
    let a = s.add(Node::default()).unwrap();
    let b = s.add(Node { parent: Some(a), ..Node::default() }).unwrap();
    let _c = s.add(Node { parent: Some(b), ..Node::default() }).unwrap();
    let keep = s.add(Node::default()).unwrap();
    assert_eq!(s.remove(a).unwrap(), 3);
    assert_eq!(s.nodes.len(), 1);
    assert!(s.node(keep).is_some());
}

#[test]
fn primitive_meshes_are_well_formed_for_any_size() {
    let nasty = [f32::NAN, f32::INFINITY, -1.0, 0.0, 1e30];
    for &v in &nasty {
        for p in [
            Primitive::Cube { size: [v, v, v] },
            Primitive::Sphere { radius: v, segments: 0 },
            Primitive::Sphere { radius: v, segments: u32::MAX },
            Primitive::Cylinder { radius: v, height: v, segments: 1 },
            Primitive::Cone { radius: v, height: v, segments: 9999 },
            Primitive::Plane { size: [v, v] },
        ] {
            let m = p.mesh();
            assert!(!m.indices.is_empty());
            assert_eq!(m.positions.len(), m.normals.len());
            assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()));
            assert!(m.positions.iter().flatten().all(|c| c.is_finite()));
            assert!(m.normals.iter().flatten().all(|c| c.is_finite()));
        }
    }
}

#[test]
fn non_finite_transforms_do_not_poison_matrices() {
    let t = Transform { translation: [f32::NAN, 1.0, f32::INFINITY], rotation: [f32::NAN; 3], scale: [f32::NAN, 2.0, 1.0] };
    assert!(t.matrix().to_cols_array().iter().all(|v| v.is_finite()));
}
