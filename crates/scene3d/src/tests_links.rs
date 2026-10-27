use glam::{Quat, Vec3};

use super::*;

fn light(kind: LightKind) -> Node {
    Node { kind: NodeKind::Light(Light { kind, ..Light::default() }), ..Node::default() }
}

fn pos(s: &Scene, id: NodeId) -> Vec3 {
    s.world_matrix(id).unwrap().transform_point3(Vec3::ZERO)
}

/// The direction a node shines: its world −Z.
fn aim(s: &Scene, id: NodeId) -> Vec3 {
    s.world_matrix(id).unwrap().transform_vector3(Vec3::NEG_Z).normalize()
}

/// A target ("face") at (1, 2, 3) and a spot light linked to it.
fn pair() -> (Scene, NodeId, NodeId) {
    let mut s = Scene::default();
    let face = s.add_primitive(Primitive::sphere(), Transform::at(1.0, 2.0, 3.0)).unwrap();
    let sun = s.add(light(LightKind::Spot { angle: 40.0 })).unwrap();
    (s, face, sun)
}

fn link(target: NodeId, distance: f32, yaw: f32, pitch: f32) -> Link {
    Link { target, distance, yaw, pitch, look_at: true, space: LinkSpace::World }
}

#[test]
fn a_linked_node_sits_at_its_distance_and_angle_from_the_target() {
    let (mut s, face, sun) = pair();
    s.set_link(sun, Some(link(face, 5.0, 0.0, 0.0))).unwrap();
    assert!((pos(&s, sun) - Vec3::new(1.0, 2.0, 8.0)).length() < 1e-4, "straight out along +Z");
    s.set_link(sun, Some(link(face, 5.0, 90.0, 0.0))).unwrap();
    assert!((pos(&s, sun) - Vec3::new(6.0, 2.0, 3.0)).length() < 1e-4, "a quarter turn round to +X");
    s.set_link(sun, Some(link(face, 2.0, 0.0, 90.0))).unwrap();
    let p = pos(&s, sun);
    assert!(p.y > 3.99 && (p.x - 1.0).abs() < 0.1, "pitch lifts it overhead (held just short of vertical): {p:?}");
}

#[test]
fn a_linked_light_aims_at_the_target_wherever_it_is() {
    let (mut s, face, sun) = pair();
    for (d, yaw, pitch) in [(4.0, 0.0, 0.0), (2.0, 135.0, 40.0), (7.5, -60.0, -30.0), (1.0, 0.0, 89.0)] {
        s.set_link(sun, Some(link(face, d, yaw, pitch))).unwrap();
        let want = (Vec3::new(1.0, 2.0, 3.0) - pos(&s, sun)).normalize();
        assert!((aim(&s, sun) - want).length() < 1e-3, "d={d} yaw={yaw} pitch={pitch}: {:?} vs {want:?}", aim(&s, sun));
    }
    // Without look-at the node keeps its own orientation.
    s.node_mut(sun).unwrap().transform.rotation = [0.0, 90.0, 0.0];
    let mut l = link(face, 3.0, 30.0, 10.0);
    l.look_at = false;
    s.set_link(sun, Some(l)).unwrap();
    assert!((aim(&s, sun) - Vec3::NEG_X).length() < 1e-4, "a quarter turn about Y points -Z at -X: {:?}", aim(&s, sun));
}

#[test]
fn moving_the_target_carries_the_linked_node_and_target_space_follows_its_turn() {
    let (mut s, face, sun) = pair();
    s.set_link(sun, Some(link(face, 5.0, 0.0, 0.0))).unwrap();
    s.node_mut(face).unwrap().transform.translation = [10.0, 0.0, 0.0];
    assert!((pos(&s, sun) - Vec3::new(10.0, 0.0, 5.0)).length() < 1e-4);
    // World space ignores the target's turn; target space follows it.
    s.node_mut(face).unwrap().transform.rotation = [0.0, 90.0, 0.0];
    assert!((pos(&s, sun) - Vec3::new(10.0, 0.0, 5.0)).length() < 1e-4);
    let mut l = link(face, 5.0, 0.0, 0.0);
    l.space = LinkSpace::Target;
    s.set_link(sun, Some(l)).unwrap();
    assert!((pos(&s, sun) - Vec3::new(15.0, 0.0, 0.0)).length() < 1e-4, "the target's +Z now points along +X: {:?}", pos(&s, sun));
}

#[test]
fn links_through_parents_are_followed_and_the_matrices_agree() {
    let (mut s, face, sun) = pair();
    let head = s.add(Node::default()).unwrap();
    s.set_parent(face, Some(head)).unwrap();
    s.node_mut(head).unwrap().transform.translation = [0.0, 10.0, 0.0];
    s.set_link(sun, Some(link(face, 3.0, 0.0, 0.0))).unwrap();
    assert!((pos(&s, sun) - Vec3::new(1.0, 12.0, 6.0)).length() < 1e-4, "the target's parent moved the target, so the light too");
    let all = s.world_matrices();
    for n in &s.nodes {
        let (a, b) = (all[&n.id].to_cols_array(), s.world_matrix(n.id).unwrap().to_cols_array());
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5), "{}", n.name);
    }
}

#[test]
fn loops_and_bad_targets_are_refused() {
    let (mut s, face, sun) = pair();
    assert!(matches!(s.set_link(sun, Some(link(sun, 1.0, 0.0, 0.0))), Err(SceneError::BadLink(_))));
    assert_eq!(s.set_link(sun, Some(link(999, 1.0, 0.0, 0.0))), Err(SceneError::NoSuchNode(999)));
    assert_eq!(s.set_link(999, None), Err(SceneError::NoSuchNode(999)));
    s.set_link(sun, Some(link(face, 1.0, 0.0, 0.0))).unwrap();
    // The face cannot be tied to the light that is tied to it, directly or through a parent.
    assert!(matches!(s.set_link(face, Some(link(sun, 1.0, 0.0, 0.0))), Err(SceneError::BadLink(_))));
    let child = s.add(Node { parent: Some(sun), ..Node::default() }).unwrap();
    assert!(matches!(s.set_link(face, Some(link(child, 1.0, 0.0, 0.0))), Err(SceneError::BadLink(_))));
    assert!(s.depends_on(sun, face) && s.depends_on(child, face) && !s.depends_on(face, sun));
}

#[test]
fn a_loop_forced_in_by_hand_still_resolves_and_never_hangs() {
    let mut s = Scene::default();
    let a = s.add(Node::default()).unwrap();
    let b = s.add(Node::default()).unwrap();
    s.node_mut(a).unwrap().link = Some(link(b, 1.0, 0.0, 0.0));
    s.node_mut(b).unwrap().link = Some(link(a, 1.0, 0.0, 0.0));
    assert!(s.world_matrix(a).is_some());
    assert_eq!(s.world_matrices().len(), 2);
}

#[test]
fn linking_keeps_the_node_where_it_is_and_unlinking_does_too() {
    let (mut s, face, sun) = pair();
    s.node_mut(sun).unwrap().transform = Transform::at(4.0, 2.0, 3.0);
    let before = pos(&s, sun);
    s.link_to(sun, face).unwrap();
    assert!((pos(&s, sun) - before).length() < 1e-4, "nothing jumped");
    let l = s.node(sun).unwrap().link.clone().unwrap();
    assert!((l.distance - 3.0).abs() < 1e-4 && (l.yaw - 90.0).abs() < 1e-3 && l.pitch.abs() < 1e-3, "{l:?}");
    assert!(l.look_at, "a spot light starts aimed at its target");
    s.node_mut(face).unwrap().transform.translation = [1.0, 7.0, 3.0];
    assert!((pos(&s, sun) - Vec3::new(4.0, 7.0, 3.0)).length() < 1e-4, "it came along");
    let placed = pos(&s, sun);
    let aimed = aim(&s, sun);
    s.unlink(sun).unwrap();
    assert!(s.node(sun).unwrap().link.is_none());
    assert!((pos(&s, sun) - placed).length() < 1e-4, "unlinking left it in place");
    assert!((aim(&s, sun) - aimed).length() < 1e-3, "and still facing the same way");
    s.node_mut(face).unwrap().transform.translation = [50.0, 0.0, 0.0];
    assert!((pos(&s, sun) - placed).length() < 1e-4, "and the target no longer matters");
    s.unlink(sun).unwrap();
}

#[test]
fn a_point_light_or_a_solid_is_not_aimed_by_default_and_keeps_its_orientation() {
    let (mut s, face, _) = pair();
    let ball = s.add_primitive(Primitive::cube(), Transform { translation: [4.0, 2.0, 3.0], rotation: [0.0, 30.0, 0.0], scale: [1.0; 3] }).unwrap();
    let before = s.world_matrix(ball).unwrap();
    s.link_to(ball, face).unwrap();
    assert!(!s.node(ball).unwrap().link.as_ref().unwrap().look_at);
    let after = s.world_matrix(ball).unwrap();
    assert!(before.to_cols_array().iter().zip(after.to_cols_array()).all(|(a, b)| (a - b).abs() < 1e-4), "same placement and turn");
}

#[test]
fn dragging_orbiting_and_scaling_a_link_edit_the_relation_not_the_node() {
    let (mut s, face, sun) = pair();
    s.set_link(sun, Some(link(face, 4.0, 0.0, 0.0))).unwrap();
    let t = Vec3::new(1.0, 2.0, 3.0);
    // Drag to a new place: distance and angles follow.
    s.set_link_from_world(sun, t + Vec3::new(0.0, 3.0, 0.0)).unwrap();
    let l = s.node(sun).unwrap().link.clone().unwrap();
    assert!((l.distance - 3.0).abs() < 1e-3 && l.pitch > 88.0);
    // Orbit a quarter turn about the vertical axis: same distance, different side.
    s.set_link(sun, Some(link(face, 4.0, 0.0, 0.0))).unwrap();
    s.orbit_link(sun, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)).unwrap();
    assert!((pos(&s, sun) - (t + Vec3::new(4.0, 0.0, 0.0))).length() < 1e-3, "{:?}", pos(&s, sun));
    assert!((s.node(sun).unwrap().link.as_ref().unwrap().distance - 4.0).abs() < 1e-3);
    // Pull closer and push out.
    s.scale_link_distance(sun, 0.5).unwrap();
    assert!((s.node(sun).unwrap().link.as_ref().unwrap().distance - 2.0).abs() < 1e-3);
    s.scale_link_distance(sun, f32::NAN).unwrap();
    s.scale_link_distance(sun, 1.0e30).unwrap();
    assert!(s.node(sun).unwrap().link.as_ref().unwrap().distance <= 1.0e6);
    // None of it works on a node that is not linked, or with nonsense.
    s.unlink(sun).unwrap();
    assert!(s.orbit_link(sun, Quat::IDENTITY).is_err() && s.scale_link_distance(sun, 2.0).is_err());
    assert!(s.set_link_from_world(sun, Vec3::NAN).is_err());
}

#[test]
fn hostile_link_numbers_cannot_poison_the_matrices() {
    let (mut s, face, sun) = pair();
    s.set_link(sun, Some(Link { target: face, distance: f32::NAN, yaw: f32::INFINITY, pitch: f32::NAN, look_at: true, space: LinkSpace::Target })).unwrap();
    assert!(s.world_matrix(sun).unwrap().to_cols_array().iter().all(|v| v.is_finite()));
    s.node_mut(face).unwrap().transform.rotation = [f32::NAN; 3];
    assert!(s.world_matrix(sun).unwrap().to_cols_array().iter().all(|v| v.is_finite()));
    s.set_link(sun, Some(link(face, -5.0, 0.0, 400.0))).unwrap();
    assert!(pos(&s, sun).is_finite());
}

#[test]
fn removing_the_target_drops_the_link_and_a_saved_scene_keeps_it() {
    let (mut s, face, sun) = pair();
    s.set_link(sun, Some(link(face, 5.0, 40.0, 10.0))).unwrap();
    let back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(back.node(sun).unwrap().link, s.node(sun).unwrap().link);
    assert!((pos(&back, sun) - pos(&s, sun)).length() < 1e-5);
    s.remove(face).unwrap();
    assert!(s.node(sun).unwrap().link.is_none());
    // A file whose link names a node that is not there loads, without the link.
    let text = r#"{"nodes":[{"id":1,"link":{"target":42}},{"id":2,"link":{"target":2}}]}"#;
    let loaded = Scene::from_json(text).unwrap();
    assert!(loaded.nodes.iter().all(|n| n.link.is_none()));
}
