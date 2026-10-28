//! VRM spring bones (hair, skirts, accessories): chains of bones whose tips swing under inertia,
//! gravity and drag, pulled back to their rest direction by a stiffness, and kept out of the
//! sphere colliders the file attaches to the body. Verlet integration like the VRM reference
//! implementation, written from the VRM specification.

use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{NodeId, NodeKind, Scene, set_world_rotation};

/// A tip no longer than this from its head cannot be steered; the joint is left alone.
const MIN_LENGTH: f32 = 1e-4;
/// Length given to a chain's last bone, which has no child to point at (VRM's 7 cm).
const LEAF_LENGTH: f32 = 0.07;

struct Joint {
    id: NodeId,
    /// The joint's own rotation when the chain was built (what stiffness pulls back to).
    rest_local: Quat,
    /// The bone's direction in its rest frame.
    axis: Vec3,
    length: f32,
    tip: Vec3,
    prev_tip: Vec3,
}

struct Sphere {
    node: NodeId,
    offset: Vec3,
    radius: f32,
}

struct Chain {
    stiffness: f32,
    gravity: Vec3,
    drag: f32,
    hit_radius: f32,
    joints: Vec<Joint>,
    spheres: Vec<Sphere>,
}

#[derive(Default)]
pub(crate) struct Springs {
    chains: Vec<Chain>,
}

fn finite(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

fn world_of(scene: &Scene, id: NodeId) -> Option<(Vec3, Quat)> {
    let m = scene.world_matrix(id)?;
    let (_, r, t) = m.to_scale_rotation_translation();
    (t.is_finite() && r.is_finite()).then_some((t, r.normalize()))
}

/// The bone nodes of the scene named `name` (a model's node names are how its files refer to them).
fn bone_named(scene: &Scene, name: &str) -> Option<NodeId> {
    scene.nodes.iter().find(|n| matches!(n.kind, NodeKind::Bone) && n.name == name).map(|n| n.id)
}

impl Springs {
    pub(crate) fn new(scene: &Scene) -> Springs {
        let mut out = Springs::default();
        for asset in &scene.assets {
            let Some(model) = scene.model(asset.id) else { continue };
            let name_of = |i: usize| model.nodes.get(i).map(|n| n.name.as_str());
            for group in &model.springs {
                let spheres: Vec<Sphere> = group
                    .collider_groups
                    .iter()
                    .filter_map(|g| model.collider_groups.get(*g))
                    .filter_map(|g| Some((bone_named(scene, name_of(g.node)?)?, g)))
                    .flat_map(|(node, g)| g.spheres.iter().map(move |(o, r)| Sphere { node, offset: Vec3::from(*o), radius: finite(*r, 0.0).max(0.0) }))
                    .collect();
                for root in &group.roots {
                    let Some(root_id) = name_of(*root).and_then(|n| bone_named(scene, n)) else { continue };
                    let joints = build_chain(scene, root_id);
                    if joints.is_empty() {
                        continue;
                    }
                    let g = Vec3::from(group.gravity_dir);
                    out.chains.push(Chain {
                        stiffness: finite(group.stiffness, 0.0).max(0.0),
                        gravity: g.normalize_or_zero() * finite(group.gravity_power, 0.0).max(0.0),
                        drag: finite(group.drag_force, 0.4).clamp(0.0, 1.0),
                        hit_radius: finite(group.hit_radius, 0.0).max(0.0),
                        joints,
                        spheres: spheres.iter().map(|s| Sphere { node: s.node, offset: s.offset, radius: s.radius }).collect(),
                    });
                }
            }
        }
        out
    }

    pub(crate) fn joint_count(&self) -> usize {
        self.chains.iter().map(|c| c.joints.len()).sum()
    }

    /// One fixed step for every chain: swing the tips, then turn each bone to point at its tip.
    pub(crate) fn step(&mut self, scene: &mut Scene, dt: f32) {
        for chain in &mut self.chains {
            let colliders: Vec<(Vec3, f32)> = chain
                .spheres
                .iter()
                .filter_map(|s| {
                    let m = scene.world_matrix(s.node)?;
                    Some((m.transform_point3(s.offset), s.radius + chain.hit_radius))
                })
                .collect();
            for j in &mut chain.joints {
                let Some(parent) = scene.node(j.id).and_then(|n| n.parent) else { continue };
                let (Some((head, _)), Some((_, parent_rot))) = (world_of(scene, j.id), world_of(scene, parent)) else { continue };
                let base = parent_rot * j.rest_local;
                let rest_dir = base * j.axis;
                let mut next = j.tip + (j.tip - j.prev_tip) * (1.0 - chain.drag) + rest_dir * (chain.stiffness * dt) + chain.gravity * dt;
                let settle = |p: Vec3| {
                    let d = (p - head).normalize_or_zero();
                    if d == Vec3::ZERO { head + rest_dir * j.length } else { head + d * j.length }
                };
                next = settle(next);
                for (c, r) in &colliders {
                    let off = next - *c;
                    if off.length() < *r {
                        next = settle(*c + off.normalize_or(rest_dir) * *r);
                    }
                }
                if !next.is_finite() {
                    next = head + rest_dir * j.length;
                }
                j.prev_tip = j.tip;
                j.tip = next;
                let to = (next - head).normalize_or_zero();
                if to != Vec3::ZERO && rest_dir.length() > 0.5 {
                    let _ = set_world_rotation(scene, j.id, Quat::from_rotation_arc(rest_dir.normalize(), to) * base);
                }
            }
        }
    }
}

/// The root bone and every bone below it, parents first, each with its rest direction and length.
fn build_chain(scene: &Scene, root: NodeId) -> Vec<Joint> {
    let mut order = vec![root];
    let mut i = 0;
    while let Some(&cur) = order.get(i) {
        i += 1;
        if order.len() > 512 {
            break;
        }
        order.extend(scene.nodes.iter().filter(|n| n.parent == Some(cur) && matches!(n.kind, NodeKind::Bone)).map(|n| n.id));
    }
    order
        .iter()
        .filter_map(|&id| {
            let node = scene.node(id)?;
            let (head, rot) = world_of(scene, id)?;
            let child = scene.nodes.iter().find(|c| c.parent == Some(id) && matches!(c.kind, NodeKind::Bone));
            let (tip, axis) = match child {
                Some(c) => {
                    let (t, _) = world_of(scene, c.id)?;
                    (t, rot.inverse() * (t - head))
                }
                None => {
                    let (pp, _) = world_of(scene, node.parent?)?;
                    let d = (head - pp).normalize_or_zero();
                    (head + d * LEAF_LENGTH, rot.inverse() * (d * LEAF_LENGTH))
                }
            };
            let length = tip.distance(head);
            if length < MIN_LENGTH || !tip.is_finite() {
                return None;
            }
            let (x, y, z) = (node.transform.rotation[0], node.transform.rotation[1], node.transform.rotation[2]);
            let rest_local = Quat::from_euler(photocraft_scene3d::glam::EulerRot::XYZ, x.to_radians(), y.to_radians(), z.to_radians());
            Some(Joint { id, rest_local, axis: axis.normalize_or_zero(), length, tip, prev_tip: tip })
        })
        .collect()
}
