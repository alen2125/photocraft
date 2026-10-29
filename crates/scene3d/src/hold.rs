//! Held bones: a bone frozen at a point in the world. While another bone above it is moved, the
//! bones between them turn to keep it there as far as bone lengths and joint limits allow (it
//! ends as near as the body can reach, rather than breaking anything).

use glam::{Quat, Vec3};

use crate::ik::{rotation_of, set_world_rotation, solve_chain};
use crate::scene::{NodeId, NodeKind, Scene, SceneError};

/// How many bones up from a held one are walked looking for the one being dragged.
const MAX_CHAIN: usize = 64;

impl Scene {
    /// World position of a node's origin.
    pub fn world_position(&self, id: NodeId) -> Option<Vec3> {
        let p = self.world_matrix(id)?.transform_point3(Vec3::ZERO);
        p.is_finite().then_some(p)
    }

    /// Freezes a bone where it is now (`true`) or lets it follow its parent again (`false`).
    pub fn set_hold(&mut self, id: NodeId, hold: bool) -> Result<(), SceneError> {
        let at = self.world_position(id);
        let node = self.node_mut(id).ok_or(SceneError::NoSuchNode(id))?;
        node.hold = if hold && matches!(node.kind, NodeKind::Bone) { at.map(|p| p.to_array()) } else { None };
        Ok(())
    }

    /// Freezes a bone's orientation as it is now (`true`) or lets it turn with its parent again.
    pub fn set_hold_rotation(&mut self, id: NodeId, hold: bool) -> Result<(), SceneError> {
        let q = self.world_matrix(id).map(|m| rotation_of(&m));
        let node = self.node_mut(id).ok_or(SceneError::NoSuchNode(id))?;
        node.hold_rot = if hold && matches!(node.kind, NodeKind::Bone) { q.map(|q| q.to_array()) } else { None };
        Ok(())
    }

    /// Switches the hand controls (bones that are not minor) of the scene On or Off in one go:
    /// `None` puts every one On (position and orientation); `Some(names)` puts On only those whose
    /// humanoid name or node name is in the list, and every other one Off.
    pub fn set_all_holds(&mut self, only: Option<&[&str]>) {
        self.set_holds_below(None, only);
    }

    /// Like [`Scene::set_all_holds`], but only for the bones below `root` (all of them when `None`).
    pub fn set_holds_below(&mut self, root: Option<NodeId>, only: Option<&[&str]>) {
        let ids: Vec<(NodeId, bool)> = self
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Bone) && !n.minor)
            .filter(|n| root.is_none_or(|r| self.is_above(r, n.id)))
            .map(|n| (n.id, only.is_none_or(|names| names.iter().any(|w| n.humanoid.as_deref() == Some(*w) || n.name == *w))))
            .collect();
        for (id, on) in ids {
            let _ = self.set_hold(id, on);
            let _ = self.set_hold_rotation(id, on);
        }
    }

    /// Whether `ancestor` is somewhere above `id` (bounded, so a loop cannot hang it).
    fn is_above(&self, ancestor: NodeId, id: NodeId) -> bool {
        let mut cur = self.node(id).and_then(|n| n.parent);
        for _ in 0..MAX_CHAIN {
            match cur {
                Some(p) if p == ancestor => return true,
                Some(p) => cur = self.node(p).and_then(|n| n.parent),
                None => return false,
            }
        }
        false
    }
}

/// Call after `dragged` was moved (`turned` false) or turned by the user (`turned` true). The bone
/// itself, if held, is anchored where it was dropped; every other held bone below it is pulled
/// back to its anchor by turning the bones between the two. A bone that was only moved (pulled by
/// IK) may turn too, unless it is the top bone of its skeleton (turning that would swing the
/// whole body); a bone the user turned is left as they set it.
pub fn enforce_holds(scene: &mut Scene, dragged: NodeId, turned: bool, iterations: usize) -> Result<(), SceneError> {
    if !scene.node(dragged).is_some_and(|n| matches!(n.kind, NodeKind::Bone)) {
        return Ok(());
    }
    if scene.node(dragged).is_some_and(|n| n.hold.is_some()) {
        scene.set_hold(dragged, true)?;
    }
    // A bone the user turned keeps the angle they gave it: that is its new frozen orientation.
    if turned && scene.node(dragged).is_some_and(|n| n.hold_rot.is_some()) {
        scene.set_hold_rotation(dragged, true)?;
    }
    let held: Vec<(NodeId, Vec3)> = scene
        .nodes
        .iter()
        .filter(|n| n.id != dragged && matches!(n.kind, NodeKind::Bone))
        .filter_map(|n| n.hold.map(|a| (n.id, Vec3::from(a))))
        .filter(|(_, a)| a.is_finite())
        .collect();
    let may_turn = !turned && scene.node(dragged).and_then(|n| n.parent).and_then(|p| scene.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Bone));
    // Two rounds: pulling one held bone back can nudge another that shares part of the chain.
    for _ in 0..2 {
        for (id, anchor) in &held {
            let mut chain = Vec::new();
            let mut cur = scene.node(*id).and_then(|n| n.parent);
            let mut found = false;
            while let Some(p) = cur {
                if chain.len() >= MAX_CHAIN {
                    break;
                }
                if p == dragged {
                    found = true;
                    if may_turn {
                        chain.push(p);
                    }
                    break;
                }
                let Some(n) = scene.node(p) else { break };
                if !matches!(n.kind, NodeKind::Bone) {
                    break;
                }
                chain.push(p);
                cur = n.parent;
            }
            if found && !chain.is_empty() {
                solve_chain(scene, *id, *anchor, &chain, iterations)?;
            }
        }
    }
    // Frozen orientations last: pulling a limb into place turned its joints, and the held bones
    // below (and the dragged bone itself, when it was only moved) put their orientation back.
    let frozen: Vec<(NodeId, Quat)> = scene
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Bone))
        .filter(|n| n.id != dragged || !turned)
        .filter(|n| n.id == dragged || scene.is_above(dragged, n.id))
        .filter_map(|n| n.hold_rot.map(|q| (n.id, Quat::from_xyzw(q[0], q[1], q[2], q[3]))))
        .filter(|(_, q)| q.is_finite() && q.length_squared() > 1e-12)
        .collect();
    for (id, q) in frozen {
        set_world_rotation(scene, id, q.normalize())?;
    }
    Ok(())
}
