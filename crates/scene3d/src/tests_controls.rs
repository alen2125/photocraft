use std::sync::Arc;

use glam::{Quat, Vec3};
use photocraft_model3d::{ControlBoneData, ControlRot, MeshData, ModelData, ModelFormat, ModelNode, PoseControlData, SkinData};

use super::*;

fn mnode(name: &str, parent: Option<usize>, children: &[usize], t: [f32; 3]) -> ModelNode {
    ModelNode { name: name.into(), parent, children: children.to_vec(), translation: t, rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
}

fn control(name: &str, min: f32, max: f32, rot: Vec<ControlRot>, links: Vec<(&str, f32)>) -> PoseControlData {
    PoseControlData { name: name.into(), label: name.into(), group: "Hand".into(), min, max, rot, links: links.into_iter().map(|(n, m)| (n.to_string(), m)).collect() }
}

/// A hand with two finger joints in a row: palm (0) -> f1 (1, out 1) -> f2 (2, out 1), the skin on node 3.
/// "Bend" curls both about Z (45 and 30 degrees per unit); "Grasp" only links to "Bend" (twice over).
fn hand() -> Scene {
    let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let m = ModelData {
        name: "hand.glb".into(),
        format: ModelFormat::Gltf,
        nodes: vec![mnode("palm", None, &[1], [0.0; 3]), mnode("f1", Some(0), &[2], [1.0, 0.0, 0.0]), mnode("f2", Some(1), &[], [1.0, 0.0, 0.0]), mnode("skin", None, &[], [0.0; 3])],
        roots: vec![0, 3],
        meshes: vec![MeshData { name: "B".into(), node: 3, skin: Some(0), positions: vec![[0.0; 3]; 3], normals: vec![[0.0, 0.0, 1.0]; 3], joints: vec![[0; 4]; 3], weights: vec![[1.0, 0.0, 0.0, 0.0]; 3], indices: vec![0, 1, 2], ..MeshData::default() }],
        skins: vec![SkinData { joints: vec![0, 1, 2], inverse_bind: vec![id; 3] }],
        materials: vec![],
        textures: vec![],
        humanoid: vec![],
        springs: vec![],
        collider_groups: vec![],
        controls: vec![],
        joint_limits: vec![],
        control_bones: vec![ControlBoneData { node: 1, frame: [0.0, 0.0, 0.0, 1.0], order: 0 }, ControlBoneData { node: 2, frame: [0.0, 0.0, 0.0, 1.0], order: 0 }],
        pose_controls: vec![
            control("Bend", -1.0, 1.0, vec![ControlRot { node: 1, deg: [0.0, 0.0, 45.0] }, ControlRot { node: 2, deg: [0.0, 0.0, 30.0] }], vec![]),
            control("Grasp", -1.0, 1.2, vec![], vec![("Bend", 1.0), ("Bend", 0.5)]),
            control("Twist", 0.0, 1.0, vec![ControlRot { node: 1, deg: [90.0, 0.0, 0.0] }], vec![]),
        ],
        physics: Default::default(),
    };
    let mut s = Scene::default();
    s.add_model(Arc::new(m), None).unwrap();
    s
}

fn node(s: &Scene, name: &str) -> NodeId {
    s.nodes.iter().find(|n| n.name == name).unwrap().id
}

fn at(s: &Scene, name: &str) -> Vec3 {
    s.world_position(node(s, name)).unwrap()
}

#[test]
fn a_slider_turns_its_joints_by_degrees_per_unit_and_zero_puts_them_back() {
    let mut s = hand();
    assert_eq!(at(&s, "f2"), Vec3::new(2.0, 0.0, 0.0));
    s.set_control(1, "Bend", 1.0).unwrap();
    // 45 degrees at the first joint (30 more at the second): the tip curls down.
    let tip = at(&s, "f2");
    let (c, sn) = (45f32.to_radians().cos(), 45f32.to_radians().sin());
    assert!((tip - Vec3::new(1.0 + c, -sn, 0.0)).length() < 1e-4, "{tip:?}");
    assert!((at(&s, "f2") - at(&s, "f1")).length() > 0.99, "bones keep their length");
    // Half the value, half the turn.
    s.set_control(1, "Bend", 0.5).unwrap();
    let half = Quat::from_euler(glam::EulerRot::XYZ, 0.0, 0.0, -22.5f32.to_radians());
    assert!(s.world_matrix(node(&s, "f1")).unwrap().to_scale_rotation_translation().1.angle_between(half) < 1e-3);
    // Back to zero: exactly the rest pose, and nothing left in the scene file.
    s.set_control(1, "Bend", 0.0).unwrap();
    assert!((at(&s, "f2") - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-4);
    assert!(s.controls.is_empty());
}

#[test]
fn a_value_is_clamped_to_the_sliders_range_and_garbage_is_harmless() {
    let mut s = hand();
    s.set_control(1, "Bend", 5.0).unwrap();
    assert_eq!(s.control_value(1, "Bend"), 1.0);
    s.set_control(1, "Bend", f32::NAN).unwrap();
    assert_eq!(s.control_value(1, "Bend"), 0.0);
    assert!(s.set_control(1, "Nope", 1.0).is_err());
    assert!(s.set_control(9, "Bend", 1.0).is_err());
    s.apply_controls(9).unwrap();
}

#[test]
fn a_linked_slider_adds_its_value_to_the_others_and_they_add_up() {
    let mut s = hand();
    s.set_control(1, "Grasp", 1.0).unwrap();
    // Grasp 1 adds 1.5 to Bend, which is capped at its own range of 1.
    let capped = at(&s, "f2");
    s.set_control(1, "Grasp", 0.0).unwrap();
    s.set_control(1, "Bend", 1.0).unwrap();
    assert!((at(&s, "f2") - capped).length() < 1e-4, "capped at Bend's maximum, the same as Bend 1");
    s.set_control(1, "Grasp", -1.0).unwrap();
    // Bend 1 + (-1.5) = -0.5: the joints turn the other way.
    assert!(at(&s, "f2").y > 0.0);
    s.reset_controls(1).unwrap();
    assert!((at(&s, "f2") - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-4 && s.controls.is_empty());
}

#[test]
fn a_joints_own_axes_decide_what_a_turn_about_x_does() {
    let mut s = hand();
    // The first joint's X axis is tilted a quarter turn about Z: a twist about "its X" lifts the tip.
    let mut m = (**s.model(1).unwrap()).clone();
    m.control_bones[0].frame = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
    let mut t = Scene::default();
    t.add_model(Arc::new(m), None).unwrap();
    t.set_control(1, "Twist", 1.0).unwrap();
    // The tip of f2 hangs off f1 along X; turning f1 90 degrees about its own X (world Y) swings it in the XZ plane.
    let tip = at(&t, "f2");
    assert!((tip.y).abs() < 1e-4 && tip.z.abs() > 0.99, "{tip:?}");
    // And a scene file keeps the values; reloading the model and re-applying restores the pose.
    s.set_control(1, "Bend", 0.7).unwrap();
    let back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(back.control_value(1, "Bend"), 0.7);
}

#[test]
fn a_scene_saved_before_nodes_knew_their_model_gets_its_sliders_back_when_the_model_is_attached() {
    let mut s = hand();
    s.set_control(1, "Bend", 1.0).unwrap();
    let model = s.model(1).unwrap().clone();
    // An old file: no link to the model's nodes, no limits, no minor flags.
    for n in &mut s.nodes {
        n.source = None;
        n.joint = None;
        n.minor = false;
    }
    let mut old = Scene::from_json(&s.to_json().unwrap()).unwrap();
    old.reset_pose();
    assert!(old.set_control(1, "Bend", 1.0).is_err(), "no model attached yet");
    old.attach_model(1, model.clone()).unwrap();
    let f1 = old.nodes.iter().find(|n| n.name == "f1").unwrap();
    assert_eq!(f1.source, Some((1, 1)));
    let before = f1.transform.rotation;
    old.set_control(1, "Bend", 1.0).unwrap();
    assert_ne!(old.nodes.iter().find(|n| n.name == "f1").unwrap().transform.rotation, before, "the slider turns the bone");
    // The model's controls list and limits come back too, and a link that exists is not overwritten.
    let mut m = (*model).clone();
    m.controls = vec![0];
    m.joint_limits = vec![photocraft_model3d::JointLimitData { node: 2, frame: [0.0, 0.0, 0.0, 1.0], twist: [-10.0, 10.0], swing: [5.0, 5.0] }];
    let mut again = Scene::from_json(&s.to_json().unwrap()).unwrap();
    again.attach_model(1, Arc::new(m)).unwrap();
    let (f1, f2) = (again.nodes.iter().find(|n| n.name == "f1").unwrap(), again.nodes.iter().find(|n| n.name == "f2").unwrap());
    assert!(f1.minor && f2.minor, "only the palm is a hand control");
    assert!(f2.joint.is_some() && f1.joint.is_none());
    let mut keep = hand();
    keep.attach_model(1, model).unwrap();
    assert!(keep.nodes.iter().all(|n| n.source.is_none() || n.source.is_some_and(|(a, _)| a == 1)));
}
