//! Gravity, collisions, ragdoll bodies and hair for the 3D Pose Editor, on rapier, set up the way
//! VaM sets up its people. A [`World`] is built from a scene:
//!
//! * pinned solids are fixed obstacles; unpinned ones fall and collide (a Plane is the floor);
//! * every hand control (the bones a person moves) is a rigid body with its model's own mass,
//!   drag and collision capsules, hung from the body part above by a ball joint; a muscle at each
//!   joint holds it at its rest pose, and a control that is On holds its part where it was put
//!   by a spring (position and orientation each on their own), as VaM's controllers do (see
//!   `drive.rs`); a control that is Off lets its part go, and gravity lets the body sag;
//! * spring bones (hair, skirts) swing under inertia and gravity and stay out of the body's
//!   colliders.
//!
//! Steps are a fixed 1/90 s with 15 solver iterations, the figures VaM runs at, with no
//! randomness, so the same scene always settles the same way. Nothing here panics on bad input:
//! shapes with non-finite or degenerate sizes are left out.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod drive;
mod fit;
mod ragdoll;
mod springs;
mod vam;

use std::collections::{HashMap, HashSet};

use photocraft_scene3d::model3d::DriveData;
use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{JointLimit, NodeId, NodeKind, Primitive, Scene, set_world_rotation_unlimited};
use rapier3d::na::{Isometry3, Point3, Quaternion, Translation3, UnitQuaternion};
use rapier3d::prelude::*;

/// Fixed simulation step in seconds (VaM's `Fixed Timestep`).
pub const STEP: f32 = 1.0 / 90.0;
/// Most falling bodies and total solids a world takes (more are left out).
pub const MAX_DYNAMIC: usize = 256;
pub const MAX_SOLIDS: usize = 1024;
/// Thinnest a shape may be, in scene units.
const MIN_SIZE: f32 = 1e-3;
/// Thickness of a Plane's collision slab (its top face sits at the plane).
const SLAB: f32 = 0.5;
/// How far a ball joint of a model without anatomical limits may bend on each axis, radians.
const JOINT_LIMIT: f32 = 1.6;

/// VaM's physics settings, used when a model does not carry its own.
const SOLVER_ITERATIONS: u32 = 15;
const FRICTION: f32 = 0.6;
const BOUNCINESS: f32 = 0.0;
const MAX_VELOCITY: f32 = 10.0;
const HOLD: DriveData = DriveData { spring: 1000.0, damper: 50.0, max_force: 10000.0 };

/// A body part: one rigid body for a hand control, and what drives it.
#[derive(Clone)]
pub(crate) struct Part {
    id: NodeId,
    handle: RigidBodyHandle,
    /// The part above it in `World::parts` (it hangs from it), if any.
    parent: Option<usize>,
    /// This part and every part that hangs from it (indices into `World::parts`): what a joint
    /// limit turns as one piece.
    subtree: Vec<usize>,
    /// Centre of mass in the body's own axes.
    com: Vec3,
    start_pos: Vec3,
    start_rot: Quat,
    /// This part's turn relative to the one above it in the pose the world was built from.
    rest_rel: Quat,
    limit: Option<JointLimit>,
    lin_drag: f32,
    ang_drag: f32,
}

/// Pairs of colliders that must not touch.
#[derive(Default)]
pub(crate) struct IgnorePairs(HashSet<(ColliderHandle, ColliderHandle)>);

impl IgnorePairs {
    fn key(a: ColliderHandle, b: ColliderHandle) -> (ColliderHandle, ColliderHandle) {
        if a.into_raw_parts() <= b.into_raw_parts() { (a, b) } else { (b, a) }
    }

    pub(crate) fn insert(&mut self, a: ColliderHandle, b: ColliderHandle) {
        self.0.insert(Self::key(a, b));
    }
}

impl PhysicsHooks for IgnorePairs {
    fn filter_contact_pair(&self, context: &PairFilterContext) -> Option<SolverFlags> {
        if self.0.contains(&Self::key(context.collider1, context.collider2)) { None } else { Some(SolverFlags::COMPUTE_IMPULSES) }
    }
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
    /// Every body part, parents before children.
    parts: Vec<Part>,
    ignore: IgnorePairs,
    /// A body that never moves, which the controllers that are On are jointed to.
    anchor: Option<RigidBodyHandle>,
    solid_count: usize,
    /// Multiplies gravity (1 = Earth's).
    gravity: f32,
    /// Multiplies how quickly limbs lose speed (1 = the default).
    damping: f32,
    friction: f32,
    bounciness: f32,
    max_velocity: f32,
    hold_position: DriveData,
    hold_rotation: DriveData,
    /// How many times the physics went wrong (a number turned bad) and was put back to the start.
    resets: u32,
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
        // The physics settings the model brings (the first one that has any), else VaM's.
        let own = scene.assets.iter().filter_map(|a| scene.model(a.id)).map(|m| &m.physics).find(|p| p.step.is_some() || !p.bodies.is_empty());
        let iterations = own.and_then(|p| p.iterations).unwrap_or(SOLVER_ITERATIONS);
        let mut w = World {
            pipeline: PhysicsPipeline::new(),
            params: IntegrationParameters { dt: STEP, num_solver_iterations: std::num::NonZeroUsize::new(iterations as usize).unwrap_or(std::num::NonZeroUsize::MIN), ..IntegrationParameters::default() },
            islands: IslandManager::new(),
            broad: DefaultBroadPhase::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            joints: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            dynamic: Vec::new(),
            parts: Vec::new(),
            ignore: IgnorePairs::default(),
            anchor: None,
            solid_count: 0,
            gravity: 1.0,
            damping: 1.0,
            friction: own.and_then(|p| p.friction).unwrap_or(FRICTION),
            bounciness: own.and_then(|p| p.bounciness).unwrap_or(BOUNCINESS),
            max_velocity: own.and_then(|p| p.max_velocity).unwrap_or(MAX_VELOCITY),
            hold_position: own.and_then(|p| p.hold_position).unwrap_or(HOLD),
            hold_rotation: own.and_then(|p| p.hold_rotation).unwrap_or(HOLD),
            resets: 0,
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
            if !scene.is_visible(n.id) || self.dynamic.len() + self.solid_count >= MAX_SOLIDS {
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
            let collider = builder.position(offset).density(1.0).friction(self.friction).restitution(self.bounciness).build();
            self.colliders.insert_with_parent(collider, handle, &mut self.bodies);
            if falls {
                self.dynamic.push((n.id, handle));
            } else {
                self.solid_count += 1;
            }
        }
    }

    /// How many bodies fall: unpinned solids and body parts.
    pub fn dynamic_count(&self) -> usize {
        self.dynamic.len() + self.parts.len()
    }

    /// How many spring-bone joints (hair, skirt) swing.
    pub fn spring_count(&self) -> usize {
        self.springs.joint_count()
    }

    /// Nothing falls and nothing swings: running would change nothing.
    pub fn is_inert(&self) -> bool {
        self.dynamic_count() == 0 && self.spring_count() == 0
    }

    /// How many times the physics had to be put back to the start because a number went bad.
    pub fn resets(&self) -> u32 {
        self.resets
    }

    /// Advances rapier one fixed step (bodies only; see [`World::step_scene`]).
    pub fn step(&mut self) {
        self.drive();
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
            &self.ignore,
            &(),
        );
        self.guard();
    }

    /// Sets gravity (a multiplier of Earth's, 0 to 5) and how quickly limbs lose speed (a
    /// multiplier of VaM's drag, 0.1 to 5). Takes effect at once, also in a running world.
    pub fn set_tuning(&mut self, gravity: f32, damping: f32) {
        self.gravity = if gravity.is_finite() { gravity.clamp(0.0, 5.0) } else { 1.0 };
        self.damping = if damping.is_finite() { damping.clamp(0.1, 5.0) } else { 1.0 };
        for p in &self.parts {
            if let Some(b) = self.bodies.get_mut(p.handle) {
                b.set_linear_damping(p.lin_drag * self.damping);
                b.set_angular_damping(p.ang_drag * self.damping);
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

    /// Whether everything that moves has all but stopped.
    pub fn settled(&self) -> bool {
        let slow = |h: &RigidBodyHandle| self.bodies.get(*h).is_none_or(|b| b.is_sleeping() || (b.linvel().norm() < 0.01 && b.angvel().norm() < 0.05));
        self.dynamic.iter().all(|(_, h)| slow(h)) && self.parts.iter().all(|p| slow(&p.handle))
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

    /// Writes the bodies' places into the scene's nodes. A body part only turns (it stays attached
    /// to the part above, so its length cannot change); one with nothing above it moves too.
    pub fn apply(&self, scene: &mut Scene) {
        for (id, pos, rot) in self.poses() {
            let _ = scene.place_world(id, pos, rot);
        }
        for part in &self.parts {
            let Some((pos, rot)) = self.pose_of(part.handle) else { continue };
            // The drives and limits already keep the joints right; clamping again would second-guess them.
            let _ = if part.parent.is_some() { set_world_rotation_unlimited(scene, part.id, rot) } else { scene.place_world(part.id, pos, rot) };
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
