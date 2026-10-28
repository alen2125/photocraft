//! The ragdoll on the VaM base figures in `samples/` (gitignored: these tests do nothing without
//! them): a released hand or forearm must hang calmly, not shake.

use std::sync::Arc;

use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::model3d;
use photocraft_scene3d::{NodeId, NodeKind, Primitive, Scene, Transform};

use super::*;
use crate::testutil::{release, stand};

fn load(who: &str) -> Option<Scene> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../samples/vam_base_{who}.vrm"));
    let bytes = std::fs::read(path).ok()?;
    let mut scene = Scene::default();
    scene.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    scene.add_model(Arc::new(model3d::load("vam.vrm", &bytes).unwrap()), None).unwrap();
    stand(&mut scene);
    Some(scene)
}

fn bone(s: &Scene, name: &str) -> NodeId {
    s.nodes.iter().find(|n| n.name == name && matches!(n.kind, NodeKind::Bone)).unwrap().id
}

fn pos(s: &Scene, id: NodeId) -> Vec3 {
    s.world_matrix(id).unwrap().transform_point3(Vec3::ZERO)
}

/// Releases `released`, runs 5 seconds and returns (start, end, the share of steps in the last 3
/// seconds on which the watched bone turned round). A swinging limb reverses a couple of times a
/// second at most; a shaking one reverses on every other step.
fn run(scene: &mut Scene, released: &str, watched: &str) -> (Vec3, Vec3, f32) {
    let (rel, w) = (bone(scene, released), bone(scene, watched));
    release(scene, rel);
    let start = pos(scene, w);
    let mut world = World::new(scene);
    let (mut last, mut vel, mut turns, mut counted, mut fastest) = (start, Vec3::ZERO, 0u32, 0u32, 0.0f32);
    for i in 0..300 {
        world.step_scene(scene);
        let p = pos(scene, w);
        let v = p - last;
        // Ignore steps under a millimetre: a limb at rest wobbles that much without anyone seeing it.
        if i > 120 && v.length() > 1e-3 && vel.length() > 1e-3 {
            counted += 1;
            if v.dot(vel) < 0.0 {
                turns += 1;
            }
        }
        fastest = fastest.max(v.length());
        vel = v;
        last = p;
    }
    // Nothing flies about: even a falling hand moves under 8 cm per step.
    assert!(fastest < 0.08, "{released}: {fastest} m in one step");
    (start, last, if counted == 0 { 0.0 } else { turns as f32 / counted as f32 })
}

#[test]
fn a_released_hand_hangs_from_the_wrist_without_shaking() {
    for who in ["female", "male"] {
        let Some(mut s) = load(who) else { return };
        let (start, end, worst) = run(&mut s, "lHand", "lMid3");
        assert!(end.is_finite());
        assert!(end.y < start.y - 0.03, "{who}: the fingertips dropped: {start:?} -> {end:?}");
        assert!(worst < 0.1, "{who}: the hand shakes: it turns round on {:.0}% of steps", worst * 100.0);
    }
}

#[test]
fn a_released_forearm_stays_calm_and_the_pinned_elbow_does_not_move() {
    for who in ["female", "male"] {
        let Some(mut s) = load(who) else { return };
        let elbow = bone(&s, "lForeArm");
        let at = pos(&s, elbow);
        let (_, end, worst) = run(&mut s, "lForeArm", "lHand");
        assert!(end.is_finite());
        // It moves for real (the drooping hand drags it about the elbow) but does not shake.
        assert!(worst < 0.3, "{who}: the forearm shakes: {:.0}% reversals", worst * 100.0);
        assert!(pos(&s, elbow).distance(at) < 0.03, "the elbow is the joint it hangs from (held by its spring): {}", pos(&s, elbow).distance(at));
        // Held out sideways it cannot fold down at the elbow (the elbow only bends forward), so it
        // stays out at the height of the shoulder; the hand may droop at the wrist.
        assert!(end.y > 1.2, "{who}: the forearm did not fall below the shoulder line: {end:?}");
    }
}

#[test]
fn a_whole_released_arm_falls_down_and_stays_inside_its_joint_limits() {
    let Some(mut s) = load("female") else { return };
    let (start, end, worst) = run(&mut s, "lShldr", "lHand");
    assert!(end.y < start.y - 0.3, "the arm fell from the T-pose: {start:?} -> {end:?}");
    assert!(worst < 0.1, "the arm shakes: {:.0}% reversals", worst * 100.0);
    // Every limited joint is inside its range afterwards.
    for n in s.nodes.iter().filter(|n| n.joint.is_some()) {
        let q = |e: [f32; 3]| photocraft_scene3d::glam::Quat::from_euler(photocraft_scene3d::glam::EulerRot::XYZ, e[0].to_radians(), e[1].to_radians(), e[2].to_radians());
        let rest = q(n.rest.as_ref().map_or([0.0; 3], |r| r.rotation));
        let local = q(n.transform.rotation);
        let j = n.joint.as_ref().unwrap();
        // A limit is a soft push, so a joint may be a few degrees past it for a moment.
        assert!(j.clamp(rest, local).angle_between(local).to_degrees() < 8.0, "{} is outside its limits by {}", n.name, j.clamp(rest, local).angle_between(local).to_degrees());
    }
}

/// A figure with everything Off over a floor (no `stand`).
fn load_free(who: &str) -> Option<Scene> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../samples/vam_base_{who}.vrm"));
    let bytes = std::fs::read(path).ok()?;
    let mut scene = Scene::default();
    scene.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    scene.add_model(Arc::new(model3d::load("vam.vrm", &bytes).unwrap()), None).unwrap();
    Some(scene)
}

fn steps(scene: &mut Scene, world: &mut World, n: usize) {
    for _ in 0..n {
        world.step_scene(scene);
    }
}

#[test]
fn a_figure_held_by_its_head_and_feet_hangs_between_them_instead_of_falling() {
    for who in ["female", "male"] {
        let Some(mut s) = load_free(who) else { return };
        s.set_all_holds(Some(&["head", "leftFoot", "rightFoot"]));
        let names = ["head", "lFoot", "rFoot", "hip"];
        let before: Vec<Vec3> = names.iter().map(|n| pos(&s, bone(&s, n))).collect();
        let mut w = World::new(&s);
        steps(&mut s, &mut w, 300);
        let after: Vec<Vec3> = names.iter().map(|n| pos(&s, bone(&s, n))).collect();
        // Held by springs (1000 N/m, as VaM's controllers), so the body's weight pulls the head down a
        // hand's breadth; the feet stand on the floor and hardly move.
        for i in 0..3 {
            assert!(before[i].distance(after[i]) < 0.3, "{who}: {} stayed about where it was held: {:?} -> {:?}", names[i], before[i], after[i]);
        }
        assert!(after[3].is_finite() && after[3].y > 0.85, "{who}: the hips hang from the head and stand on the legs instead of dropping: {:?} -> {:?}", before[3], after[3]);
        // Nothing went through the floor, and the body is calm after 5 seconds.
        assert!(s.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone)).all(|n| pos(&s, n.id).y > -0.02), "{who}: nothing is under the floor");
        let settle = pos(&s, bone(&s, "hip"));
        steps(&mut s, &mut w, 30);
        assert!(pos(&s, bone(&s, "hip")).distance(settle) < 0.06, "{who}: it hangs still");
    }
}

#[test]
fn a_hand_let_go_droops_below_the_wrist_within_a_second_and_a_half() {
    for who in ["female", "male"] {
        let Some(mut s) = load("female").filter(|_| who == "female").or_else(|| load(who)) else { return };
        let (rel, tip, wrist) = (bone(&s, "lHand"), bone(&s, "lMid3"), bone(&s, "lHand"));
        release(&mut s, rel);
        let mut w = World::new(&s);
        let mut at = None;
        for i in 0..150 {
            w.step_scene(&mut s);
            if at.is_none() && pos(&s, tip).y < pos(&s, wrist).y - 0.05 {
                at = Some(i);
            }
        }
        assert!(at.is_some_and(|i| i < 90), "{who}: the fingertip dropped below the wrist after {at:?} steps");
    }
}

#[test]
fn a_body_with_everything_off_lies_down_on_the_floor_and_does_not_go_through_it() {
    for who in ["female", "male"] {
        let Some(mut s) = load_free(who) else { return };
        let mut w = World::new(&s);
        steps(&mut s, &mut w, 600);
        let under: Vec<String> = s.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone) && pos(&s, n.id).y < -0.06).map(|n| format!("{} {:.2}", n.name, pos(&s, n.id).y)).collect();
        assert!(under.is_empty(), "{who}: joints under the floor: {under:?}");
        let hips = pos(&s, bone(&s, "hip")).y;
        assert!(hips < 0.4 && hips > 0.0, "{who}: the hips came down to the floor: {hips}");
    }
}

#[test]
fn a_body_dropped_onto_a_cube_lands_on_it_and_does_not_sink_into_it() {
    for who in ["female", "male"] {
        let Some(mut s) = load_free(who) else { return };
        // Lift the whole figure a metre, over a wide cube 0.4 m high.
        let root = s.nodes.iter().find(|n| n.parent.is_none() && n.name == "vam").unwrap().id;
        s.node_mut(root).unwrap().transform.translation[1] = 1.0;
        s.add_primitive(Primitive::Cube { size: [1.6, 0.4, 1.6] }, Transform::at(0.0, 0.2, 0.0)).unwrap();
        let mut w = World::new(&s);
        steps(&mut s, &mut w, 360);
        let sunk: Vec<String> = s.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone) && n.humanoid.is_some()).filter(|n| {
            let p = pos(&s, n.id);
            p.x.abs() < 0.7 && p.z.abs() < 0.7 && p.y < 0.37
        }).map(|n| format!("{} {:.2}", n.name, pos(&s, n.id).y)).collect();
        assert!(sunk.is_empty(), "{who}: joints inside the cube: {sunk:?}");
        let hips = pos(&s, bone(&s, "hip"));
        assert!(hips.y > 0.37 || hips.x.abs() > 0.7 || hips.z.abs() > 0.7, "{who}: the hips are not inside the cube: {hips:?}");
        assert!(hips.y < 1.0, "{who}: and they came down: {hips:?}");
    }
}

#[test]
fn capsule_widths_follow_the_figures_own_skin_within_sane_bounds() {
    for who in ["female", "male"] {
        let Some(s) = load(who) else { return };
        let world = s.world_matrices();
        let local_end = |from: &str, to: &str| {
            let (a, b) = (bone(&s, from), bone(&s, to));
            let (_, rot, p) = world[&a].to_scale_rotation_translation();
            rot.inverse() * (world[&b].transform_point3(Vec3::ZERO) - p)
        };
        let thigh = crate::fit::fit_radius(&s, s.node(bone(&s, "lThigh")).unwrap(), local_end("lThigh", "lShin"), 0.07);
        let forearm = crate::fit::fit_radius(&s, s.node(bone(&s, "lForeArm")).unwrap(), local_end("lForeArm", "lHand"), 0.038);
        assert!((0.035..=0.14).contains(&thigh) && thigh != 0.07, "{who}: the thigh is measured, not stock: {thigh}");
        assert!((0.019..=0.076).contains(&forearm), "{who}: forearm {forearm}");
        assert!(thigh > forearm, "{who}: a thigh is thicker than a forearm: {thigh} vs {forearm}");
    }
}

#[test]
fn without_a_skin_to_measure_the_stock_width_is_kept_and_garbage_is_harmless() {
    let Some(s) = load("female") else { return };
    let node = s.node(bone(&s, "lThigh")).unwrap().clone();
    // Another asset, a missing model, a zero-length bone, bad defaults.
    let mut other = node.clone();
    other.source = Some((99, 0));
    assert_eq!(crate::fit::fit_radius(&s, &other, Vec3::Y, 0.07), 0.07);
    let mut none = node.clone();
    none.source = None;
    assert_eq!(crate::fit::fit_radius(&s, &none, Vec3::Y, 0.07), 0.07);
    assert_eq!(crate::fit::fit_radius(&s, &node, Vec3::ZERO, 0.07), 0.07);
    assert!(crate::fit::fit_radius(&s, &node, Vec3::new(f32::NAN, 0.0, 0.0), 0.07).is_finite());
    let _ = crate::fit::fit_radius(&s, &node, Vec3::Y, f32::NAN); // no panic
}
