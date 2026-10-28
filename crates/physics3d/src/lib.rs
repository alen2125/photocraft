//! Gravity, collisions, ragdoll limbs and hair for the 3D Pose Editor, on rapier. A [`World`] is
//! built from a scene:
//!
//! * pinned solids are fixed obstacles; unpinned ones fall and collide (a Plane is the floor);
//! * every bone of a VRM humanoid gets a collision capsule, so limbs cannot pass through the body
//!   or each other and solids land on the character; a pinned bone is fixed in its pose, and an
//!   unpinned bone lets go of its whole limb, which then hangs from the joint above it;
//! * spring bones (hair, skirts) swing under inertia and gravity and stay out of the body's
//!   colliders.
//!
//! Steps are a fixed 1/60 s with no randomness, so the same scene always settles the same way.
//! Nothing here panics on bad input: shapes with non-finite or degenerate sizes are left out.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod springs;

use std::collections::HashMap;

use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{NodeId, NodeKind, Primitive, Scene, set_world_rotation};
use rapier3d::na::{Isometry3, Point3, Quaternion, Translation3, UnitQuaternion};
use rapier3d::prelude::*;

/// Fixed simulation step in seconds.
pub const STEP: f32 = 1.0 / 60.0;
/// Most falling bodies and total solids a world takes (more are left out).
pub const MAX_DYNAMIC: usize = 256;
pub const MAX_SOLIDS: usize = 1024;
/// Thinnest a shape may be, in scene units.
const MIN_SIZE: f32 = 1e-3;
/// Thickness of a Plane's collision slab (its top face sits at the plane).
const SLAB: f32 = 0.5;
/// How far a ragdoll joint may bend on each axis, radians.
const JOINT_LIMIT: f32 = 1.6;

// Collision groups: which things may touch which. A shoulder or upper arm does not collide with
// the torso it hangs from (their capsules overlap at the joint), but forearms and hands do.
const G_TORSO: u32 = 1;
const G_UPPER: u32 = 1 << 1;
const G_LIMB: u32 = 1 << 2;
const G_OBJECT: u32 = 1 << 3;
const ALL: u32 = G_TORSO | G_UPPER | G_LIMB | G_OBJECT;

fn groups(member: u32) -> InteractionGroups {
    let filter = match member {
        G_TORSO => G_LIMB | G_OBJECT,
        G_UPPER => G_UPPER | G_LIMB | G_OBJECT,
        _ => ALL,
    };
    InteractionGroups::new(Group::from_bits_truncate(member), Group::from_bits_truncate(filter))
}

/// Collision group, capsule radius and the length given to a bone with no child, by VRM humanoid name.
fn region(h: &str) -> Option<(u32, f32, f32)> {
    Some(match h {
        "hips" => (G_TORSO, 0.11, 0.10),
        "spine" | "chest" | "upperChest" => (G_TORSO, 0.11, 0.12),
        "neck" => (G_TORSO, 0.045, 0.08),
        "head" => (G_TORSO, 0.09, 0.20),
        "leftShoulder" | "rightShoulder" => (G_UPPER, 0.04, 0.10),
        "leftUpperArm" | "rightUpperArm" => (G_UPPER, 0.045, 0.25),
        "leftLowerArm" | "rightLowerArm" => (G_LIMB, 0.038, 0.25),
        "leftHand" | "rightHand" => (G_LIMB, 0.035, 0.10),
        "leftUpperLeg" | "rightUpperLeg" => (G_UPPER, 0.07, 0.40),
        "leftLowerLeg" | "rightLowerLeg" => (G_LIMB, 0.055, 0.40),
        "leftFoot" | "rightFoot" => (G_LIMB, 0.04, 0.12),
        "leftToes" | "rightToes" => (G_LIMB, 0.03, 0.06),
        h if ["Thumb", "Index", "Middle", "Ring", "Little"].iter().any(|f| h.contains(f)) => (G_LIMB, 0.012, 0.04),
        _ => return None,
    })
}

struct BoneBody {
    id: NodeId,
    handle: RigidBodyHandle,
    /// Whether the bone has a bone above it in the ragdoll to hang from.
    has_parent: bool,
}

pub struct World {
    pipeline: PhysicsPipeline,
    params: IntegrationParameters,
    islands: IslandManager,
    broad: DefaultBroadPhase,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    joints: ImpulseJointSet,
    multibody: MultibodyJointSet,
    ccd: CCDSolver,
    /// The falling solids and the scene node each drives.
    dynamic: Vec<(NodeId, RigidBodyHandle)>,
    /// The ragdoll limbs that fall (pinned bones are fixed and not listed).
    limbs: Vec<BoneBody>,
    fixed: usize,
    springs: springs::Springs,
}

fn iso(pos: Vec3, rot: Quat) -> Isometry3<f32> {
    Isometry3::from_parts(Translation3::new(pos.x, pos.y, pos.z), UnitQuaternion::from_quaternion(Quaternion::new(rot.w, rot.x, rot.y, rot.z)))
}

fn pt(v: Vec3) -> Point3<f32> {
    Point3::new(v.x, v.y, v.z)
}

/// The collision shape of a solid, sized by the node's world scale, and where it sits relative to
/// the node's origin. `None` for a degenerate shape.
fn shape(p: &Primitive, scale: Vec3) -> Option<(ColliderBuilder, Isometry3<f32>)> {
    let s = scale.abs().max(Vec3::splat(MIN_SIZE));
    let clamp = |v: f32| if v.is_finite() { v.abs().clamp(MIN_SIZE, 1.0e6) } else { 1.0 };
    let ident = Isometry3::identity();
    Some(match *p {
        Primitive::Cube { size } => (ColliderBuilder::cuboid(clamp(size[0]) * s.x / 2.0, clamp(size[1]) * s.y / 2.0, clamp(size[2]) * s.z / 2.0), ident),
        Primitive::Sphere { radius, .. } => (ColliderBuilder::ball(clamp(radius) * s.max_element()), ident),
        Primitive::Cylinder { radius, height, .. } => (ColliderBuilder::cylinder(clamp(height) * s.y / 2.0, clamp(radius) * s.x.max(s.z)), ident),
        Primitive::Cone { radius, height, .. } => (ColliderBuilder::cone(clamp(height) * s.y / 2.0, clamp(radius) * s.x.max(s.z)), ident),
        Primitive::Plane { size } => (ColliderBuilder::cuboid(clamp(size[0]) * s.x / 2.0, SLAB, clamp(size[1]) * s.z / 2.0), Isometry3::translation(0.0, -SLAB, 0.0)),
    })
}

impl World {
    /// Builds the world from the scene's visible solids and bones.
    pub fn new(scene: &Scene) -> World {
        let mut w = World {
            pipeline: PhysicsPipeline::new(),
            params: IntegrationParameters { dt: STEP, ..IntegrationParameters::default() },
            islands: IslandManager::new(),
            broad: DefaultBroadPhase::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            joints: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            dynamic: Vec::new(),
            limbs: Vec::new(),
            fixed: 0,
            springs: springs::Springs::default(),
        };
        let world = scene.world_matrices();
        w.add_solids(scene, &world);
        w.add_ragdoll(scene, &world);
        w.springs = springs::Springs::new(scene);
        w
    }

    fn add_solids(&mut self, scene: &Scene, world: &HashMap<NodeId, photocraft_scene3d::glam::Mat4>) {
        for n in &scene.nodes {
            let NodeKind::Primitive { primitive, .. } = &n.kind else { continue };
            if !scene.is_visible(n.id) || self.dynamic.len() + self.fixed >= MAX_SOLIDS {
                continue;
            }
            let Some(m) = world.get(&n.id) else { continue };
            let (scale, rot, pos) = m.to_scale_rotation_translation();
            if !scale.is_finite() || !pos.is_finite() || !rot.is_finite() {
                continue;
            }
            let Some((builder, offset)) = shape(primitive, scale) else { continue };
            let falls = !n.pinned && self.dynamic.len() < MAX_DYNAMIC;
            let body = if falls { RigidBodyBuilder::dynamic() } else { RigidBodyBuilder::fixed() }.position(iso(pos, rot.normalize())).build();
            let handle = self.bodies.insert(body);
            let collider = builder.position(offset).density(1.0).friction(0.8).restitution(0.1).collision_groups(groups(G_OBJECT)).build();
            self.colliders.insert_with_parent(collider, handle, &mut self.bodies);
            if falls {
                self.dynamic.push((n.id, handle));
            } else {
                self.fixed += 1;
            }
        }
    }

    /// A capsule on every humanoid bone, fixed in the pose or (unpinned, with everything below it)
    /// free and hung from the bone above by a limited ball joint.
    fn add_ragdoll(&mut self, scene: &Scene, world: &HashMap<NodeId, photocraft_scene3d::glam::Mat4>) {
        // Humanoid bones, parents first, and each one's nearest humanoid ancestor.
        let humanoid: Vec<NodeId> = scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone) && n.humanoid.as_deref().and_then(region).is_some() && scene.is_visible(n.id)).map(|n| n.id).collect();
        if humanoid.is_empty() {
            return;
        }
        let set: std::collections::HashSet<NodeId> = humanoid.iter().copied().collect();
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
        for (&c, &p) in &parent_h {
            if let Some((t, _)) = pose(c) {
                children.entry(p).or_default().push(t);
            }
        }
        // (body, falls, world position, world rotation) of each bone placed so far.
        let mut placed: HashMap<NodeId, (RigidBodyHandle, bool, Vec3, Quat)> = HashMap::new();
        for id in order {
            let Some(node) = scene.node(id) else { continue };
            let Some((member, radius, default_len)) = node.humanoid.as_deref().and_then(region) else { continue };
            let Some((p0, rot)) = pose(id) else { continue };
            let parent = parent_h.get(&id).and_then(|p| placed.get(p)).copied();
            let falls = !node.pinned || parent.is_some_and(|(_, f, _, _)| f);
            let end = match children.get(&id) {
                Some(c) if !c.is_empty() => c.iter().copied().sum::<Vec3>() / c.len() as f32,
                _ => {
                    let dir = parent.map_or(Vec3::Y, |(_, _, pp, _)| (p0 - pp).normalize_or(Vec3::Y));
                    p0 + dir * default_len
                }
            };
            let local_end = rot.inverse() * (end - p0);
            if !local_end.is_finite() || local_end.length() < MIN_SIZE || self.dynamic.len() + self.limbs.len() + self.fixed >= MAX_SOLIDS {
                continue;
            }
            let body = if falls { RigidBodyBuilder::dynamic().linear_damping(0.5).angular_damping(5.0) } else { RigidBodyBuilder::fixed() }.position(iso(p0, rot)).build();
            let handle = self.bodies.insert(body);
            let collider = ColliderBuilder::capsule_from_endpoints(pt(Vec3::ZERO), pt(local_end), radius).density(1000.0).friction(0.8).restitution(0.0).collision_groups(groups(member)).build();
            self.colliders.insert_with_parent(collider, handle, &mut self.bodies);
            if let Some((ph, _, pp, prot)) = parent
                && falls
            {
                let anchor = prot.inverse() * (p0 - pp);
                let mut joint = SphericalJointBuilder::new().local_anchor1(pt(anchor)).local_anchor2(pt(Vec3::ZERO));
                for axis in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
                    joint = joint.limits(axis, [-JOINT_LIMIT, JOINT_LIMIT]);
                }
                self.joints.insert(ph, handle, joint.build(), true);
            }
            placed.insert(id, (handle, falls, p0, rot));
            if falls {
                self.limbs.push(BoneBody { id, handle, has_parent: parent.is_some() });
            } else {
                self.fixed += 1;
            }
        }
    }

    /// How many bodies fall: unpinned solids and released bones.
    pub fn dynamic_count(&self) -> usize {
        self.dynamic.len() + self.limbs.len()
    }

    /// How many spring-bone joints (hair, skirt) swing.
    pub fn spring_count(&self) -> usize {
        self.springs.joint_count()
    }

    /// Nothing falls and nothing swings: running would change nothing.
    pub fn is_inert(&self) -> bool {
        self.dynamic_count() == 0 && self.spring_count() == 0
    }

    /// Advances rapier one fixed step (bodies only; see [`World::step_scene`]).
    pub fn step(&mut self) {
        self.pipeline.step(
            &vector![0.0, -9.81, 0.0],
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.joints,
            &mut self.multibody,
            &mut self.ccd,
            None,
            &(),
            &(),
        );
    }

    /// One fixed step of everything, written into the scene: bodies fall and collide, limbs
    /// turn, and the hair swings.
    pub fn step_scene(&mut self, scene: &mut Scene) {
        self.step();
        self.apply(scene);
        self.springs.step(scene, STEP);
    }

    /// Whether every falling body has come to rest.
    pub fn settled(&self) -> bool {
        let asleep = |h: &RigidBodyHandle| self.bodies.get(*h).is_none_or(RigidBody::is_sleeping);
        self.dynamic.iter().all(|(_, h)| asleep(h)) && self.limbs.iter().all(|l| asleep(&l.handle))
    }

    fn pose_of(&self, h: RigidBodyHandle) -> Option<(Vec3, Quat)> {
        let b = self.bodies.get(h)?;
        let (t, q) = (b.translation(), b.rotation().quaternion());
        let (pos, rot) = (Vec3::new(t.x, t.y, t.z), Quat::from_xyzw(q.i, q.j, q.k, q.w));
        (pos.is_finite() && rot.is_finite()).then_some((pos, rot.normalize()))
    }

    /// Where each falling solid is now: node, world position and turn. Bodies that went
    /// non-finite are left out.
    pub fn poses(&self) -> Vec<(NodeId, Vec3, Quat)> {
        self.dynamic.iter().filter_map(|(id, h)| self.pose_of(*h).map(|(p, r)| (*id, p, r))).collect()
    }

    /// Writes the bodies' places into the scene's nodes. A released bone only turns (it stays
    /// attached to the bone above, so its length cannot change); one with nothing above it moves too.
    pub fn apply(&self, scene: &mut Scene) {
        for (id, pos, rot) in self.poses() {
            let _ = scene.place_world(id, pos, rot);
        }
        for l in &self.limbs {
            let Some((pos, rot)) = self.pose_of(l.handle) else { continue };
            let _ = if l.has_parent { set_world_rotation(scene, l.id, rot) } else { scene.place_world(l.id, pos, rot) };
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_ragdoll;

#[cfg(test)]
mod tests_real;
