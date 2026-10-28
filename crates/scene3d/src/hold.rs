//! Held bones: a bone frozen at a point in the world. While another bone above it is moved, the
//! bones between them turn to keep it there as far as bone lengths and joint limits allow (it
//! ends as near as the body can reach, rather than breaking anything).

use glam::Vec3;

use crate::ik::solve_chain;
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
    Ok(())
}
