//! Links between nodes: a node tied to a target node at a distance and angle, optionally aimed
//! at it. Put a sun on a character's face and turn it round the face, pull it closer, and it
//! keeps shining on that point: the relation is data on the node, worked out whenever the
//! world matrices are, so moving the target carries the linked node with it.
//!
//! The offset uses the same convention as the camera's orbit: yaw turns about the vertical axis
//! (0° is towards +Z), pitch lifts it above the horizontal.

use std::collections::HashMap;

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::{MAX_DEPTH, Node, NodeId, NodeKind, Scene, SceneError, Transform};
use glam::EulerRot;

/// Which axes the orbit angles are measured against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkSpace {
    /// World axes: the node stays on its side of the world as the target turns.
    #[default]
    World,
    /// The target's own axes: the node turns with the target (a light that stays in front of a
    /// face while the head turns).
    Target,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Link {
    pub target: NodeId,
    /// Distance from the target in scene units.
    pub distance: f32,
    /// Orbit angle about the vertical axis, degrees (0 = towards +Z).
    pub yaw: f32,
    /// Orbit angle above the horizontal, degrees.
    pub pitch: f32,
    /// Turn the node's −Z axis (where lights shine) to the target.
    pub look_at: bool,
    pub space: LinkSpace,
}

impl Default for Link {
    fn default() -> Self {
        Self { target: 0, distance: 3.0, yaw: 0.0, pitch: 20.0, look_at: true, space: LinkSpace::World }
    }
}

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

fn rotation_of(m: &Mat4) -> Quat {
    let (_, r, _) = m.to_scale_rotation_translation();
    if r.is_finite() && r.length_squared() > 1e-12 { r.normalize() } else { Quat::IDENTITY }
}

impl Link {
    /// Distance, yaw and pitch made safe to use (finite, distance positive, pitch short of vertical).
    pub fn sanitized(&self) -> (f32, f32, f32) {
        (finite_or(self.distance, 3.0).clamp(0.001, 1.0e6), finite_or(self.yaw, 0.0), finite_or(self.pitch, 0.0).clamp(-89.0, 89.0))
    }

    /// The offset from the target to the node, in the link's space.
    pub fn offset(&self) -> Vec3 {
        let (d, yaw, pitch) = self.sanitized();
        let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
        Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()) * d
    }

    /// A link that keeps a node at world position `pos` relative to the target.
    pub fn from_world(target: NodeId, pos: Vec3, target_world: &Mat4, space: LinkSpace, look_at: bool) -> Link {
        let t = target_world.transform_point3(Vec3::ZERO);
        let mut off = pos - t;
        if space == LinkSpace::Target {
            off = rotation_of(target_world).inverse() * off;
        }
        let d = off.length();
        if !d.is_finite() || d < 1e-5 {
            return Link { target, space, look_at, ..Link::default() };
        }
        Link { target, distance: d, yaw: off.x.atan2(off.z).to_degrees(), pitch: (off.y / d).clamp(-1.0, 1.0).asin().to_degrees().clamp(-89.0, 89.0), look_at, space }
    }

    /// Where the node sits in the world, given the target's world matrix.
    pub fn position(&self, target_world: &Mat4) -> Vec3 {
        let off = self.offset();
        let off = if self.space == LinkSpace::Target { rotation_of(target_world) * off } else { off };
        target_world.transform_point3(Vec3::ZERO) + off
    }

    /// The node's world matrix: at the link's position, aimed at the target when `look_at`, else
    /// keeping the orientation and scale of its own transform.
    pub fn world_matrix(&self, target_world: &Mat4, own: &Transform) -> Mat4 {
        let pos = self.position(target_world);
        let own_m = own.matrix();
        let (scale, own_rot, _) = own_m.to_scale_rotation_translation();
        let scale = if scale.is_finite() { scale } else { Vec3::ONE };
        let mut rot = if own_rot.is_finite() { own_rot.normalize() } else { Quat::IDENTITY };
        if self.look_at {
            let f = (target_world.transform_point3(Vec3::ZERO) - pos).normalize_or_zero();
            if f != Vec3::ZERO {
                let up = if f.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
                let right = f.cross(up).normalize_or_zero();
                if right != Vec3::ZERO {
                    let q = Quat::from_mat3(&Mat3::from_cols(right, right.cross(f), -f));
                    if q.is_finite() {
                        rot = q.normalize();
                    }
                }
            }
        }
        Mat4::from_scale_rotation_translation(scale, rot, pos)
    }
}

/// How deep world-matrix resolution may recurse through parents and link targets.
const RESOLVE_LIMIT: usize = 2 * MAX_DEPTH;

/// The world matrix of `id`, following parents and links; memoised. `None` for an unknown node
/// or a chain deeper than the limit.
pub(crate) fn resolve<'a, F: Fn(NodeId) -> Option<&'a Node>>(find: &F, id: NodeId, memo: &mut HashMap<NodeId, Mat4>, depth: usize) -> Option<Mat4> {
    if let Some(m) = memo.get(&id) {
        return Some(*m);
    }
    if depth > RESOLVE_LIMIT {
        return None;
    }
    let n = find(id)?;
    let parent = match n.parent {
        Some(p) => resolve(find, p, memo, depth + 1)?,
        None => Mat4::IDENTITY,
    };
    let mut m = parent * n.transform.matrix();
    if let Some(l) = &n.link
        && l.target != id
        && let Some(t) = resolve(find, l.target, memo, depth + 1)
    {
        m = l.world_matrix(&t, &n.transform);
    }
    memo.insert(id, m);
    Some(m)
}

impl Scene {
    /// Whether `a` depends on `b`: `b` is one of its ancestors, or the target of its link or of
    /// an ancestor's link, however far. Used to refuse links that would loop.
    pub fn depends_on(&self, a: NodeId, b: NodeId) -> bool {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![a];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            let Some(n) = self.node(cur) else { continue };
            for next in n.parent.into_iter().chain(n.link.as_ref().map(|l| l.target)) {
                if next == b {
                    return true;
                }
                stack.push(next);
            }
        }
        false
    }

    /// Sets or clears a node's link. A target that is missing, is the node itself, or depends on
    /// the node is refused. Clearing keeps the node's own stored transform (see [`Scene::unlink`]
    /// to keep where it is).
    pub fn set_link(&mut self, id: NodeId, link: Option<Link>) -> Result<(), SceneError> {
        if self.node(id).is_none() {
            return Err(SceneError::NoSuchNode(id));
        }
        if let Some(l) = &link {
            if l.target == id {
                return Err(SceneError::BadLink("a node cannot be linked to itself".into()));
            }
            if self.node(l.target).is_none() {
                return Err(SceneError::NoSuchNode(l.target));
            }
            if self.depends_on(l.target, id) {
                return Err(SceneError::BadLink("that link would make the nodes depend on each other".into()));
            }
        }
        if let Some(n) = self.node_mut(id) {
            n.link = link;
        }
        Ok(())
    }

    fn world_pos(&self, id: NodeId) -> Result<Vec3, SceneError> {
        self.world_matrix(id).map(|m| m.transform_point3(Vec3::ZERO)).ok_or(SceneError::NoSuchNode(id))
    }

    /// Links `id` to `target` where the node is now: nothing moves. Lights start aimed at the target.
    pub fn link_to(&mut self, id: NodeId, target: NodeId) -> Result<(), SceneError> {
        let node = self.node(id).ok_or(SceneError::NoSuchNode(id))?;
        let aim = matches!(&node.kind, NodeKind::Light(l) if !matches!(l.kind, crate::scene::LightKind::Point));
        let (space, look_at) = node.link.as_ref().map_or((LinkSpace::World, aim), |l| (l.space, l.look_at));
        let (pos, world) = (self.world_pos(id)?, self.world_matrix(id).ok_or(SceneError::NoSuchNode(id))?);
        let target_world = self.world_matrix(target).ok_or(SceneError::NoSuchNode(target))?;
        let link = Link::from_world(target, pos, &target_world, space, look_at);
        self.set_link(id, Some(link))?;
        // Keep the orientation the node has now when it is not aimed.
        let (x, y, z) = rotation_of(&world).to_euler(EulerRot::XYZ);
        if let Some(n) = self.node_mut(id) {
            n.transform.rotation = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
        }
        Ok(())
    }

    /// Removes the link and bakes the node's current world placement into its own transform, so
    /// it stays where it is.
    pub fn unlink(&mut self, id: NodeId) -> Result<(), SceneError> {
        let node = self.node(id).ok_or(SceneError::NoSuchNode(id))?;
        if node.link.is_none() {
            return Ok(());
        }
        let world = self.world_matrix(id).ok_or(SceneError::NoSuchNode(id))?;
        let parent_world = node.parent.and_then(|p| self.world_matrix(p)).unwrap_or(Mat4::IDENTITY);
        let local = parent_world.inverse() * world;
        if let Some(n) = self.node_mut(id) {
            n.link = None;
            if local.to_cols_array().iter().all(|v| v.is_finite()) {
                let (s, r, t) = local.to_scale_rotation_translation();
                let (x, y, z) = r.normalize().to_euler(EulerRot::XYZ);
                n.transform = Transform { translation: t.to_array(), rotation: [x.to_degrees(), y.to_degrees(), z.to_degrees()], scale: s.to_array() };
            }
        }
        Ok(())
    }

    fn linked(&self, id: NodeId) -> Result<(Link, Mat4), SceneError> {
        let link = self.node(id).ok_or(SceneError::NoSuchNode(id))?.link.clone().ok_or_else(|| SceneError::BadLink("the node is not linked".into()))?;
        let target_world = self.world_matrix(link.target).ok_or(SceneError::NoSuchNode(link.target))?;
        Ok((link, target_world))
    }

    /// Re-aims a link so the node sits at world position `pos` (a drag): distance and angles follow.
    pub fn set_link_from_world(&mut self, id: NodeId, pos: Vec3) -> Result<(), SceneError> {
        if !pos.is_finite() {
            return Err(SceneError::BadLink("position is not finite".into()));
        }
        let (link, target_world) = self.linked(id)?;
        let new = Link::from_world(link.target, pos, &target_world, link.space, link.look_at);
        if let Some(n) = self.node_mut(id) {
            n.link = Some(new);
        }
        Ok(())
    }

    /// Swings the node round its target by `q` (a rotation about the target's position).
    pub fn orbit_link(&mut self, id: NodeId, q: Quat) -> Result<(), SceneError> {
        if !q.is_finite() {
            return Ok(());
        }
        let (link, target_world) = self.linked(id)?;
        let t = target_world.transform_point3(Vec3::ZERO);
        let pos = link.position(&target_world);
        self.set_link_from_world(id, t + q.normalize() * (pos - t))
    }

    /// Multiplies the distance to the target.
    pub fn scale_link_distance(&mut self, id: NodeId, factor: f32) -> Result<(), SceneError> {
        let (link, _) = self.linked(id)?;
        let f = finite_or(factor, 1.0);
        if let Some(n) = self.node_mut(id)
            && let Some(l) = &mut n.link
        {
            l.distance = (link.sanitized().0 * f).clamp(0.001, 1.0e6);
        }
        Ok(())
    }

    /// Drops links whose target no longer exists (a node was removed, or a file was edited).
    pub(crate) fn drop_dangling_links(&mut self) {
        let ids: std::collections::HashSet<NodeId> = self.nodes.iter().map(|n| n.id).collect();
        for n in &mut self.nodes {
            if n.link.as_ref().is_some_and(|l| !ids.contains(&l.target) || l.target == n.id) {
                n.link = None;
            }
        }
    }
}
