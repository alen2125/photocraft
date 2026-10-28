//! The anatomical limits on the VaM base figures in `samples/` (gitignored: these tests do
//! nothing without them).

use std::sync::Arc;

use glam::{Quat, Vec3};

use super::*;

fn load(who: &str) -> Option<Scene> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../samples/vam_base_{who}.vrm"));
    let bytes = std::fs::read(path).ok()?;
    let mut scene = Scene::default();
    scene.add_model(Arc::new(model3d::load("vam.vrm", &bytes).unwrap()), None).unwrap();
    Some(scene)
}

fn bone(s: &Scene, name: &str) -> NodeId {
    s.nodes.iter().find(|n| n.name == name && matches!(n.kind, NodeKind::Bone)).unwrap().id
}

fn pos(s: &Scene, id: NodeId) -> Vec3 {
    s.world_matrix(id).unwrap().transform_point3(Vec3::ZERO)
}

/// Angle between the thigh and the shin, 0 when the leg is straight.
fn knee_bend(s: &Scene) -> f32 {
    let (hip, knee, foot) = (pos(s, bone(s, "lThigh")), pos(s, bone(s, "lShin")), pos(s, bone(s, "lFoot")));
    (knee - hip).normalize().angle_between((foot - knee).normalize()).to_degrees()
}

#[test]
fn the_figures_carry_their_joint_limits_and_controls() {
    for who in ["female", "male"] {
        let Some(s) = load(who) else { return };
        let limited = s.nodes.iter().filter(|n| n.joint.is_some()).count();
        assert!(limited >= 60, "{who}: {limited} limited joints");
        let shin = s.node(bone(&s, "lShin")).unwrap().joint.clone().unwrap();
        assert_eq!((shin.twist, shin.swing), ([-11.0, 150.0], [10.0, 5.0]));
        let shown = s.bone_positions().len();
        assert_eq!(shown, 23, "{who}: only the hand controls are drawn");
    }
}

#[test]
fn a_knee_bends_one_way_only() {
    let Some(base) = load("female") else { return };
    let shin = bone(&base, "lShin");
    // Whichever way the shin is pushed about the knee's axis, one way stops almost at once and the
    // other goes on to a deep bend.
    let mut bends = Vec::new();
    for sign in [1.0f32, -1.0] {
        let mut s = base.clone();
        rotate_world(&mut s, shin, Quat::from_axis_angle(Vec3::X, sign * 2.0)).unwrap();
        bends.push(knee_bend(&s));
        if knee_bend(&s) > 100.0 {
            // The figure faces +Z, so a real knee bend sends the foot backwards.
            assert!(pos(&s, bone(&s, "lFoot")).z < pos(&s, bone(&s, "lShin")).z - 0.1, "the heel went forward");
        }
    }
    let (low, high) = (bends[0].min(bends[1]), bends[0].max(bends[1]));
    assert!(low < 15.0, "the wrong way stops near straight: {bends:?}");
    assert!(high > 100.0, "the right way bends deeply: {bends:?}");
}

#[test]
fn no_push_on_any_limited_joint_leaves_it_outside_its_range() {
    let Some(base) = load("male") else { return };
    let limited: Vec<NodeId> = base.nodes.iter().filter(|n| n.joint.is_some()).map(|n| n.id).collect();
    for id in limited {
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            for deg in [-170.0f32, -90.0, 90.0, 170.0] {
                let mut s = base.clone();
                rotate_world(&mut s, id, Quat::from_axis_angle(axis, deg.to_radians())).unwrap();
                let n = s.node(id).unwrap();
                let j = n.joint.as_ref().unwrap();
                let rest = n.rest.as_ref().map_or([0.0; 3], |r| r.rotation);
                let q = |e: [f32; 3]| Quat::from_euler(glam::EulerRot::XYZ, e[0].to_radians(), e[1].to_radians(), e[2].to_radians());
                let (rest, local) = (q(rest), q(n.transform.rotation));
                // Clamping an already clamped pose changes nothing.
                assert!(j.clamp(rest, local).angle_between(local).to_degrees() < 0.2, "{} {axis:?} {deg}", n.name);
            }
        }
    }
}

#[test]
fn an_elbow_bends_forward_and_a_shoulder_cannot_swing_the_arm_behind_the_back() {
    let Some(base) = load("female") else { return };
    let (fore, hand, upper) = (bone(&base, "lForeArm"), bone(&base, "lHand"), bone(&base, "lShldr"));
    // Push the forearm both ways about the vertical axis: one way folds deeply and forward (+Z),
    // the other hardly at all.
    let bend = |s: &Scene| (pos(s, fore) - pos(s, upper)).normalize().angle_between((pos(s, hand) - pos(s, fore)).normalize()).to_degrees();
    let mut bends = Vec::new();
    for sign in [1.0f32, -1.0] {
        let mut s = base.clone();
        rotate_world(&mut s, fore, Quat::from_axis_angle(Vec3::Y, sign * 2.5)).unwrap();
        bends.push(bend(&s));
        if bend(&s) > 90.0 {
            assert!(pos(&s, hand).z > pos(&s, fore).z + 0.05, "the hand went backwards");
        }
    }
    assert!(bends[0].min(bends[1]) < 30.0 && bends[0].max(bends[1]) > 90.0, "{bends:?}");
    // Shove the whole arm round from the shoulder, far past anything a body can do: without
    // limits it would end up pointing anywhere, with them it stays within about 100 degrees of
    // where it started.
    let rest_dir = (pos(&base, hand) - pos(&base, upper)).normalize();
    for (axis, deg) in [(Vec3::Y, 170.0f32), (Vec3::Y, -170.0), (Vec3::X, 170.0), (Vec3::Z, 170.0), (Vec3::Z, -170.0)] {
        let mut s = base.clone();
        rotate_world(&mut s, upper, Quat::from_axis_angle(axis, deg.to_radians())).unwrap();
        let dir = (pos(&s, hand) - pos(&s, upper)).normalize();
        assert!(dir.angle_between(rest_dir).to_degrees() < 105.0, "{axis:?} {deg}: {}", dir.angle_between(rest_dir).to_degrees());
    }
}
