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

mod fit;
mod springs;

use std::collections::HashMap;

use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{NodeId, NodeKind, Primitive, Scene, set_world_rotation_unlimited};
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
/// Finger joints touch objects only: they are tiny, overlap their neighbours at every knuckle and
/// would otherwise fight the hand and each other.
const G_FINGER: u32 = 1 << 4;
const ALL: u32 = G_TORSO | G_UPPER | G_LIMB | G_OBJECT | G_FINGER;
/// No body part is lighter than this (kg): a feather-light finger hanging off a hand makes the
/// joint solver jitter.
const MIN_MASS: f32 = 0.15;
/// How quickly a free limb loses speed, before the damping setting scales it.
const LIMB_LINEAR_DAMPING: f32 = 0.2;
const LIMB_ANGULAR_DAMPING: f32 = 1.5;
/// The part of a joint's overshoot taken out each step.
const LIMIT_GAIN: f32 = 0.4;

fn groups(member: u32) -> InteractionGroups {
    let filter = match member {
        G_TORSO => G_LIMB | G_OBJECT,
        G_UPPER => G_UPPER | G_LIMB | G_OBJECT,
        G_FINGER => G_OBJECT,
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
        h if ["Thumb", "Index", "Middle", "Ring", "Little"].iter().any(|f| h.contains(f)) => (G_FINGER, 0.012, 0.04),
        _ => return None,
    })
}

#[derive(Clone, Copy)]
struct BoneBody {
    id: NodeId,
    handle: RigidBodyHandle,
    /// Whether the bone has a bone above it in the ragdoll to hang from.
    has_parent: bool,
}

/// A bone's body as placed in the world, for the bones below it to hang from or ride on.
#[derive(Clone, Copy)]
struct Placed {
    handle: RigidBodyHandle,
    pos: Vec3,
    rot: Quat,
    pos_locked: bool,
    rot_locked: bool,
}

/// An anatomical limit on the ball joint between two bodies. Only joints with nothing held at or
/// below them get one: they are corrected by turning the child with everything below it, as one
/// piece, about the joint.
struct LimitJoint {
    child: RigidBodyHandle,
    parent: RigidBodyHandle,
    limit: photocraft_scene3d::JointLimit,
    /// The child's turn relative to its parent in the pose the world was built from.
    rest: Quat,
    /// The child and every body below it.
    subtree: Vec<RigidBodyHandle>,
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
    /// The ragdoll bodies that are not fixed in the world.
    limbs: Vec<BoneBody>,
    /// Joints that carry anatomical limits, parents before children.
    limits: Vec<LimitJoint>,
    /// Every bone that has a body, parents before children, for writing the pose back.
    order: Vec<BoneBody>,
    /// Multiplies gravity (1 = Earth's).
    gravity: f32,
    /// Multiplies how quickly limbs lose speed (1 = the default).
    damping: f32,
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
            params: IntegrationParameters { dt: STEP, num_solver_iterations: std::num::NonZeroUsize::new(12).unwrap_or(std::num::NonZeroUsize::MIN), ..IntegrationParameters::default() },
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
            limits: Vec::new(),
            order: Vec::new(),
            gravity: 1.0,
            damping: 1.0,
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
            let body = if falls { RigidBodyBuilder::dynamic().ccd_enabled(true) } else { RigidBodyBuilder::fixed() }.position(iso(pos, rot.normalize())).build();
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

    /// A capsule on every humanoid bone. A hand control gets a body of its own: fixed in the world
    /// when its position and orientation are both On, held in only one of the two when just one
    /// is, free otherwise (it then hangs from the bone above by a ball joint). The small joints
    /// (fingers, the model's minor bones) have no body: their capsules ride on the hand or foot
    /// body they belong to, so a hand is one solid thing and cannot shake itself apart.
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
        for &c in &humanoid {
            if let (Some(&p), Some((t, _))) = (parent_h.get(&c), pose(c)) {
                children.entry(p).or_default().push(t);
            }
        }
        // A held bone, and every bone above it, cannot be turned about its joint by the limit.
        let mut blocked: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
        for &id in &humanoid {
            if scene.node(id).is_some_and(|n| !n.minor && (n.hold.is_some() || n.hold_rot.is_some())) {
                let mut cur = Some(id);
                for _ in 0..photocraft_scene3d::MAX_DEPTH {
                    let Some(c) = cur else { break };
                    if !blocked.insert(c) {
                        break;
                    }
                    cur = parent_h.get(&c).copied();
                }
            }
        }
        // Each bone that has a body of its own, as placed so far, and which bodies hang from which.
        let mut placed: HashMap<NodeId, Placed> = HashMap::new();
        let mut below: HashMap<RigidBodyHandle, Vec<RigidBodyHandle>> = HashMap::new();
        let mut pending: Vec<(RigidBodyHandle, RigidBodyHandle, photocraft_scene3d::JointLimit, Quat)> = Vec::new();
        for id in order {
            let Some(node) = scene.node(id) else { continue };
            let Some((member, radius, default_len)) = node.humanoid.as_deref().and_then(region) else { continue };
            let Some((p0, rot)) = pose(id) else { continue };
            // The nearest bone above that has a body: the joint to hang from, or the body to ride on.
            let host = {
                let mut cur = parent_h.get(&id).copied();
                let mut found = None;
                for _ in 0..photocraft_scene3d::MAX_DEPTH {
                    let Some(c) = cur else { break };
                    if let Some(p) = placed.get(&c) {
                        found = Some(*p);
                        break;
                    }
                    cur = parent_h.get(&c).copied();
                }
                found
            };
            let end = match children.get(&id) {
                Some(c) if !c.is_empty() => c.iter().copied().sum::<Vec3>() / c.len() as f32,
                _ => {
                    let dir = host.map_or(Vec3::Y, |h| (p0 - h.pos).normalize_or(Vec3::Y));
                    p0 + dir * default_len
                }
            };
            let local_end = rot.inverse() * (end - p0);
            if !local_end.is_finite() || local_end.length() < MIN_SIZE || self.dynamic.len() + self.limbs.len() + self.fixed >= MAX_SOLIDS {
                continue;
            }
            let volume = std::f32::consts::PI * radius * radius * local_end.length() + 4.0 / 3.0 * std::f32::consts::PI * radius.powi(3);

            // A small joint rides on its host's body.
            if node.minor || member == G_FINGER {
                let Some(h) = host else { continue };
                let (a, b) = (h.rot.inverse() * (p0 - h.pos), h.rot.inverse() * (end - h.pos));
                let density = if volume > 1e-9 { 500.0f32.max(0.02 / volume) } else { 500.0 };
                let collider = ColliderBuilder::capsule_from_endpoints(pt(a), pt(b), radius).density(density).friction(0.8).restitution(0.0).collision_groups(groups(member)).build();
                self.colliders.insert_with_parent(collider, h.handle, &mut self.bodies);
                continue;
            }

            // The capsule is as thick as this figure's own limb (from its skin), not a stock one.
            let radius = fit::fit_radius(scene, node, local_end, radius);
            let volume = std::f32::consts::PI * radius * radius * local_end.length() + 4.0 / 3.0 * std::f32::consts::PI * radius.powi(3);
            let (pos_on, rot_on) = (node.hold.is_some(), node.hold_rot.is_some());
            let fixed = pos_on && rot_on;
            let builder = if fixed {
                RigidBodyBuilder::fixed()
            } else {
                let mut b = RigidBodyBuilder::dynamic().linear_damping(LIMB_LINEAR_DAMPING * self.damping).angular_damping(LIMB_ANGULAR_DAMPING * self.damping).can_sleep(false).ccd_enabled(true);
                if pos_on {
                    b = b.lock_translations();
                }
                if rot_on {
                    b = b.lock_rotations();
                }
                b
            };
            let handle = self.bodies.insert(builder.position(iso(p0, rot)).build());
            let density = if volume > 1e-9 { 1000.0f32.max(MIN_MASS / volume) } else { 1000.0 };
            let collider = ColliderBuilder::capsule_from_endpoints(pt(Vec3::ZERO), pt(local_end), radius).density(density).friction(0.8).restitution(0.0).collision_groups(groups(member)).build();
            self.colliders.insert_with_parent(collider, handle, &mut self.bodies);
            let me = Placed { handle, pos: p0, rot, pos_locked: pos_on, rot_locked: rot_on };
            if let Some(h) = host
                && !(h.pos_locked && h.rot_locked && fixed)
            {
                let anchor = h.rot.inverse() * (p0 - h.pos);
                // A ball joint; the two bodies it joins never collide (their capsules overlap at
                // the joint). The model's anatomical limits are enforced after each step instead
                // of through rapier's per-axis angle limits, which do not hold for combined swings.
                let mut joint = GenericJointBuilder::new(JointAxesMask::LOCKED_SPHERICAL_AXES).contacts_enabled(false).local_anchor1(pt(anchor)).local_anchor2(pt(Vec3::ZERO));
                let limit = node.joint.clone().filter(|j| j.enabled && !blocked.contains(&id));
                if limit.is_none() {
                    for axis in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
                        joint = joint.limits(axis, [-JOINT_LIMIT, JOINT_LIMIT]);
                    }
                }
                self.joints.insert(h.handle, handle, joint.build(), true);
                below.entry(h.handle).or_default().push(handle);
                if let Some(limit) = limit {
                    pending.push((handle, h.handle, limit, (h.rot.inverse() * rot).normalize()));
                }
            }
            placed.insert(id, me);
            let body = BoneBody { id, handle, has_parent: host.is_some() };
            self.order.push(body);
            if fixed {
                self.fixed += 1;
            } else {
                self.limbs.push(body);
            }
        }
        // The bodies below each limited joint, for turning them together.
        for (child, parent, limit, rest) in pending {
            let mut subtree = Vec::new();
            let mut todo = vec![child];
            while let Some(h) = todo.pop() {
                if subtree.len() > MAX_SOLIDS {
                    break;
                }
                subtree.push(h);
                todo.extend(below.get(&h).into_iter().flatten().copied());
            }
            self.limits.push(LimitJoint { child, parent, limit, rest, subtree });
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
            &vector![0.0, -9.81 * self.gravity, 0.0],
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
        self.enforce_limits();
    }

    /// Sets gravity (a multiplier of Earth's, 0 to 5) and how quickly limbs lose speed (a
    /// multiplier of the default, 0.1 to 5). Takes effect at once, also in a running world.
    pub fn set_tuning(&mut self, gravity: f32, damping: f32) {
        self.gravity = if gravity.is_finite() { gravity.clamp(0.0, 5.0) } else { 1.0 };
        self.damping = if damping.is_finite() { damping.clamp(0.1, 5.0) } else { 1.0 };
        for l in &self.limbs {
            if let Some(b) = self.bodies.get_mut(l.handle) {
                b.set_linear_damping(LIMB_LINEAR_DAMPING * self.damping);
                b.set_angular_damping(LIMB_ANGULAR_DAMPING * self.damping);
            }
        }
    }

    /// Brings every limited joint back inside its anatomical range: the child is turned, with
    /// everything below it as one piece, about the joint to the nearest allowed pose, and loses
    /// a part of its speed. Nothing is stretched, so nothing is yanked about afterwards.
    fn enforce_limits(&mut self) {
        let Self { limits, bodies, .. } = self;
        let rot_of = |b: &RigidBody| {
            let q = b.rotation().quaternion();
            Quat::from_xyzw(q.i, q.j, q.k, q.w)
        };
        for l in limits.iter() {
            let (Some(pb), Some(cb)) = (bodies.get(l.parent), bodies.get(l.child)) else { continue };
            let (pq, cq) = (rot_of(pb), rot_of(cb));
            if !pq.is_finite() || !cq.is_finite() {
                continue;
            }
            let (pq, cq) = (pq.normalize(), cq.normalize());
            let rel = (pq.inverse() * cq).normalize();
            let allowed = l.limit.clamp(l.rest, rel);
            if !allowed.is_finite() || allowed.angle_between(rel) <= 1e-3 {
                continue;
            }
            // A part of the error each step, so that a floor or a cube in the way can win the tug of war.
            let r = Quat::IDENTITY.slerp(((pq * allowed).normalize() * cq.inverse()).normalize(), LIMIT_GAIN);
            let t = cb.translation();
            let pivot = Vec3::new(t.x, t.y, t.z);
            let moved: Vec<(RigidBodyHandle, Vec3, Quat, Vec3, Vec3)> = l
                .subtree
                .iter()
                .filter_map(|h| {
                    let b = bodies.get(*h)?;
                    let (p, q, lin, ang) = (b.translation(), rot_of(b), b.linvel(), b.angvel());
                    let pos = pivot + r * (Vec3::new(p.x, p.y, p.z) - pivot);
                    Some((*h, pos, (r * q).normalize(), r * Vec3::new(lin.x, lin.y, lin.z) * 0.7, r * Vec3::new(ang.x, ang.y, ang.z) * 0.7))
                })
                .collect();
            for (h, pos, rot, lin, ang) in moved {
                if let Some(b) = bodies.get_mut(h)
                    && pos.is_finite()
                    && rot.is_finite()
                {
                    b.set_position(iso(pos, rot), true);
                    b.set_linvel(vector![lin.x, lin.y, lin.z], true);
                    b.set_angvel(vector![ang.x, ang.y, ang.z], true);
                }
            }
        }
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
        // Every bone that has a body, parents first: held bones too, so that a bone that stays
        // put in the world is not carried round by the turning bones above it.
        for l in &self.order {
            let Some((pos, rot)) = self.pose_of(l.handle) else { continue };
            // The bodies' own joint limits already hold; clamping again would second-guess them.
            let _ = if l.has_parent { set_world_rotation_unlimited(scene, l.id, rot) } else { scene.place_world(l.id, pos, rot) };
        }
    }
}

#[cfg(test)]
mod testutil;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_ragdoll;

#[cfg(test)]
mod tests_real;

#[cfg(test)]
mod tests_vam;
