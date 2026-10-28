//! Pose controls: the sliders a figure ships with (finger bend, toe curl, tongue…). A slider is a
//! list of joint turns per unit of its value, in the joints' own axes, plus links that add its
//! value to other sliders (a hand's "grasp" drives its thumb's and fingers'). Moving one sets the
//! rotation of every joint the model's sliders touch, so the sliders always agree with each other.

use std::collections::HashMap;

use glam::{EulerRot, Quat, Vec3};
use photocraft_model3d::PoseControlData;
use serde::{Deserialize, Serialize};

use crate::scene::{NodeId, Scene, SceneError};

/// How deep slider links are followed (a link to a link to a link…).
const LINK_DEPTH: usize = 4;

/// One slider's value for one imported model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ControlValue {
    pub asset: u32,
    pub name: String,
    pub value: f32,
}

impl Default for ControlValue {
    fn default() -> Self {
        Self { asset: 0, name: String::new(), value: 0.0 }
    }
}

/// The `order` field of a joint (VaM's rotation order) as the axes in the order they are applied.
fn axis_order(order: u8) -> [usize; 3] {
    match order {
        1 => [0, 2, 1],
        2 => [1, 0, 2],
        3 => [1, 2, 0],
        4 => [2, 0, 1],
        5 => [2, 1, 0],
        _ => [0, 1, 2],
    }
}

/// The turn a joint makes for `deg` degrees about each of its own axes. `frame` is the joint's
/// axes in the file's own (mirrored) space; the result is mirrored into ours.
fn joint_turn(frame: [f32; 4], order: u8, deg: [f32; 3]) -> Quat {
    let f = Quat::from_xyzw(frame[0], frame[1], frame[2], frame[3]);
    let f = if f.is_finite() && f.length_squared() > 1e-12 { f.normalize() } else { Quat::IDENTITY };
    let axes = [f * Vec3::X, f * Vec3::Y, f * Vec3::Z];
    let mut d = Quat::IDENTITY;
    for i in axis_order(order) {
        let a = deg.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
        // The file stores these angles with the opposite handedness to the axes they turn about.
        d = Quat::from_axis_angle(axes.get(i).copied().unwrap_or(Vec3::X), -a.to_radians()) * d;
    }
    // The file's space has its z axis flipped relative to ours: a turn about x or y reverses.
    let m = Quat::from_xyzw(-d.x, -d.y, d.z, d.w);
    if m.is_finite() { m.normalize() } else { Quat::IDENTITY }
}

fn euler_quat(e: [f32; 3]) -> Quat {
    let q = Quat::from_euler(EulerRot::XYZ, e[0].to_radians(), e[1].to_radians(), e[2].to_radians());
    if q.is_finite() { q } else { Quat::IDENTITY }
}

impl Scene {
    /// The stored value of a slider (0 when it was never moved).
    pub fn control_value(&self, asset: u32, name: &str) -> f32 {
        self.controls.iter().find(|c| c.asset == asset && c.name == name).map_or(0.0, |c| c.value)
    }

    /// Sets a slider (clamped to its range) and re-poses the model's joints.
    pub fn set_control(&mut self, asset: u32, name: &str, value: f32) -> Result<(), SceneError> {
        let model = self.model(asset).ok_or_else(|| SceneError::Parse(format!("no model {asset}")))?.clone();
        let Some(c) = model.pose_controls.iter().find(|c| c.name == name) else {
            return Err(SceneError::Parse(format!("no control {name}")));
        };
        let (lo, hi) = if c.min <= c.max { (c.min, c.max) } else { (c.max, c.min) };
        let value = if value.is_finite() { value.clamp(lo, hi) } else { 0.0 };
        self.controls.retain(|x| !(x.asset == asset && x.name == name));
        if value != 0.0 {
            self.controls.push(ControlValue { asset, name: name.to_string(), value });
        }
        self.apply_controls(asset)
    }

    /// Sets every slider of a model back to 0.
    pub fn reset_controls(&mut self, asset: u32) -> Result<(), SceneError> {
        self.controls.retain(|x| x.asset != asset);
        self.apply_controls(asset)
    }

    /// Turns every joint the model's sliders touch to what the slider values say.
    pub fn apply_controls(&mut self, asset: u32) -> Result<(), SceneError> {
        let Some(model) = self.model(asset).cloned() else { return Ok(()) };
        if model.pose_controls.is_empty() {
            return Ok(());
        }
        let own: HashMap<&str, f32> = self.controls.iter().filter(|c| c.asset == asset).map(|c| (c.name.as_str(), c.value)).collect();
        let by_name: HashMap<&str, &PoseControlData> = model.pose_controls.iter().map(|c| (c.name.as_str(), c)).collect();
        // Effective value of each slider: its own plus what the sliders linked to it add.
        let mut eff: HashMap<&str, f32> = own.clone();
        for _ in 0..LINK_DEPTH {
            let mut next = own.clone();
            for (name, v) in &eff {
                if let Some(c) = by_name.get(name) {
                    for (to, mult) in &c.links {
                        if let Some((key, _)) = by_name.get_key_value(to.as_str()) {
                            *next.entry(key).or_insert(0.0) += v * mult;
                        }
                    }
                }
            }
            for (name, v) in next.iter_mut() {
                if let Some(c) = by_name.get(name) {
                    let (lo, hi) = if c.min <= c.max { (c.min, c.max) } else { (c.max, c.min) };
                    *v = if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
                }
            }
            eff = next;
        }
        // Degrees about each axis, per joint.
        let mut deg: HashMap<usize, [f32; 3]> = HashMap::new();
        for c in &model.pose_controls {
            let v = eff.get(c.name.as_str()).copied().unwrap_or(0.0);
            if v == 0.0 {
                continue;
            }
            for r in &c.rot {
                let d = deg.entry(r.node).or_insert([0.0; 3]);
                for (acc, turn) in d.iter_mut().zip(r.deg) {
                    *acc += v * turn;
                }
            }
        }
        let targets: Vec<(NodeId, Quat)> = model
            .control_bones
            .iter()
            .filter_map(|b| {
                let id = self.nodes.iter().find(|n| n.source == Some((asset, b.node as u32)))?.id;
                let turn = joint_turn(b.frame, b.order, deg.get(&b.node).copied().unwrap_or([0.0; 3]));
                Some((id, turn))
            })
            .collect();
        if targets.is_empty() && !model.control_bones.is_empty() {
            return Err(SceneError::Parse("the sliders cannot find the model's joints in this scene; re-import the model".into()));
        }
        for (id, turn) in targets {
            if let Some(n) = self.node_mut(id) {
                let rest = n.rest.as_ref().map_or(Quat::IDENTITY, |r| euler_quat(r.rotation));
                let (x, y, z) = (rest * turn).normalize().to_euler(EulerRot::XYZ);
                n.transform.rotation = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
            }
        }
        Ok(())
    }
}
