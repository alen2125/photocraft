//! Gravity and collisions for the 3D Pose Editor, on rapier. A [`World`] is built from a scene:
//! pinned solids become fixed obstacles, unpinned ones fall and collide with each other and
//! with the pinned ones (a Plane makes the floor). Steps are a fixed 1/60 s with no randomness,
//! so the same scene always settles the same way. Nothing here panics on bad input: shapes with
//! non-finite or degenerate sizes are left out.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{NodeId, NodeKind, Primitive, Scene};
use rapier3d::na::{Isometry3, Quaternion, Translation3, UnitQuaternion};
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
    /// The falling bodies and the scene node each drives.
    dynamic: Vec<(NodeId, RigidBodyHandle)>,
    fixed: usize,
}

fn finite3(v: Vec3) -> bool {
    v.is_finite()
}

fn iso(pos: Vec3, rot: Quat) -> Isometry3<f32> {
    Isometry3::from_parts(Translation3::new(pos.x, pos.y, pos.z), UnitQuaternion::from_quaternion(Quaternion::new(rot.w, rot.x, rot.y, rot.z)))
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
    /// Builds the world from the scene's visible solids. Pinned solids are fixed; unpinned ones fall.
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
            fixed: 0,
        };
        let world = scene.world_matrices();
        for n in &scene.nodes {
            let NodeKind::Primitive { primitive, .. } = &n.kind else { continue };
            if !scene.is_visible(n.id) || w.dynamic.len() + w.fixed >= MAX_SOLIDS {
                continue;
            }
            let Some(m) = world.get(&n.id) else { continue };
            let (scale, rot, pos) = m.to_scale_rotation_translation();
            if !finite3(scale) || !finite3(pos) || !rot.is_finite() {
                continue;
            }
            let Some((builder, offset)) = shape(primitive, scale) else { continue };
            let falls = !n.pinned && w.dynamic.len() < MAX_DYNAMIC;
            let body = if falls { RigidBodyBuilder::dynamic() } else { RigidBodyBuilder::fixed() }.position(iso(pos, rot.normalize())).build();
            let handle = w.bodies.insert(body);
            w.colliders.insert_with_parent(builder.position(offset).density(1.0).friction(0.8).restitution(0.1).build(), handle, &mut w.bodies);
            if falls {
                w.dynamic.push((n.id, handle));
            } else {
                w.fixed += 1;
            }
        }
        w
    }

    /// How many bodies fall (unpinned solids that made it into the world).
    pub fn dynamic_count(&self) -> usize {
        self.dynamic.len()
    }

    /// Advances one fixed step.
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

    /// Whether every falling body has come to rest.
    pub fn settled(&self) -> bool {
        self.dynamic.iter().all(|(_, h)| self.bodies.get(*h).is_none_or(RigidBody::is_sleeping))
    }

    /// Where each falling body is now: node, world position and turn. Bodies that went non-finite
    /// are left out.
    pub fn poses(&self) -> Vec<(NodeId, Vec3, Quat)> {
        self.dynamic
            .iter()
            .filter_map(|(id, h)| {
                let b = self.bodies.get(*h)?;
                let (t, q) = (b.translation(), b.rotation().quaternion());
                let (pos, rot) = (Vec3::new(t.x, t.y, t.z), Quat::from_xyzw(q.i, q.j, q.k, q.w));
                (pos.is_finite() && rot.is_finite()).then_some((*id, pos, rot.normalize()))
            })
            .collect()
    }

    /// Writes the bodies' places into the scene's nodes.
    pub fn apply(&self, scene: &mut Scene) {
        for (id, pos, rot) in self.poses() {
            let _ = scene.place_world(id, pos, rot);
        }
    }
}

#[cfg(test)]
mod tests;
