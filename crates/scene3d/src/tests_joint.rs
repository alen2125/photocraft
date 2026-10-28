use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use glam::{Quat, Vec3};
use photocraft_model3d::{JointLimitData, MeshData, ModelData, ModelFormat, ModelNode, SkinData};

use super::*;

fn knee() -> JointLimit {
    JointLimit { frame: [0.0, 0.0, 0.0, 1.0], twist: [-150.0, 11.0], swing: [10.0, 5.0], enabled: true }
}

fn about(axis: Vec3, deg: f32) -> Quat {
    Quat::from_axis_angle(axis, deg.to_radians())
}

fn angle_between(a: Quat, b: Quat) -> f32 {
    a.angle_between(b).to_degrees()
}

#[test]
fn a_hinge_bends_its_way_and_stops_at_its_range() {
    let j = knee();
    // Inside the range nothing changes.
    let ok = about(Vec3::X, -90.0);
    assert!(angle_between(j.clamp(Quat::IDENTITY, ok), ok) < 1e-3);
    // Past either end it stops at the end.
    assert!(angle_between(j.clamp(Quat::IDENTITY, about(Vec3::X, -170.0)), about(Vec3::X, -150.0)) < 0.05);
    assert!(angle_between(j.clamp(Quat::IDENTITY, about(Vec3::X, 60.0)), about(Vec3::X, 11.0)) < 0.05);
}

#[test]
fn swing_is_an_ellipse_with_its_own_reach_on_each_side() {
    let j = knee();
    // Y reaches 10 degrees, Z only 5.
    assert!(angle_between(j.clamp(Quat::IDENTITY, about(Vec3::Y, 40.0)), about(Vec3::Y, 10.0)) < 0.1);
    assert!(angle_between(j.clamp(Quat::IDENTITY, about(Vec3::Z, -40.0)), about(Vec3::Z, -5.0)) < 0.1);
    assert!(angle_between(j.clamp(Quat::IDENTITY, about(Vec3::Y, 5.0)), about(Vec3::Y, 5.0)) < 1e-3, "inside the cone nothing changes");
    // The same joint with the clamp applied twice does not move again.
    let once = j.clamp(Quat::IDENTITY, about(Vec3::new(1.0, 1.0, 1.0).normalize(), 80.0));
    assert!(angle_between(j.clamp(Quat::IDENTITY, once), once) < 0.05);
}

#[test]
fn the_frame_and_the_rest_pose_decide_which_way_is_which() {
    // Rotate the joint frame a quarter turn about Z: its X axis now points along world Y.
    let q = about(Vec3::Z, 90.0);
    let j = JointLimit { frame: q.to_array(), ..knee() };
    assert!(angle_between(j.clamp(Quat::IDENTITY, about(Vec3::Y, 60.0)), about(Vec3::Y, 11.0)) < 0.1);
    // World X is now the joint's (negated) Y axis, which reaches 10 degrees.
    assert!(angle_between(j.clamp(Quat::IDENTITY, about(Vec3::X, -60.0)), about(Vec3::X, -10.0)) < 0.2, "world X is now a swing axis");
    // Limits are about the turn away from rest, not about the absolute rotation.
    let rest = about(Vec3::Y, FRAC_PI_2.to_degrees());
    let j = knee();
    let bent = rest * about(Vec3::X, -170.0);
    assert!(angle_between(j.clamp(rest, bent), rest * about(Vec3::X, -150.0)) < 0.05);
    assert!(angle_between(j.clamp(rest, rest), rest) < 1e-3, "at rest the joint is where it was");
}

#[test]
fn a_joint_that_is_off_or_fed_garbage_never_misbehaves() {
    let off = JointLimit { enabled: false, ..knee() };
    let far = about(Vec3::X, 120.0);
    assert_eq!(off.clamp(Quat::IDENTITY, far), far);
    let bad = JointLimit { frame: [f32::NAN, 0.0, 0.0, 0.0], twist: [f32::NAN, f32::INFINITY], swing: [f32::NAN, -1.0], enabled: true };
    assert!(bad.clamp(Quat::IDENTITY, far).is_finite());
    let _ = knee().clamp(Quat::IDENTITY, Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0));
    assert!(knee().clamp(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0), far).is_finite());
    let locked = JointLimit { twist: [0.0, 0.0], swing: [0.0, 0.0], ..knee() };
    assert!(angle_between(locked.clamp(Quat::IDENTITY, about(Vec3::new(1.0, 2.0, 3.0).normalize(), 70.0)), Quat::IDENTITY) < 0.1);
}

fn limited_arm(limit: bool) -> (Scene, NodeId, NodeId) {
    let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let n = |name: &str, parent: Option<usize>, ch: &[usize], t: [f32; 3]| ModelNode { name: name.into(), parent, children: ch.to_vec(), translation: t, rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] };
    let m = ModelData {
        name: "arm.glb".into(),
        format: ModelFormat::Gltf,
        nodes: vec![n("shoulder", None, &[1], [0.0; 3]), n("elbow", Some(0), &[2], [1.0, 0.0, 0.0]), n("hand", Some(1), &[], [1.0, 0.0, 0.0]), n("body", None, &[], [0.0; 3])],
        roots: vec![0, 3],
        meshes: vec![MeshData { name: "B".into(), node: 3, skin: Some(0), positions: vec![[0.0; 3]; 3], normals: vec![[0.0, 0.0, 1.0]; 3], joints: vec![[0; 4]; 3], weights: vec![[1.0, 0.0, 0.0, 0.0]; 3], indices: vec![0, 1, 2], ..MeshData::default() }],
        skins: vec![SkinData { joints: vec![0, 1, 2], inverse_bind: vec![id; 3] }],
        materials: vec![],
        textures: vec![],
        humanoid: vec![],
        springs: vec![],
        collider_groups: vec![],
        controls: vec![],
        // The elbow bends one way only, 0 to 130 degrees, about Z.
        joint_limits: if limit { vec![JointLimitData { node: 1, frame: [0.0, 0.0, 0.0, 1.0], twist: [0.0, 0.0], swing: [0.0, 0.0] }] } else { vec![] },
    };
    let mut s = Scene::default();
    s.add_model(Arc::new(m), None).unwrap();
    let find = |s: &Scene, n: &str| s.nodes.iter().find(|x| x.name == n).unwrap().id;
    let (elbow, hand) = (find(&s, "elbow"), find(&s, "hand"));
    (s, elbow, hand)
}

#[test]
fn limits_from_the_file_hold_for_gizmo_turns_and_for_dragging_the_hand() {
    let (mut s, elbow, hand) = limited_arm(true);
    assert!(s.node(elbow).unwrap().joint.is_some() && s.node(hand).unwrap().joint.is_none());
    // A completely locked elbow does not turn however hard it is pushed.
    rotate_world(&mut s, elbow, about(Vec3::Z, 90.0)).unwrap();
    let at = s.world_matrix(hand).unwrap().transform_point3(Vec3::ZERO);
    assert!((at - Vec3::new(2.0, 0.0, 0.0)).length() < 0.02, "the arm stayed straight: {at:?}");
    // Dragging the hand around: the elbow stays straight, only the shoulder follows.
    solve_ik(&mut s, hand, Vec3::new(0.5, 1.5, 0.0), 2, 16).unwrap();
    let e = s.node(elbow).unwrap().transform.rotation;
    assert!(e.iter().all(|c| c.abs() < 0.5), "elbow rotation {e:?}");
    // Without limits the same turn works.
    let (mut free, elbow, hand) = limited_arm(false);
    rotate_world(&mut free, elbow, about(Vec3::Z, 90.0)).unwrap();
    let at = free.world_matrix(hand).unwrap().transform_point3(Vec3::ZERO);
    assert!((at - Vec3::new(1.0, 1.0, 0.0)).length() < 0.02, "{at:?}");
}

#[test]
fn a_joint_switched_off_moves_freely_and_the_limit_survives_a_save() {
    let (mut s, elbow, hand) = limited_arm(true);
    s.node_mut(elbow).unwrap().joint.as_mut().unwrap().enabled = false;
    rotate_world(&mut s, elbow, about(Vec3::Z, 90.0)).unwrap();
    let at = s.world_matrix(hand).unwrap().transform_point3(Vec3::ZERO);
    assert!((at - Vec3::new(1.0, 1.0, 0.0)).length() < 0.02);
    let back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(back.node(elbow).unwrap().joint, s.node(elbow).unwrap().joint);
}
