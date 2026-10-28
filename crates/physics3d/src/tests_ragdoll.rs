use std::sync::Arc;

use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::model3d::{ColliderGroup, ModelData, ModelFormat, ModelNode, SkinData, SpringGroup};
use photocraft_scene3d::{Node, NodeId, NodeKind, Primitive, Scene, Transform};

use super::*;
use crate::testutil::{release, stand};

/// A small T-pose humanoid standing with its hips at (0, 1, 0): spine, chest, both arms, a head,
/// and a strand of hair that sticks out backwards from the top of the head.
fn humanoid_model(hair_collider: bool) -> Arc<ModelData> {
    let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let node = |name: &str, parent: Option<usize>, t: [f32; 3]| ModelNode {
        name: name.into(),
        parent,
        children: vec![],
        translation: t,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
    let mut nodes = vec![
        node("hips", None, [0.0, 1.0, 0.0]),
        node("spine", Some(0), [0.0, 0.15, 0.0]),
        node("chest", Some(1), [0.0, 0.15, 0.0]),
        node("lUpper", Some(2), [0.15, 0.1, 0.0]),
        node("lLower", Some(3), [0.3, 0.0, 0.0]),
        node("lHand", Some(4), [0.25, 0.0, 0.0]),
        node("rUpper", Some(2), [-0.15, 0.1, 0.0]),
        node("rLower", Some(6), [-0.3, 0.0, 0.0]),
        node("rHand", Some(7), [-0.25, 0.0, 0.0]),
        node("head", Some(2), [0.0, 0.25, 0.0]),
        node("hair1", Some(9), [0.0, 0.2, 0.0]),
        node("hair2", Some(10), [0.0, 0.0, -0.15]),
    ];
    for i in 0..nodes.len() {
        if let Some(p) = nodes[i].parent {
            nodes[p].children.push(i);
        }
    }
    let n = nodes.len();
    Arc::new(ModelData {
        name: "hero.vrm".into(),
        format: ModelFormat::Gltf,
        roots: vec![0],
        nodes,
        meshes: vec![],
        skins: vec![SkinData { joints: (0..n).collect(), inverse_bind: vec![id; n] }],
        materials: vec![],
        textures: vec![],
        humanoid: ["hips", "spine", "chest", "leftUpperArm", "leftLowerArm", "leftHand", "rightUpperArm", "rightLowerArm", "rightHand", "head"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name.to_string(), if i < 9 { i } else { 9 }))
            .collect(),
        springs: vec![SpringGroup {
            comment: "hair".into(),
            stiffness: 0.3,
            gravity_power: 1.0,
            gravity_dir: [0.0, -1.0, 0.0],
            drag_force: 0.3,
            hit_radius: 0.02,
            roots: vec![10],
            collider_groups: if hair_collider { vec![0] } else { vec![] },
        }],
        collider_groups: vec![ColliderGroup { node: 9, spheres: vec![([0.0, 0.0, 0.0], 0.12)] }],
        controls: vec![],
        joint_limits: vec![],
        control_bones: vec![],
        pose_controls: vec![],
    })
}

fn scene_with(model: Arc<ModelData>, floor: bool) -> Scene {
    let mut s = Scene::default();
    if floor {
        s.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    }
    s.add_model(model, None).unwrap();
    stand(&mut s);
    s
}

fn bone(s: &Scene, name: &str) -> NodeId {
    s.nodes.iter().find(|n| n.name == name && matches!(n.kind, NodeKind::Bone)).unwrap().id
}

fn pos(s: &Scene, id: NodeId) -> Vec3 {
    s.world_matrix(id).unwrap().transform_point3(Vec3::ZERO)
}

/// Lets go of a bone and everything below it.
fn unpin(s: &mut Scene, name: &str) {
    let id = bone(s, name);
    release(s, id);
}

fn run(s: &mut Scene, seconds: f32) -> World {
    let mut w = World::new(s);
    for _ in 0..(seconds / STEP) as usize {
        w.step_scene(s);
    }
    w
}

#[test]
fn bones_get_their_humanoid_names_and_unnamed_nodes_do_not() {
    let s = scene_with(humanoid_model(false), false);
    let named: Vec<&str> = s.nodes.iter().filter_map(|n| n.humanoid.as_deref()).collect();
    assert!(named.contains(&"leftUpperArm") && named.contains(&"hips") && named.contains(&"head"));
    assert_eq!(s.node(bone(&s, "hair1")).unwrap().humanoid, None);
}

#[test]
fn a_pinned_character_is_a_solid_obstacle_that_things_land_on() {
    let mut s = scene_with(humanoid_model(false), true);
    let before: Vec<Transform> = s.nodes.iter().map(|n| n.transform.clone()).collect();
    let cube = s.add_primitive(Primitive::cube(), Transform::at(0.0, 3.5, 0.0)).unwrap();
    s.node_mut(cube).unwrap().pinned = false;
    let w = run(&mut s, 4.0);
    assert_eq!(w.dynamic_count(), 1, "only the cube falls");
    assert!(pos(&s, cube).y > 1.3, "it came to rest on the head and shoulders, not on the floor: {:?}", pos(&s, cube));
    for (n, t) in s.nodes.iter().zip(&before) {
        if !matches!(n.kind, NodeKind::Primitive { .. }) && !n.name.starts_with("hair") {
            assert_eq!(&n.transform, t, "{} is pinned and did not move", n.name);
        }
    }
}

#[test]
fn an_unpinned_arm_lets_go_of_its_whole_limb_and_hangs_from_the_shoulder() {
    let mut s = scene_with(humanoid_model(false), true);
    unpin(&mut s, "lUpper");
    let (upper, lower, hand) = (bone(&s, "lUpper"), bone(&s, "lLower"), bone(&s, "lHand"));
    let (hips, chest) = (bone(&s, "hips"), bone(&s, "chest"));
    let start = (pos(&s, upper), pos(&s, lower), pos(&s, hand), pos(&s, hips), pos(&s, chest));
    let w = run(&mut s, 3.0);
    assert_eq!(w.dynamic_count(), 3, "upper arm, forearm and hand");
    assert!((pos(&s, upper) - start.0).length() < 1e-4, "the shoulder joint stayed");
    assert!((pos(&s, hips) - start.3).length() < 1e-4 && (pos(&s, chest) - start.4).length() < 1e-4, "the body stayed");
    let end = pos(&s, hand);
    assert!(end.y < start.2.y - 0.25, "the hand dropped: {:?} -> {end:?}", start.2);
    assert!((pos(&s, lower).distance(pos(&s, upper)) - start.1.distance(start.0)).abs() < 1e-3, "bone lengths held");
    assert!((end.distance(pos(&s, lower)) - start.2.distance(start.1)).abs() < 1e-3);
    // The other arm was pinned and still sticks out sideways.
    assert!((pos(&s, bone(&s, "rHand")).y - 1.4).abs() < 1e-3 && pos(&s, bone(&s, "rHand")).x < -0.5);
}

#[test]
fn a_released_forearm_cannot_swing_down_through_a_pinned_slab() {
    let mut s = scene_with(humanoid_model(false), true);
    unpin(&mut s, "lLower");
    let elbow = pos(&s, bone(&s, "lLower"));
    // A thin pinned slab just under the elbow's height, out where the forearm would swing.
    let top = elbow.y - 0.15;
    s.add_primitive(Primitive::Cube { size: [1.0, 0.1, 1.0] }, Transform::at(elbow.x + 0.2, top - 0.05, 0.0)).unwrap();
    let hand = bone(&s, "lHand");
    // The lowest the hand ever gets over three seconds.
    let (mut w, mut lowest) = (World::new(&s), f32::MAX);
    for _ in 0..180 {
        w.step_scene(&mut s);
        lowest = lowest.min(pos(&s, hand).y);
    }
    assert!(lowest > top - 0.02, "the hand never goes through the slab: lowest hand y {lowest}, slab top {top}");
    assert!(lowest < elbow.y - 0.05, "but it did swing down towards it: {lowest}");
}

#[test]
fn hair_sags_under_gravity_and_keeps_its_length() {
    let mut s = scene_with(humanoid_model(false), false);
    let (root, tip) = (bone(&s, "hair1"), bone(&s, "hair2"));
    let (start_tip, len) = (pos(&s, tip), pos(&s, tip).distance(pos(&s, root)));
    let mut w = World::new(&s);
    assert_eq!((w.dynamic_count(), w.spring_count() > 0, w.is_inert()), (0, true, false), "only hair moves");
    for _ in 0..180 {
        w.step_scene(&mut s);
    }
    let end = pos(&s, tip);
    assert!(end.y < start_tip.y - 0.05, "it hangs lower than the horizontal strand it started as: {start_tip:?} -> {end:?}");
    assert!((end.distance(pos(&s, root)) - len).abs() < 1e-3, "rotation only, so the strand did not stretch");
    assert!(end.is_finite());
}

#[test]
fn hair_stays_out_of_the_heads_collider_sphere() {
    let head_centre = |s: &Scene| pos(s, bone(s, "head"));
    let mut with = scene_with(humanoid_model(true), false);
    let mut without = scene_with(humanoid_model(false), false);
    run(&mut with, 4.0);
    run(&mut without, 4.0);
    let d_with = pos(&with, bone(&with, "hair2")).distance(head_centre(&with));
    let d_without = pos(&without, bone(&without, "hair2")).distance(head_centre(&without));
    assert!(d_with >= 0.12 + 0.02 - 0.01, "kept outside the sphere and the strand's own radius: {d_with}");
    assert!(d_without < d_with - 0.03, "without the collider it sinks into the head: {d_without} vs {d_with}");
}

#[test]
fn releasing_the_hips_lets_the_whole_body_fall_to_the_floor_without_stretching() {
    let mut s = scene_with(humanoid_model(false), true);
    unpin(&mut s, "hips");
    let hips = bone(&s, "hips");
    let lengths = |s: &Scene| -> Vec<f32> { ["spine", "chest", "lUpper", "lLower", "rHand"].iter().map(|n| pos(s, bone(s, n)).distance(pos(s, s.node(bone(s, n)).unwrap().parent.unwrap()))).collect() };
    let before = lengths(&s);
    let w = run(&mut s, 5.0);
    assert!(w.dynamic_count() >= 8);
    let p = pos(&s, hips);
    assert!(p.is_finite() && p.y < 0.6 && p.y > 0.0, "it fell, and not through the floor: {p:?}");
    for (a, b) in before.iter().zip(lengths(&s)) {
        assert!((a - b).abs() < 1e-2, "a bone changed length: {a} -> {b}");
    }
    assert!(s.nodes.iter().all(|n| n.transform.translation.iter().all(|v| v.is_finite())));
}

#[test]
fn the_same_character_always_falls_the_same_way() {
    let build = || {
        let mut s = scene_with(humanoid_model(true), true);
        unpin(&mut s, "rUpper");
        s
    };
    let (mut a, mut b) = (build(), build());
    run(&mut a, 2.0);
    run(&mut b, 2.0);
    assert_eq!(a.nodes, b.nodes);
}

#[test]
fn broken_bones_and_models_without_a_humanoid_are_skipped_not_fatal() {
    let mut s = scene_with(humanoid_model(true), true);
    let upper = bone(&s, "lUpper");
    s.node_mut(upper).unwrap().transform.translation = [f32::NAN, 0.0, 0.0];
    s.node_mut(bone(&s, "rLower")).unwrap().transform.scale = [0.0, 0.0, 0.0];
    unpin(&mut s, "lUpper");
    let w = run(&mut s, 1.0);
    assert!(w.dynamic_count() <= 12);
    // Bones that are not a humanoid (a plain Bone node) are left alone.
    let mut plain = Scene::default();
    plain.add(Node { kind: NodeKind::Bone, pinned: false, ..Node::default() }).unwrap();
    let mut w = World::new(&plain);
    assert!(w.is_inert());
    w.step_scene(&mut plain);
}
