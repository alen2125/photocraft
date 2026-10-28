//! The physics on the real VRM character in `samples/` (gitignored: these tests do nothing
//! without it).

use std::sync::Arc;

use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::model3d;
use photocraft_scene3d::{NodeId, NodeKind, Primitive, Scene, Transform};

use super::*;

fn load() -> Option<Scene> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/8590256991748008892.vrm");
    let bytes = std::fs::read(path).ok()?;
    let mut scene = Scene::default();
    scene.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    scene.add_model(Arc::new(model3d::load("hero.vrm", &bytes).unwrap()), None).unwrap();
    Some(scene)
}

fn named(scene: &Scene, humanoid: &str) -> NodeId {
    scene.nodes.iter().find(|n| n.humanoid.as_deref() == Some(humanoid)).unwrap().id
}

fn pos(scene: &Scene, id: NodeId) -> Vec3 {
    scene.world_matrix(id).unwrap().transform_point3(Vec3::ZERO)
}

#[test]
fn the_real_characters_hair_swings_and_stays_finite() {
    let Some(mut scene) = load() else { return };
    let mut w = World::new(&scene);
    assert!(w.spring_count() > 20, "the hair, skirt and accessories chains were found: {}", w.spring_count());
    assert_eq!(w.dynamic_count(), 0, "everything is pinned");
    let before: Vec<Vec3> = scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone) && n.humanoid.is_none()).map(|n| pos(&scene, n.id)).collect();
    for _ in 0..120 {
        w.step_scene(&mut scene);
    }
    let after: Vec<Vec3> = scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone) && n.humanoid.is_none()).map(|n| pos(&scene, n.id)).collect();
    assert!(after.iter().all(|p| p.is_finite()));
    let moved = before.iter().zip(&after).filter(|(a, b)| a.distance(**b) > 0.003).count();
    assert!(moved > 5, "some of the hair bones swung: {moved}");
    // The body did not move.
    assert!(pos(&scene, named(&scene, "hips")).y > 0.6);
}

#[test]
fn releasing_a_real_arm_lets_it_hang_without_stretching_or_going_through_the_body() {
    let Some(mut scene) = load() else { return };
    let upper = named(&scene, "leftUpperArm");
    scene.node_mut(upper).unwrap().pinned = false;
    let (lower, hand) = (named(&scene, "leftLowerArm"), named(&scene, "leftHand"));
    let (hips, torso) = (pos(&scene, named(&scene, "hips")), pos(&scene, named(&scene, "chest")));
    let len = pos(&scene, hand).distance(pos(&scene, lower));
    let start = pos(&scene, hand);
    let mut w = World::new(&scene);
    assert!(w.dynamic_count() >= 3, "the arm and everything below it");
    for _ in 0..240 {
        w.step_scene(&mut scene);
    }
    let end = pos(&scene, hand);
    assert!(end.y < start.y - 0.15, "the hand dropped from the T-pose: {start:?} -> {end:?}");
    assert!((end.distance(pos(&scene, lower)) - len).abs() < 2e-3, "the forearm kept its length");
    assert!((pos(&scene, named(&scene, "hips")) - hips).length() < 1e-4 && (pos(&scene, named(&scene, "chest")) - torso).length() < 1e-4, "the body stayed");
    // The hand is outside the torso's column (it hangs beside the body rather than through it).
    let axis_distance = ((end.x - torso.x).powi(2) + (end.z - torso.z).powi(2)).sqrt();
    assert!(axis_distance > 0.08, "not inside the torso: {axis_distance}");
}
