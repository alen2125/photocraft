//! Building the body: one rigid body per hand control (the bones a person moves), made of
//! the model's own collision capsules, with the model's own mass and drag, hung from the body
//! part above by a ball joint. The small joints (fingers, toes' tips, the model's minor bones)
//! have no body: their capsules ride on the hand or foot they belong to, so a hand is one solid
//! thing and cannot shake itself apart.

use std::collections::{HashMap, HashSet};

use photocraft_scene3d::glam::{Mat4, Quat, Vec3};
use photocraft_scene3d::{NodeId, NodeKind, Scene};
use rapier3d::na::Point3;
use rapier3d::parry::bounding_volume::BoundingVolume;
use rapier3d::prelude::*;

use crate::{fit, iso, pt, vam, World, JOINT_LIMIT, MAX_SOLIDS};

/// Pairs of capsules closer than this at rest never collide: neighbouring body parts overlap at
/// their joints, and would shove each other apart the moment the world starts.
const IGNORE_MARGIN: f32 = 0.004;

/// Capsule radius and the length given to a bone with no child, by VRM humanoid name, and
/// whether the bone is a finger.
fn region(h: &str) -> Option<(f32, f32, bool)> {
    Some(match h {
        "hips" => (0.11, 0.10, false),
        "spine" | "chest" | "upperChest" => (0.11, 0.12, false),
        "neck" => (0.045, 0.08, false),
        "head" => (0.09, 0.20, false),
        "leftShoulder" | "rightShoulder" => (0.04, 0.10, false),
        "leftUpperArm" | "rightUpperArm" => (0.045, 0.25, false),
        "leftLowerArm" | "rightLowerArm" => (0.038, 0.25, false),
        "leftHand" | "rightHand" => (0.035, 0.10, false),
        "leftUpperLeg" | "rightUpperLeg" => (0.07, 0.40, false),
        "leftLowerLeg" | "rightLowerLeg" => (0.055, 0.40, false),
        "leftFoot" | "rightFoot" => (0.04, 0.12, false),
        "leftToes" | "rightToes" => (0.03, 0.06, false),
        h if ["Thumb", "Index", "Middle", "Ring", "Little"].iter().any(|f| h.contains(f)) => (0.012, 0.04, true),
        _ => return None,
    })
}

/// A capsule as (end point, end point, radius), in the body's own axes.
type Capsule = (Vec3, Vec3, f32);

fn capsule_volume(c: &Capsule) -> f32 {
    let r = c.2;
    std::f32::consts::PI * r * r * c.0.distance(c.1) + 4.0 / 3.0 * std::f32::consts::PI * r * r * r
}

fn collider_for(c: &Capsule, friction: f32, bounciness: f32) -> Collider {
    let builder = if c.0.distance(c.1) < 1e-5 { ColliderBuilder::ball(c.2).translation(vector![c.0.x, c.0.y, c.0.z]) } else { ColliderBuilder::capsule_from_endpoints(pt(c.0), pt(c.1), c.2) };
    builder.density(0.0).friction(friction).restitution(bounciness).active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS).build()
}

/// Strength of an arm's muscles against VaM's numbers. PhysX counts drive strength differently from
/// rapier; at full strength a let-go arm hardly sags here, while in the game it swings down. The
/// trunk and legs keep the full numbers: they carry the body.
const ARM_MUSCLE: f32 = 0.0;

impl World {
    pub(crate) fn add_ragdoll(&mut self, scene: &Scene, world: &HashMap<NodeId, Mat4>) {
        // Humanoid bones, parents first, and each one's nearest humanoid ancestor.
        let humanoid: Vec<NodeId> = scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone) && n.humanoid.as_deref().and_then(region).is_some() && scene.is_visible(n.id)).map(|n| n.id).collect();
        if humanoid.is_empty() {
            return;
        }
        let set: HashSet<NodeId> = humanoid.iter().copied().collect();
        let ancestor_of = |id: NodeId| -> Option<NodeId> {
            let mut cur = scene.node(id)?.parent;
            for _ in 0..photocraft_scene3d::MAX_DEPTH {
                let c = cur?;
                if set.contains(&c) {
                    return Some(c);
                }
                cur = scene.node(c)?.parent;
            }
            None
        };
        let parent_h: HashMap<NodeId, NodeId> = humanoid.iter().filter_map(|&id| Some((id, ancestor_of(id)?))).collect();
        let depth = |id: NodeId| {
            let (mut d, mut cur) = (0, id);
            while let Some(p) = parent_h.get(&cur) {
                d += 1;
                cur = *p;
                if d > photocraft_scene3d::MAX_DEPTH {
                    break;
                }
            }
            d
        };
        let mut order = humanoid.clone();
        order.sort_by_key(|id| depth(*id));
        let pose = |id: NodeId| -> Option<(Vec3, Quat)> {
            let (_, r, t) = world.get(&id)?.to_scale_rotation_translation();
            (t.is_finite() && r.is_finite()).then_some((t, r.normalize()))
        };
        let mut children: HashMap<NodeId, Vec<Vec3>> = HashMap::new();
        for &c in &humanoid {
            if let (Some(&p), Some((t, _))) = (parent_h.get(&c), pose(c)) {
                children.entry(p).or_default().push(t);
            }
        }
        let (friction, bounciness) = (self.friction, self.bounciness);
        let anchor_body = self.bodies.insert(RigidBodyBuilder::fixed().build());
        self.anchor = Some(anchor_body);
        let mut placed: HashMap<NodeId, usize> = HashMap::new();
        let mut collider_parts: Vec<(usize, ColliderHandle)> = Vec::new();
        for id in order {
            let Some(node) = scene.node(id) else { continue };
            let Some(hn) = node.humanoid.as_deref() else { continue };
            let Some((radius, default_len, finger)) = region(hn) else { continue };
            let Some((p0, rot)) = pose(id) else { continue };
            // The nearest bone above that has a body: the joint to hang from, or the body to ride on.
            let host = {
                let mut cur = parent_h.get(&id).copied();
                let mut found = None;
                for _ in 0..photocraft_scene3d::MAX_DEPTH {
                    let Some(c) = cur else { break };
                    if let Some(&p) = placed.get(&c) {
                        found = Some(p);
                        break;
                    }
                    cur = parent_h.get(&c).copied();
                }
                found
            };
            let host_pose = host.map(|h| (self.parts[h].start_pos, self.parts[h].start_rot));
            let end = match children.get(&id) {
                Some(c) if !c.is_empty() => c.iter().copied().sum::<Vec3>() / c.len() as f32,
                _ => {
                    let dir = host_pose.map_or(Vec3::Y, |(hp, _)| (p0 - hp).normalize_or(Vec3::Y));
                    p0 + dir * default_len
                }
            };
            let local_end = rot.inverse() * (end - p0);
            if !local_end.is_finite() || local_end.length() < 1e-3 || self.parts.len() + self.solid_count >= MAX_SOLIDS {
                continue;
            }
            let thick = fit::fit_radius(scene, node, local_end, radius);

            // A small joint rides on its host's body.
            if node.minor || finger {
                let (Some(h), Some((hp, hr))) = (host, host_pose) else { continue };
                let cap = (hr.inverse() * (p0 - hp), hr.inverse() * (end - hp), thick);
                let handle = self.colliders.insert_with_parent(collider_for(&cap, friction, bounciness), self.parts[h].handle, &mut self.bodies);
                collider_parts.push((h, handle));
                continue;
            }

            // The model's own physical makeup, or VaM's numbers for the same body part.
            let model = node.source.and_then(|(a, i)| scene.model(a).map(|m| (m, i as usize)));
            let data = model.and_then(|(m, i)| m.physics.bodies.iter().find(|b| b.node == i));
            let fallback = vam::defaults(hn);
            let (mass, drag, angular_drag) = match (data, fallback) {
                (Some(d), _) => (d.mass, d.drag, d.angular_drag),
                (None, Some(f)) => (f.mass, f.drag, f.angular_drag),
                _ => (1.0, 0.5, 5.0),
            };
            let arm = hn.contains("Arm") || hn.contains("Hand") || hn.contains("Shoulder");
            let muscle = data.and_then(|d| d.muscle).or(fallback.map(|f| f.muscle)).filter(|_| host.is_some());
            let caps: Vec<Capsule> = match model {
                Some((m, i)) if m.physics.colliders.iter().any(|c| c.node == i) => m.physics.colliders.iter().filter(|c| c.node == i).map(|c| (Vec3::from(c.a), Vec3::from(c.b), c.radius)).collect(),
                _ => vec![(Vec3::ZERO, local_end, thick)],
            };
            // Centre of mass and a moment of inertia from the capsules, the mass spread by volume.
            let vols: Vec<f32> = caps.iter().map(capsule_volume).collect();
            let total: f32 = vols.iter().sum::<f32>().max(1e-9);
            let com = caps.iter().zip(&vols).map(|(c, v)| (c.0 + c.1) * 0.5 * (v / total)).sum::<Vec3>();
            let inertia = caps.iter().zip(&vols).map(|(c, v)| mass * (v / total) * (((c.0 + c.1) * 0.5 - com).length_squared() + c.0.distance_squared(c.1) / 12.0 + 0.4 * c.2 * c.2)).sum::<f32>().max(1e-4);

            let (pos_on, rot_on) = (node.hold.is_some(), node.hold_rot.is_some());
            let body = RigidBodyBuilder::dynamic()
                .position(iso(p0, rot))
                .linear_damping(drag * self.damping)
                .angular_damping(angular_drag * self.damping)
                .can_sleep(false)
                .additional_mass_properties(MassProperties::new(Point3::new(com.x, com.y, com.z), mass.max(0.01), vector![inertia, inertia, inertia]))
                .build();
            let handle = self.bodies.insert(body);
            let idx = self.parts.len();
            for c in &caps {
                let ch = self.colliders.insert_with_parent(collider_for(c, friction, bounciness), handle, &mut self.bodies);
                collider_parts.push((idx, ch));
            }
            let limit = node.joint.clone().filter(|j| j.enabled && host.is_some());
            let rest_rel = host_pose.map_or(Quat::IDENTITY, |(_, hr)| (hr.inverse() * rot).normalize());
            if let (Some(h), Some((hp, hr))) = (host, host_pose) {
                let anchor = hr.inverse() * (p0 - hp);
                // A ball joint at the part's origin, its frame turned to the pose at rest, so that
                // the muscle (a motor on each of its three turning axes) pulls the part back to
                // that pose. The two bodies it joins never collide (their capsules overlap at the
                // joint). The model's anatomical limits are `drive.rs`'s job, not rapier's.
                let mut joint = GenericJointBuilder::new(JointAxesMask::LOCKED_SPHERICAL_AXES).contacts_enabled(false).local_frame1(iso(anchor, rest_rel)).local_frame2(rapier3d::na::Isometry3::identity());
                if limit.is_none() {
                    for axis in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
                        joint = joint.limits(axis, [-JOINT_LIMIT, JOINT_LIMIT]);
                    }
                }
                if let Some(m) = muscle {
                    let scale = if arm { ARM_MUSCLE } else { 1.0 };
                    for axis in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
                        joint = joint.motor_model(axis, MotorModel::ForceBased).motor_position(axis, 0.0, m.spring * scale, m.damper * scale).motor_max_force(axis, m.max_force * scale);
                    }
                }
                self.joints.insert(self.parts[h].handle, handle, joint.build(), true);
            }
            // A controller that is On: a joint to the world, free except for a spring-damper
            // motor on each axis it holds (position, orientation), as VaM's controllers do.
            if pos_on || rot_on {
                let target_pos = node.hold.map(Vec3::from).filter(|p| p.is_finite()).unwrap_or(p0);
                let target_rot = node.hold_rot.map(|q| Quat::from_xyzw(q[0], q[1], q[2], q[3])).filter(|q| q.is_finite() && q.length_squared() > 1e-9).map_or(rot, Quat::normalize);
                let mut joint = GenericJointBuilder::new(JointAxesMask::empty()).contacts_enabled(false).local_frame1(iso(target_pos, target_rot)).local_frame2(rapier3d::na::Isometry3::identity());
                let (hp, hr) = (self.hold_position, self.hold_rotation);
                if pos_on {
                    for axis in [JointAxis::LinX, JointAxis::LinY, JointAxis::LinZ] {
                        joint = joint.motor_model(axis, MotorModel::ForceBased).motor_position(axis, 0.0, hp.spring, hp.damper).motor_max_force(axis, hp.max_force);
                    }
                }
                if rot_on {
                    for axis in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
                        joint = joint.motor_model(axis, MotorModel::ForceBased).motor_position(axis, 0.0, hr.spring, hr.damper).motor_max_force(axis, hr.max_force);
                    }
                }
                self.joints.insert(anchor_body, handle, joint.build(), true);
            }
            self.parts.push(crate::Part { id, handle, parent: host, subtree: vec![idx], com, start_pos: p0, start_rot: rot, rest_rel, limit, lin_drag: drag, ang_drag: angular_drag });
            placed.insert(id, idx);
        }
        self.collect_subtrees();
        self.ignore_rest_overlaps(&collider_parts);
    }

    /// For each part, itself and every part that hangs from it.
    fn collect_subtrees(&mut self) {
        for j in 0..self.parts.len() {
            // Walk up from j: every part above it gets j in its subtree.
            let mut cur = self.parts[j].parent;
            for _ in 0..photocraft_scene3d::MAX_DEPTH {
                let Some(c) = cur else { break };
                self.parts[c].subtree.push(j);
                cur = self.parts[c].parent;
            }
        }
    }

    /// Parts whose capsules overlap (or all but touch) in the pose the world is built from never
    /// collide with each other afterwards, as VaM does for neighbouring body parts.
    fn ignore_rest_overlaps(&mut self, colliders: &[(usize, ColliderHandle)]) {
        let boxes: Vec<(usize, ColliderHandle, rapier3d::parry::bounding_volume::Aabb)> = colliders
            .iter()
            .filter_map(|(p, h)| {
                let c = self.colliders.get(*h)?;
                Some((*p, *h, c.compute_aabb().loosened(IGNORE_MARGIN)))
            })
            .collect();
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                if a.0 == b.0 || !a.2.intersects(&b.2) {
                    continue;
                }
                let (Some(ca), Some(cb)) = (self.colliders.get(a.1), self.colliders.get(b.1)) else { continue };
                let touching = rapier3d::parry::query::contact(ca.position(), ca.shape(), cb.position(), cb.shape(), IGNORE_MARGIN).is_ok_and(|c| c.is_some());
                if touching {
                    self.ignore.insert(a.1, b.1);
                }
            }
        }
    }
}
