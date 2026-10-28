//! Helpers shared by the ragdoll tests. A figure comes in with every bone Off (it would all fall),
//! so a test first puts the whole body On and then lets go of the part it wants to see move.

use photocraft_scene3d::{NodeId, NodeKind, Scene};

/// Puts every hand control of the scene On (position and orientation): the body stands.
pub(crate) fn stand(scene: &mut Scene) {
    scene.set_all_holds(None);
}

/// Switches `id` and every bone below it Off: that limb is let go.
pub(crate) fn release(scene: &mut Scene, id: NodeId) {
    let below = |scene: &Scene, mut cur: Option<NodeId>| {
        for _ in 0..photocraft_scene3d::MAX_DEPTH {
            match cur {
                Some(c) if c == id => return true,
                Some(c) => cur = scene.node(c).and_then(|n| n.parent),
                None => return false,
            }
        }
        false
    };
    let ids: Vec<NodeId> = scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bone) && below(scene, Some(n.id))).map(|n| n.id).collect();
    for b in ids {
        let _ = scene.set_hold(b, false);
        let _ = scene.set_hold_rotation(b, false);
    }
}
