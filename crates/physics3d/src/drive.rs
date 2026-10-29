//! What the solver does not do on its own: the anatomical limit of a joint, and a guard against
//! numbers going bad. (The muscles and the controllers that hold a part are motors on rapier
//! joints, set up in `ragdoll.rs`, so the solver sees them together with the whole chain they
//! pull on.) The limit is a push, never a move: it turns the child and everything that hangs
//! from it, as one rigid piece, about the joint by a change of speed (never of place), so contacts
//! with the floor and with other parts always get their say.

use photocraft_scene3d::glam::{Quat, Vec3};
use rapier3d::prelude::*;

use crate::{Part, World};

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

/// How easily the body turns about the world axis `n`: n·I⁻¹·n (zero for a body that cannot move).
fn inv_inertia(b: &RigidBody, n: Vec3) -> f32 {
    let v = b.mass_properties().effective_world_inv_inertia_sqrt * na(n);
    let r = v.dot(&v);
    if r.is_finite() { r.max(0.0) } else { 0.0 }
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
    /// One pass of the joint limits, as turning impulses on the bodies. Each joint looks ahead: it
    /// works out where the relative turn will be at the end of the step at its present speed, and
    /// if that is outside the limits it asks for exactly the speed that lands on the edge (so a
    /// limit holds like a wall, adds no energy, and does not wait for the joint to go through it).
    pub(crate) fn drive(&mut self) {
        let dt = self.params.dt;
        for i in 0..self.parts.len() {
            let part = self.parts[i].clone();
            let (Some(p), Some(limit)) = (part.parent, &part.limit) else { continue };
            let (Some(parent), Some(child)) = (self.bodies.get(self.parts[p].handle), self.bodies.get(part.handle)) else { continue };
            let (pq, cq) = (quat(parent), quat(child));
            let rel = (pq.inverse() * cq).normalize();
            // Where the joint will be at the end of the step if nothing changes its speed.
            let w_rel = pq.inverse() * (vec3(child.angvel()) - vec3(parent.angvel()));
            let ahead = (Quat::from_scaled_axis(w_rel * dt) * rel).normalize();
            let allowed = limit.clamp(part.rest_rel, ahead);
            if !allowed.is_finite() || allowed.angle_between(ahead) <= 1e-4 {
                continue;
            }
            // The turn that brings it back to the edge, in the world, as a change of speed.
            let e = pq * turn(ahead, allowed);
            let (theta, n) = (e.length(), e.normalize_or_zero());
            if n == Vec3::ZERO {
                continue;
            }
            let want = (theta / dt).min(MAX_CORRECTION);
            // A two-body impulse about the axis: the child is turned toward the allowed side
            // and the parent turned back by what the child's push costs it, so nothing is
            // added to the pair's total spin and a part resting on the floor is not thrown.
            let (ic, ip) = (inv_inertia(child, n), inv_inertia(parent, n));
            let total = ic + ip;
            if total > 1e-9 {
                let l = want / total;
                let (hc, hp) = (part.handle, self.parts[p].handle);
                if l.is_finite() {
                    if let Some(b) = self.bodies.get_mut(hc) {
                        b.apply_torque_impulse(na(n * l), true);
                    }
                    if let Some(b) = self.bodies.get_mut(hp) {
                        b.apply_torque_impulse(na(-n * l), true);
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
