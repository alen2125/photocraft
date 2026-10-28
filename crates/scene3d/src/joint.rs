//! Anatomical joint limits in the swing-twist form physics engines use: a joint frame (which
//! way the joint's axes point), a twist range about its X axis and elliptical swing limits about
//! its Y and Z axes. The numbers come from a model file (the VaM figures carry its own), so a
//! shoulder cannot be turned into a pose a body could not hold.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Limits of one joint, relative to the pose the joint was imported in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JointLimit {
    /// The joint frame in the node's own rest axes, as a quaternion `[x, y, z, w]`.
    pub frame: [f32; 4],
    /// Allowed twist about the frame's X axis, degrees (`[low, high]`).
    pub twist: [f32; 2],
    /// Allowed swing about the frame's Y and Z axes, degrees (each side; an ellipse).
    pub swing: [f32; 2],
    /// Off lets the joint move freely (the way out when a pose is meant to break the rules).
    pub enabled: bool,
}

impl Default for JointLimit {
    fn default() -> Self {
        Self { frame: [0.0, 0.0, 0.0, 1.0], twist: [-180.0, 180.0], swing: [180.0, 180.0], enabled: true }
    }
}

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

impl JointLimit {
    fn frame_quat(&self) -> Quat {
        let q = Quat::from_xyzw(self.frame[0], self.frame[1], self.frame[2], self.frame[3]);
        if q.is_finite() && q.length_squared() > 1e-12 { q.normalize() } else { Quat::IDENTITY }
    }

    /// `local` (the node's local rotation) brought inside the limits, given the rotation it had
    /// at rest. A joint that is off, or numbers that are not finite, leave it as it is.
    pub fn clamp(&self, rest: Quat, local: Quat) -> Quat {
        if !self.enabled || !local.is_finite() || !rest.is_finite() {
            return local;
        }
        let f = self.frame_quat();
        // The turn away from rest, seen in the joint's own axes.
        let r = (f.inverse() * (rest.inverse() * local).normalize() * f).normalize();
        let r = if r.w < 0.0 { -r } else { r };
        // Swing-twist about X: twist is the part of `r` about X, swing is what remains.
        let twist = Quat::from_xyzw(r.x, 0.0, 0.0, r.w);
        let twist = if twist.length_squared() > 1e-12 { twist.normalize() } else { Quat::IDENTITY };
        let swing = (r * twist.inverse()).normalize();

        let (lo, hi) = (finite_or(self.twist[0], -180.0).to_radians(), finite_or(self.twist[1], 180.0).to_radians());
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        let angle = 2.0 * twist.x.atan2(twist.w);
        let twist = Quat::from_axis_angle(Vec3::X, angle.clamp(lo, hi));

        let swing = if swing.w < 0.0 { -swing } else { swing };
        let v = Vec3::new(swing.x, swing.y, swing.z);
        let len = v.length();
        let swing = if len > 1e-9 {
            let a = 2.0 * len.atan2(swing.w);
            let rv = v / len * a; // rotation vector: components about Y and Z
            let ly = finite_or(self.swing[0], 180.0).to_radians().abs().max(1e-4);
            let lz = finite_or(self.swing[1], 180.0).to_radians().abs().max(1e-4);
            let k = ((rv.y / ly).powi(2) + (rv.z / lz).powi(2)).sqrt();
            let rv = if k > 1.0 { Vec3::new(0.0, rv.y / k, rv.z / k) } else { Vec3::new(0.0, rv.y, rv.z) };
            let angle = rv.length();
            if angle > 1e-9 { Quat::from_axis_angle(rv / angle, angle) } else { Quat::IDENTITY }
        } else {
            Quat::IDENTITY
        };

        let r = (swing * twist).normalize();
        let out = (rest * (f * r * f.inverse())).normalize();
        if out.is_finite() { out } else { local }
    }
}
