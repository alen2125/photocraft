use std::sync::Arc;

use glam::Vec3;
use photocraft_model3d::{MeshData, ModelData, ModelFormat, ModelNode, SkinData};

use super::*;

fn mnode(name: &str, parent: Option<usize>, children: &[usize], t: [f32; 3]) -> ModelNode {
    ModelNode { name: name.into(), parent, children: children.to_vec(), translation: t, rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
}

/// hips -> spine (up 1) -> shoulder (out 0.5) -> elbow (out 1) -> hand (out 1); a mesh on node 5.
fn body() -> (Scene, [NodeId; 5]) {
    let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let m = ModelData {
        name: "body.glb".into(),
        format: ModelFormat::Gltf,
        nodes: vec![
            mnode("hips", None, &[1], [0.0; 3]),
            mnode("spine", Some(0), &[2], [0.0, 1.0, 0.0]),
            mnode("shoulder", Some(1), &[3], [0.5, 0.0, 0.0]),
            mnode("elbow", Some(2), &[4], [1.0, 0.0, 0.0]),
            mnode("hand", Some(3), &[], [1.0, 0.0, 0.0]),
            mnode("skin", None, &[], [0.0; 3]),
        ],
        roots: vec![0, 5],
        meshes: vec![MeshData { name: "B".into(), node: 5, skin: Some(0), positions: vec![[0.0; 3]; 3], normals: vec![[0.0, 0.0, 1.0]; 3], joints: vec![[0; 4]; 3], weights: vec![[1.0, 0.0, 0.0, 0.0]; 3], indices: vec![0, 1, 2], ..MeshData::default() }],
        skins: vec![SkinData { joints: vec![0, 1, 2, 3, 4], inverse_bind: vec![id; 5] }],
        materials: vec![],
        textures: vec![],
        humanoid: vec![],
        springs: vec![],
        collider_groups: vec![],
        controls: vec![],
        joint_limits: vec![],
        control_bones: vec![],
        pose_controls: vec![],
        physics: Default::default(),
        vertex_base: Vec::new(),
        graft_start: None,
    };
    let mut s = Scene::default();
    s.add_model(Arc::new(m), None).unwrap();
    let f = |s: &Scene, n: &str| s.nodes.iter().find(|x| x.name == n).unwrap().id;
    let ids = [f(&s, "hips"), f(&s, "spine"), f(&s, "shoulder"), f(&s, "elbow"), f(&s, "hand")];
    (s, ids)
}

fn at(s: &Scene, id: NodeId) -> Vec3 {
    s.world_position(id).unwrap()
}

#[test]
fn a_held_hand_stays_where_it_was_while_the_body_moves() {
    let (mut s, [hips, _, _, elbow, hand]) = body();
    let anchor = at(&s, hand);
    s.set_hold(hand, true).unwrap();
    assert_eq!(s.node(hand).unwrap().hold, Some(anchor.to_array()));
    let e_len = at(&s, elbow).distance(at(&s, hand));
    // Slide the hips a little to one side, as dragging them would.
    translate_to(&mut s, hips, Vec3::new(0.25, -0.1, 0.0)).unwrap();
    assert!(at(&s, hand).distance(anchor) > 0.2, "without the hold the hand would have gone with the body");
    enforce_holds(&mut s, hips, false, 32).unwrap();
    assert!(at(&s, hand).distance(anchor) < 0.05, "held: {:?} vs {anchor:?}", at(&s, hand));
    assert!((at(&s, elbow).distance(at(&s, hand)) - e_len).abs() < 1e-3, "bones never stretch");
    assert!(at(&s, hips).distance(Vec3::new(0.25, -0.1, 0.0)) < 1e-4, "the dragged bone is where the user put it");
}

#[test]
fn a_hold_that_cannot_be_kept_gets_as_close_as_the_body_allows_and_nothing_breaks() {
    let (mut s, [hips, spine, shoulder, elbow, hand]) = body();
    let anchor = at(&s, hand);
    s.set_hold(hand, true).unwrap();
    let lengths = |s: &Scene| [at(s, spine).distance(at(s, shoulder)), at(s, shoulder).distance(at(s, elbow)), at(s, elbow).distance(at(s, hand))];
    let before = lengths(&s);
    translate_to(&mut s, hips, Vec3::new(8.0, 0.0, 0.0)).unwrap();
    enforce_holds(&mut s, hips, false, 32).unwrap();
    let end = at(&s, hand);
    assert!(end.is_finite());
    assert!(end.distance(anchor) < Vec3::new(8.0, 0.0, 0.0).distance(Vec3::ZERO), "nearer than if it had simply come along");
    for (a, b) in before.iter().zip(lengths(&s)) {
        assert!((a - b).abs() < 1e-3);
    }
}

#[test]
fn dragging_the_held_bone_itself_moves_its_anchor_and_other_drags_leave_holds_alone() {
    let (mut s, [hips, _, _, elbow, hand]) = body();
    s.set_hold(hand, true).unwrap();
    // Dragging the hand: it is the dragged bone, so the hold follows it.
    let target = at(&s, hand) + Vec3::new(0.0, 0.5, 0.0);
    solve_ik(&mut s, hand, target, 3, 32).unwrap();
    enforce_holds(&mut s, hand, false, 32).unwrap();
    assert!(Vec3::from(s.node(hand).unwrap().hold.unwrap()).distance(at(&s, hand)) < 1e-4);
    // Moving the elbow is a drag of a bone *above* the hand: the hand is pulled back.
    let anchor = at(&s, hand);
    let down = at(&s, elbow) + Vec3::new(0.0, -0.3, 0.0);
    solve_ik(&mut s, elbow, down, 2, 32).unwrap();
    enforce_holds(&mut s, elbow, false, 32).unwrap();
    // The elbow's own turn is the only joint that can pull it back, so it gets near, not exactly there.
    assert!(at(&s, hand).distance(anchor) < 0.15, "{:?} vs {anchor:?}", at(&s, hand));
    // A drag elsewhere in the tree that is not above the held bone does nothing to it.
    let hips_before = at(&s, hips);
    let h_before = at(&s, hand);
    enforce_holds(&mut s, hand, false, 0).unwrap();
    assert_eq!((at(&s, hips), at(&s, hand)), (hips_before, h_before));
    // Not a bone: ignored; releasing clears the anchor.
    enforce_holds(&mut s, 9999, false, 8).unwrap();
    s.set_hold(hand, false).unwrap();
    assert_eq!(s.node(hand).unwrap().hold, None);
    // Only bones can be held.
    let skin = s.nodes.iter().find(|n| n.name == "skin").unwrap().id;
    s.set_hold(skin, true).unwrap();
    assert_eq!(s.node(skin).unwrap().hold, None);
}

#[test]
fn a_hold_survives_a_save_and_bad_anchors_are_ignored() {
    let (mut s, [hips, _, _, _, hand]) = body();
    s.set_hold(hand, true).unwrap();
    let back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(back.node(hand).unwrap().hold, s.node(hand).unwrap().hold);
    s.node_mut(hand).unwrap().hold = Some([f32::NAN, 0.0, 0.0]);
    translate_to(&mut s, hips, Vec3::new(1.0, 0.0, 0.0)).unwrap();
    enforce_holds(&mut s, hips, false, 8).unwrap();
    assert!(at(&s, hand).is_finite());
}

#[test]
fn turning_a_bone_by_hand_is_respected_but_the_hold_still_pulls_the_rest_of_the_limb() {
    let (mut s, [_, spine, shoulder, _, hand]) = body();
    let anchor = at(&s, hand);
    s.set_hold(hand, true).unwrap();
    // The user bends the spine over; the shoulder and arm below it take up the slack.
    rotate_world(&mut s, spine, glam::Quat::from_rotation_z(-0.4)).unwrap();
    let turned = s.node(spine).unwrap().transform.rotation;
    enforce_holds(&mut s, spine, true, 32).unwrap();
    assert_eq!(s.node(spine).unwrap().transform.rotation, turned, "the bone the user turned keeps their angle");
    assert!(at(&s, hand).distance(anchor) < 0.15, "{:?} vs {anchor:?}", at(&s, hand));
    assert!(s.node(shoulder).unwrap().transform.rotation != [0.0; 3], "the limb below turned to compensate");
}

fn world_rot(s: &Scene, id: NodeId) -> glam::Quat {
    s.world_matrix(id).unwrap().to_scale_rotation_translation().1
}

#[test]
fn a_frozen_orientation_survives_the_limb_being_pulled_about_and_is_independent_of_the_position_hold() {
    let (mut s, [hips, _, _, elbow, hand]) = body();
    s.set_hold_rotation(hand, true).unwrap();
    assert!(s.node(hand).unwrap().hold.is_none(), "only the orientation is frozen");
    let rot = world_rot(&s, hand);
    // Bending the elbow by IK turns the hand with it, unless its orientation is frozen.
    let target = at(&s, elbow) + Vec3::new(0.0, 0.6, 0.0);
    solve_ik(&mut s, elbow, target, 2, 32).unwrap();
    assert!(world_rot(&s, hand).angle_between(rot) > 0.1, "without the freeze the hand would have turned");
    enforce_holds(&mut s, elbow, false, 32).unwrap();
    assert!(world_rot(&s, hand).angle_between(rot) < 1e-3, "frozen: {:?}", world_rot(&s, hand));
    // Its position was free the whole time.
    assert!(s.node(hand).unwrap().hold.is_none());
    // Moving the hips changes nothing about it either.
    translate_to(&mut s, hips, Vec3::new(0.3, 0.0, 0.0)).unwrap();
    enforce_holds(&mut s, hips, false, 32).unwrap();
    assert!(world_rot(&s, hand).angle_between(rot) < 1e-3);
}

#[test]
fn dragging_a_bone_with_a_frozen_orientation_keeps_it_while_turning_it_by_hand_changes_it() {
    let (mut s, [_, _, _, elbow, hand]) = body();
    s.set_hold_rotation(hand, true).unwrap();
    let rot = world_rot(&s, hand);
    // The hand dragged to a new place by IK: it arrives with the orientation it had.
    let target = at(&s, hand) + Vec3::new(-0.2, 0.5, 0.0);
    solve_ik(&mut s, hand, target, 3, 32).unwrap();
    enforce_holds(&mut s, hand, false, 32).unwrap();
    assert!(world_rot(&s, hand).angle_between(rot) < 1e-3);
    // Turned by hand it takes the new angle as the frozen one.
    rotate_world(&mut s, hand, glam::Quat::from_rotation_z(0.5)).unwrap();
    let turned = world_rot(&s, hand);
    enforce_holds(&mut s, hand, true, 32).unwrap();
    assert!(world_rot(&s, hand).angle_between(turned) < 1e-4);
    assert!(glam::Quat::from_array(s.node(hand).unwrap().hold_rot.unwrap()).angle_between(turned) < 1e-4, "the anchor followed");
    // Position and orientation freeze together, and release independently.
    s.set_hold(hand, true).unwrap();
    s.set_hold_rotation(hand, false).unwrap();
    assert!(s.node(hand).unwrap().hold.is_some() && s.node(hand).unwrap().hold_rot.is_none());
    // Only bones can be frozen; a hostile anchor is ignored.
    let skin = s.nodes.iter().find(|n| n.name == "skin").unwrap().id;
    s.set_hold_rotation(skin, true).unwrap();
    assert!(s.node(skin).unwrap().hold_rot.is_none());
    s.node_mut(hand).unwrap().hold_rot = Some([f32::NAN, 0.0, 0.0, 0.0]);
    enforce_holds(&mut s, elbow, false, 8).unwrap();
    assert!(at(&s, hand).is_finite());
    let back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(back.node(skin).unwrap().hold_rot, None);
}

#[test]
fn all_on_all_off_and_only_some_switch_every_hand_control_at_once() {
    let (mut s, [hips, spine, shoulder, elbow, hand]) = body();
    let on = |s: &Scene, id: NodeId| s.node(id).unwrap().hold.is_some() && s.node(id).unwrap().hold_rot.is_some();
    let off = |s: &Scene, id: NodeId| s.node(id).unwrap().hold.is_none() && s.node(id).unwrap().hold_rot.is_none();
    s.set_all_holds(None);
    assert!([hips, spine, shoulder, elbow, hand].iter().all(|&b| on(&s, b)));
    s.set_all_holds(Some(&["hand", "hips"]));
    assert!(on(&s, hand) && on(&s, hips) && [spine, shoulder, elbow].iter().all(|&b| off(&s, b)));
    s.set_all_holds(Some(&[]));
    assert!([hips, spine, shoulder, elbow, hand].iter().all(|&b| off(&s, b)));
    // Minor joints are left alone; only controls are switched.
    s.node_mut(spine).unwrap().minor = true;
    s.set_all_holds(None);
    assert!(off(&s, spine) && on(&s, hand));
    // Something that is not a bone has nothing to hold.
    let skin = s.nodes.iter().find(|n| n.name == "skin").unwrap().id;
    assert!(s.node(skin).unwrap().hold.is_none());
}
