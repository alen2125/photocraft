//! Capsule widths fitted to the skin. A model's own skin says how thick each limb really is: the
//! vertices that belong mostly to a bone, seen in the bone's own axes, lie round it at some
//! distance. The collision capsule takes the width most of them fall within, so a thigh is as
//! thick as this figure's thigh, not as a stock one.

use photocraft_scene3d::glam::{Mat4, Vec3};
use photocraft_scene3d::Node;
use photocraft_scene3d::Scene;

/// Most vertices looked at per bone (a hostile model cannot make this slow).
const MAX_SAMPLES: usize = 200_000;
/// Fewer vertices than this say nothing reliable about the shape.
const MIN_SAMPLES: usize = 40;

/// The radius for the capsule of `node`, which runs from its origin to `local_end` (in the
/// bone's own axes). `default` is used when the model has no skin to measure, and the result
/// stays between half and twice of it, so one odd vertex (a hair strand, a stray triangle)
/// cannot make a limb enormous or paper thin.
pub(crate) fn fit_radius(scene: &Scene, node: &Node, local_end: Vec3, default: f32) -> f32 {
    let Some((asset, idx)) = node.source else { return default };
    let Some(model) = scene.model(asset) else { return default };
    let len = local_end.length();
    if !len.is_finite() || len < 1e-3 || !default.is_finite() || default <= 0.0 {
        return default;
    }
    let axis = local_end / len;
    let mut dists: Vec<f32> = Vec::new();
    'meshes: for mesh in &model.meshes {
        let Some(skin) = mesh.skin.and_then(|s| model.skins.get(s)) else { continue };
        let Some(slot) = skin.joints.iter().position(|&j| j == idx as usize) else { continue };
        let Some(ibm) = skin.inverse_bind.get(slot) else { continue };
        let ibm = Mat4::from_cols_array_2d(ibm);
        for (v, (joints, weights)) in mesh.joints.iter().zip(&mesh.weights).enumerate() {
            let w: f32 = joints.iter().zip(weights).filter(|(j, _)| usize::from(**j) == slot).map(|(_, w)| *w).sum();
            if w < 0.5 {
                continue;
            }
            let Some(p) = mesh.positions.get(v) else { continue };
            let local = ibm.transform_point3(Vec3::from(*p));
            let t = local.dot(axis);
            // Only the shaft of the bone: the ends belong to the joints.
            if !t.is_finite() || t < -0.15 * len || t > 1.15 * len {
                continue;
            }
            let perp = (local - axis * t).length();
            if perp.is_finite() {
                dists.push(perp);
            }
            if dists.len() >= MAX_SAMPLES {
                break 'meshes;
            }
        }
    }
    if dists.len() < MIN_SAMPLES {
        return default;
    }
    dists.sort_by(f32::total_cmp);
    let r = dists.get((dists.len() * 7 / 10).min(dists.len() - 1)).copied().unwrap_or(default);
    r.clamp(default * 0.5, default * 2.0)
}
