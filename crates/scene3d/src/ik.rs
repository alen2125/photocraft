//! Posing: rotate a joint in world space (forward kinematics) or drag a joint to a target by
//! turning the joints above it (inverse kinematics, cyclic coordinate descent). Rotations are
//! the only thing changed, so bone lengths never stretch. A node's [`RotationLimits`] are
//! honoured after every step.

use glam::{EulerRot, Mat4, Quat, Vec3};

use crate::scene::{NodeId, NodeKind, Scene, SceneError};

/// Outcome of [`solve_ik`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IkResult {
    /// How far the effector ended from the target, in scene units.
    pub distance: f32,
    pub iterations: usize,
}

fn euler_quat(e: [f32; 3]) -> Quat {
    let q = Quat::from_euler(EulerRot::XYZ, e[0].to_radians(), e[1].to_radians(), e[2].to_radians());
    if q.is_finite() { q } else { Quat::IDENTITY }
}

pub(crate) fn rotation_of(m: &Mat4) -> Quat {
    let (_, r, _) = m.to_scale_rotation_translation();
    if r.is_finite() && r.length_squared() > 1e-12 { r.normalize() } else { Quat::IDENTITY }
}

/// Sets a node's world rotation exactly as given (its position stays), with no limits applied.
/// For a simulation that enforces its own limits and must not be second-guessed.
pub fn set_world_rotation_unlimited(scene: &mut Scene, id: NodeId, world_rotation: Quat) -> Result<(), SceneError> {
    let parent = scene.node(id).ok_or(SceneError::NoSuchNode(id))?.parent;
    let parent_rot = parent.and_then(|p| scene.world_matrix(p)).map_or(Quat::IDENTITY, |m| rotation_of(&m));
    let local = (parent_rot.inverse() * world_rotation).normalize();
    if !local.is_finite() {
        return Ok(());
    }
    let (x, y, z) = local.to_euler(EulerRot::XYZ);
    if let Some(n) = scene.node_mut(id) {
        n.transform.rotation = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
    }
    Ok(())
}

/// Sets a node's world rotation (its position stays), applying the node's rotation limits.
pub fn set_world_rotation(scene: &mut Scene, id: NodeId, world_rotation: Quat) -> Result<(), SceneError> {
    let parent = scene.node(id).ok_or(SceneError::NoSuchNode(id))?.parent;
    let parent_rot = parent.and_then(|p| scene.world_matrix(p)).map_or(Quat::IDENTITY, |m| rotation_of(&m));
    let mut local = (parent_rot.inverse() * world_rotation).normalize();
    if !local.is_finite() {
        return Ok(());
    }
    // Anatomical limits first, relative to the pose the joint was imported in.
    if let Some(n) = scene.node(id)
        && let Some(joint) = &n.joint
    {
        let rest = n.rest.as_ref().map_or(Quat::IDENTITY, |r| euler_quat(r.rotation));
        local = joint.clamp(rest, local);
    }
    let (x, y, z) = local.to_euler(EulerRot::XYZ);
    let mut euler = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
    if let Some(n) = scene.node_mut(id) {
        if let Some(limits) = &n.limits {
            euler = limits.clamp(euler);
        }
        n.transform.rotation = euler;
    }
    Ok(())
}

/// Rotates a node by `delta` about its own origin, in world space (what dragging a gizmo does).
pub fn rotate_world(scene: &mut Scene, id: NodeId, delta: Quat) -> Result<(), SceneError> {
    if !delta.is_finite() {
        return Ok(());
    }
    let world = scene.world_matrix(id).ok_or(SceneError::NoSuchNode(id))?;
    set_world_rotation(scene, id, (delta.normalize() * rotation_of(&world)).normalize())
}

/// Moves `effector` toward the world position `target` by rotating up to `chain_len` bone
/// ancestors of it (nearest first), up to `iterations` sweeps. Only [`NodeKind::Bone`] ancestors
/// turn: the chain stops at the model root.
pub fn solve_ik(scene: &mut Scene, effector: NodeId, target: Vec3, chain_len: usize, iterations: usize) -> Result<IkResult, SceneError> {
    if !target.is_finite() {
        return Err(SceneError::Parse("IK target is not a finite position".into()));
    }
    let mut chain = Vec::new();
    let mut cur = scene.node(effector).ok_or(SceneError::NoSuchNode(effector))?.parent;
    while let Some(p) = cur {
        if chain.len() >= chain_len.min(32) {
            break;
        }
        let Some(n) = scene.node(p) else { break };
        if !matches!(n.kind, NodeKind::Bone) {
            break;
        }
        chain.push(p);
        cur = n.parent;
    }
    solve_chain(scene, effector, target, &chain, iterations)
}

/// The cyclic-coordinate-descent sweeps over an explicit `chain` of bones (nearest first).
pub(crate) fn solve_chain(scene: &mut Scene, effector: NodeId, target: Vec3, chain: &[NodeId], iterations: usize) -> Result<IkResult, SceneError> {
    let pos = |scene: &Scene, id: NodeId| scene.world_matrix(id).map(|m| m.transform_point3(Vec3::ZERO));
    let mut done = 0;
    for sweep in 0..iterations.min(64) {
        done = sweep + 1;
        for &joint in chain {
            let (Some(pj), Some(pe)) = (pos(scene, joint), pos(scene, effector)) else { continue };
            let (to_e, to_t) = ((pe - pj).normalize_or_zero(), (target - pj).normalize_or_zero());
            if to_e == Vec3::ZERO || to_t == Vec3::ZERO {
                continue;
            }
            let delta = Quat::from_rotation_arc(to_e, to_t);
            let Some(world) = scene.world_matrix(joint) else { continue };
            set_world_rotation(scene, joint, (delta * rotation_of(&world)).normalize())?;
        }
        if pos(scene, effector).is_some_and(|pe| pe.distance(target) < 1e-3) {
            break;
        }
    }
    let distance = pos(scene, effector).map_or(f32::INFINITY, |pe| pe.distance(target));
    Ok(IkResult { distance, iterations: done })
}

/// Moves a node so its origin sits at the world position `target`, by changing its local
/// translation. Used for the top bone of a skeleton (the hips), which has no bone above it to
/// turn.
pub fn translate_to(scene: &mut Scene, id: NodeId, target: Vec3) -> Result<(), SceneError> {
    if !target.is_finite() {
        return Err(SceneError::Parse("target is not a finite position".into()));
    }
    let parent = scene.node(id).ok_or(SceneError::NoSuchNode(id))?.parent;
    let parent_world = parent.and_then(|p| scene.world_matrix(p)).unwrap_or(Mat4::IDENTITY);
    let inv = parent_world.inverse();
    if !inv.to_cols_array().iter().all(|v| v.is_finite()) {
        return Ok(());
    }
    let local = inv.transform_point3(target);
    if let Some(n) = scene.node_mut(id) {
        n.transform.translation = local.to_array();
    }
    Ok(())
}
