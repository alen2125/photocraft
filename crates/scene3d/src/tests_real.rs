//! Posing on a real VRM character from `samples/` (gitignored: these tests do nothing without it).

use std::sync::Arc;

use glam::Vec3;

use super::*;

fn load() -> Option<Scene> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/8590256991748008892.vrm");
    let bytes = std::fs::read(path).ok()?;
    let model = Arc::new(model3d::load("hero.vrm", &bytes).unwrap());
    let mut scene = Scene::default();
    scene.add_model(model, None).unwrap();
    Some(scene)
}

/// The scene node of a VRM humanoid bone.
fn humanoid(scene: &Scene, name: &str) -> NodeId {
    let model = scene.model(1).unwrap();
    let node = model.humanoid_node(name).unwrap();
    let want = &model.nodes[node].name;
    scene.nodes.iter().find(|n| &n.name == want && matches!(n.kind, NodeKind::Bone)).unwrap().id
}

fn pos(scene: &Scene, id: NodeId) -> Vec3 {
    scene.world_matrix(id).unwrap().transform_point3(Vec3::ZERO)
}

#[test]
fn the_real_skeleton_comes_in_as_bones_and_meshes() {
    let Some(scene) = load() else { return };
    let bones = scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone)).count();
    let meshes = scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Mesh(_))).count();
    assert!(bones >= 54, "{bones} bones");
    assert_eq!(meshes, 7);
    scene.validate().unwrap();
    // Standing character: head above hips above feet, hips about a metre up.
    let (hips, head, foot) = (pos(&scene, humanoid(&scene, "hips")), pos(&scene, humanoid(&scene, "head")), pos(&scene, humanoid(&scene, "leftFoot")));
    assert!(head.y > hips.y && hips.y > foot.y && (0.6..1.4).contains(&hips.y), "{hips:?} {head:?} {foot:?}");
}

#[test]
fn a_hand_can_be_pulled_above_the_head_with_ik_keeping_bone_lengths() {
    let Some(mut scene) = load() else { return };
    let (hand, head) = (humanoid(&scene, "leftHand"), humanoid(&scene, "head"));
    let forearm = scene.node(hand).unwrap().parent.unwrap();
    let before = (pos(&scene, hand).distance(pos(&scene, forearm)), pos(&scene, forearm).distance(pos(&scene, scene.node(forearm).unwrap().parent.unwrap())));
    let target = pos(&scene, head) + Vec3::new(0.0, 0.25, 0.1);
    let r = solve_ik(&mut scene, hand, target, 3, 40).unwrap();
    assert!(r.distance < 0.02, "{r:?}");
    let upper = scene.node(forearm).unwrap().parent.unwrap();
    let after = (pos(&scene, hand).distance(pos(&scene, forearm)), pos(&scene, forearm).distance(pos(&scene, upper)));
    assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3, "{before:?} {after:?}");
    // The rest of the body stayed put.
    assert!(pos(&scene, humanoid(&scene, "rightHand")).x < -0.3 || pos(&scene, humanoid(&scene, "rightHand")).x > 0.3);
}

#[test]
fn the_real_pose_round_trips_through_a_pose_and_a_scene_file() {
    let Some(mut scene) = load() else { return };
    let hand = humanoid(&scene, "leftHand");
    let target = pos(&scene, hand) + Vec3::new(0.0, 0.4, 0.2);
    solve_ik(&mut scene, hand, target, 3, 40).unwrap();
    let moved = pos(&scene, hand);
    let i = scene.capture_pose("hands up");
    scene.reset_pose();
    assert!(pos(&scene, hand).distance(moved) > 0.1);
    scene.apply_pose(i).unwrap();
    assert!(pos(&scene, hand).distance(moved) < 1e-3);
    // Saved and reloaded, the pose is still there and applies to the same bones.
    let mut back = Scene::from_json(&scene.to_json().unwrap()).unwrap();
    assert_eq!(back.poses, scene.poses);
    back.attach_model(1, scene.model(1).unwrap().clone()).unwrap();
    back.reset_pose();
    back.apply_pose(0).unwrap();
    assert!(pos(&back, hand).distance(moved) < 1e-3);
}
