//! What the solver does not do on its own: the anatomical limit of a joint, and a guard against
//! numbers going bad. (The muscles and the controllers that hold a part are motors on rapier
//! joints, set up in `ragdoll.rs`, so the solver sees them together with the whole chain they
//! pull on.) The limit is a push, never a move: it turns the child and everything that hangs
//! from it, as one rigid piece, about the joint by a change of speed (never of place), so contacts
//! with the floor and with other parts always get their say.

use photocraft_scene3d::glam::{Quat, Vec3};
use rapier3d::prelude::*;

use crate::{Part, World, STEP};

/// How much of a limit's overshoot is taken out per step.
const LIMIT_BETA: f32 = 0.3;
/// The most a limit changes a joint's turning speed in one step (rad/s).
const MAX_CORRECTION: f32 = 20.0;

fn vec3(v: &Vector<f32>) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn quat(b: &RigidBody) -> Quat {
    let q = b.rotation().quaternion();
    let r = Quat::from_xyzw(q.i, q.j, q.k, q.w);
    if r.is_finite() { r.normalize() } else { Quat::IDENTITY }
}

fn na(v: Vec3) -> Vector<f32> {
    vector![v.x, v.y, v.z]
}

/// The turn that takes `from` to `to` as a rotation vector (axis times angle), the short way.
fn turn(from: Quat, to: Quat) -> Vec3 {
    let (axis, angle) = (to * from.inverse()).normalize().to_axis_angle();
    let angle = if angle > std::f32::consts::PI { angle - std::f32::consts::TAU } else { angle };
    if axis.is_finite() && angle.is_finite() { axis * angle } else { Vec3::ZERO }
}

impl World {
    /// One step's worth of joint limits, as turning impulses on the bodies.
    pub(crate) fn drive(&mut self) {
        let dt = STEP;
        for i in 0..self.parts.len() {
            let part = self.parts[i].clone();
            let (Some(p), Some(limit)) = (part.parent, &part.limit) else { continue };
            let (Some(parent), Some(child)) = (self.bodies.get(self.parts[p].handle), self.bodies.get(part.handle)) else { continue };
            let (pq, cq) = (quat(parent), quat(child));
            let rel = (pq.inverse() * cq).normalize();
            let allowed = limit.clamp(part.rest_rel, rel);
            if !allowed.is_finite() || allowed.angle_between(rel) <= 1e-4 {
                continue;
            }
            // The turn that would bring the joint back inside, in the world, and how fast the
            // child already turns about it relative to the part above.
            let e = pq * turn(rel, allowed);
            let (theta, n) = (e.length(), e.normalize_or_zero());
            if n == Vec3::ZERO {
                continue;
            }
            let s = (vec3(child.angvel()) - vec3(parent.angvel())).dot(n);
            let want = LIMIT_BETA * theta / dt;
            if s < want {
                // Everything below the joint changes its turning speed by the same amount about
                // the joint, and its linear speed with it, so nothing is stretched.
                let dw = n * (want - s).min(MAX_CORRECTION);
                let pivot = vec3(child.translation());
                let shifts: Vec<(RigidBodyHandle, Vec3, Vec3)> = part
                    .subtree
                    .iter()
                    .filter_map(|&k| {
                        let q = &self.parts[k];
                        let b = self.bodies.get(q.handle)?;
                        let com = vec3(b.translation()) + quat(b) * q.com;
                        Some((q.handle, dw, dw.cross(com - pivot)))
                    })
                    .collect();
                for (h, dw, dv) in shifts {
                    if let Some(b) = self.bodies.get_mut(h)
                        && dw.is_finite()
                        && dv.is_finite()
                    {
                        b.set_angvel(na(vec3(b.angvel()) + dw), true);
                        b.set_linvel(na(vec3(b.linvel()) + dv), true);
                    }
                }
            }
        }
    }

    /// Slows any part that moves faster than the fastest VaM lets one (10 m/s), and puts
    /// everything back where it started when a number has gone wrong, as VaM resets its physics.
    pub(crate) fn guard(&mut self) {
        let fastest = self.max_velocity;
        let mut broken = false;
        for part in &self.parts {
            let Some(b) = self.bodies.get(part.handle) else { continue };
            let (p, v, w) = (vec3(b.translation()), vec3(b.linvel()), vec3(b.angvel()));
            if !(p.is_finite() && v.is_finite() && w.is_finite() && p.abs().max_element() < 1.0e4) {
                broken = true;
                break;
            }
        }
        if broken {
            self.reset_parts();
            return;
        }
        for part in &self.parts {
            if let Some(b) = self.bodies.get_mut(part.handle) {
                let v = vec3(b.linvel());
                if v.length() > fastest {
                    b.set_linvel(na(v * (fastest / v.length())), false);
                }
                let w = vec3(b.angvel());
                if w.length() > 60.0 {
                    b.set_angvel(na(w * (60.0 / w.length())), false);
                }
            }
        }
    }

    /// Puts every body part back where it was when the world was built, still.
    pub(crate) fn reset_parts(&mut self) {
        self.resets += 1;
        for Part { handle, start_pos, start_rot, .. } in &self.parts {
            if let Some(b) = self.bodies.get_mut(*handle) {
                b.set_position(crate::iso(*start_pos, *start_rot), true);
                b.set_linvel(vector![0.0, 0.0, 0.0], true);
                b.set_angvel(vector![0.0, 0.0, 0.0], true);
            }
        }
    }
}
