//! Transform gizmo math: which handle a pointer ray grabs, and what a drag of that handle means.
//! Pure geometry in world space (no pixels, no egui): the editor turns pointer positions into
//! rays and draws the handles; this module decides what they do. Three tools, as in any 3D
//! editor: move along an axis, turn about an axis, scale along an axis or all together.

use glam::{Quat, Vec3};

/// What dragging does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

/// A handle that can be grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// One of the three axes (0 = X, 1 = Y, 2 = Z).
    Axis(usize),
    /// The centre of the scale gizmo: scales on every axis at once.
    Uniform,
}

/// A pointer ray in world space: origin and unit direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// The gizmo's place: where it is, how its axes point and how big it is (world units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gizmo {
    pub origin: Vec3,
    /// Unit axes, world or the node's own.
    pub axes: [Vec3; 3],
    pub size: f32,
}

/// The result of a drag step, relative to where the drag began.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delta {
    /// Move by this world vector.
    Translate(Vec3),
    /// Turn by this rotation about the gizmo's origin.
    Rotate(Quat),
    /// Multiply the node's scale on each local axis by these factors.
    Scale(Vec3),
}

/// A drag in progress (what the pointer grabbed and where).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub mode: GizmoMode,
    pub handle: Handle,
    gizmo: Gizmo,
    /// Where on the axis (translate, scale) or the ring's plane (rotate) the press landed.
    start: Vec3,
    start_t: f32,
    /// Plane through the origin facing the pointer at the press, and the camera's up: the
    /// uniform scale follows the pointer up (bigger) and down (smaller) in it.
    plane_normal: Vec3,
    up: Vec3,
}

const EPS: f32 = 1e-6;
/// How close, as a fraction of the gizmo size, a ray must pass to grab a handle. Generous on
/// purpose: the handles are thin lines, and they should be easy to hit.
const GRAB: f32 = 0.24;
/// Scale handles sit at the tips of the axes: grabbed from this fraction of the way out.
const TIP: f32 = 0.55;

fn ok(v: Vec3) -> bool {
    v.is_finite()
}

/// Distance from a point to a segment, and the point's parameter 0–1 along it.
fn point_segment(p: Vec3, a: Vec3, b: Vec3) -> (f32, f32) {
    let ab = b - a;
    let t = if ab.length_squared() > EPS { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (p.distance(a + ab * t), t)
}

/// Where on the infinite line `(lo, ld)` the ray passes closest: the parameter along the line.
/// `None` when the ray is parallel to it.
pub fn closest_on_line(lo: Vec3, ld: Vec3, ray: Ray) -> Option<f32> {
    let (w, a, b, c) = (lo - ray.origin, ld.dot(ld), ld.dot(ray.dir), ray.dir.dot(ray.dir));
    let denom = a * c - b * b;
    if denom.abs() < EPS {
        return None;
    }
    let (d, e) = (ld.dot(w), ray.dir.dot(w));
    let t = (b * e - c * d) / denom;
    t.is_finite().then_some(t)
}

/// Where the ray meets a plane; `None` when parallel or behind the origin.
pub fn ray_plane(ray: Ray, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = ray.dir.dot(normal);
    if denom.abs() < EPS {
        return None;
    }
    let t = (point - ray.origin).dot(normal) / denom;
    (t.is_finite() && t > 0.0).then(|| ray.origin + ray.dir * t)
}

impl Gizmo {
    /// Whether the numbers are usable (finite, positive size).
    pub fn is_valid(&self) -> bool {
        ok(self.origin) && self.axes.iter().all(|a| ok(*a) && a.length() > 0.5) && self.size.is_finite() && self.size > EPS
    }

    fn tip(&self, axis: usize) -> Vec3 {
        self.origin + self.axes.get(axis).copied().unwrap_or(Vec3::X) * self.size
    }

    /// The handle the ray grabs, if any. Of several, the nearest to the ray wins.
    pub fn pick(&self, mode: GizmoMode, ray: Ray) -> Option<Handle> {
        if !self.is_valid() || !ok(ray.origin) || !ok(ray.dir) {
            return None;
        }
        let reach = self.size * GRAB;
        let mut best: Option<(f32, Handle)> = None;
        let mut offer = |d: f32, h: Handle| {
            if d <= reach && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, h));
            }
        };
        match mode {
            GizmoMode::Translate | GizmoMode::Scale => {
                let from = if mode == GizmoMode::Scale { TIP } else { 0.0 };
                for axis in 0..3 {
                    let a = self.origin + self.axes[axis] * (self.size * from);
                    let (d, _) = segment_ray_distance(a, self.tip(axis), ray);
                    offer(d, Handle::Axis(axis));
                }
                if mode == GizmoMode::Scale {
                    // A centre box scales everything; the axes start further out, so it never competes.
                    let (d, _) = point_segment(self.origin, ray.origin, ray.origin + ray.dir * 1e4);
                    offer(d * 0.8, Handle::Uniform);
                }
            }
            GizmoMode::Rotate => {
                for axis in 0..3 {
                    if let Some(hit) = ray_plane(ray, self.origin, self.axes[axis]) {
                        offer((hit.distance(self.origin) - self.size).abs(), Handle::Axis(axis));
                    }
                }
            }
        }
        best.map(|(_, h)| h)
    }

    /// Starts dragging `handle` with the pointer at `ray`; `up` is the camera's up direction.
    pub fn begin(&self, mode: GizmoMode, handle: Handle, ray: Ray, up: Vec3) -> Option<Drag> {
        if !self.is_valid() || !ok(ray.origin) || !ok(ray.dir) || !ok(up) {
            return None;
        }
        let (start, start_t) = match (mode, handle) {
            (GizmoMode::Rotate, Handle::Axis(a)) => (ray_plane(ray, self.origin, *self.axes.get(a)?)?, 0.0),
            (_, Handle::Axis(a)) => {
                let t = closest_on_line(self.origin, *self.axes.get(a)?, ray)?;
                (self.origin, t)
            }
            (_, Handle::Uniform) => (ray_plane(ray, self.origin, ray.dir)?, 0.0),
        };
        Some(Drag { mode, handle, gizmo: *self, start, start_t, plane_normal: ray.dir, up: up.normalize_or_zero() })
    }
}

/// Distance between a ray and a segment, and the parameter 0–1 on the segment where they are
/// closest.
fn segment_ray_distance(a: Vec3, b: Vec3, ray: Ray) -> (f32, f32) {
    let d = b - a;
    let t = closest_on_line(a, d, ray).map_or(0.0, |t| t.clamp(0.0, 1.0));
    let on_seg = a + d * t;
    // Distance from that point to the ray (in front of the eye only).
    let s = (on_seg - ray.origin).dot(ray.dir).max(0.0);
    (on_seg.distance(ray.origin + ray.dir * s), t)
}

impl Drag {
    /// What the drag amounts to with the pointer now at `ray`; `None` when the ray no longer
    /// reaches the handle's line or plane (nothing to apply this step).
    pub fn update(&self, ray: Ray) -> Option<Delta> {
        let g = &self.gizmo;
        match (self.mode, self.handle) {
            (GizmoMode::Translate, Handle::Axis(a)) => {
                let axis = *g.axes.get(a)?;
                let t = closest_on_line(g.origin, axis, ray)?;
                Some(Delta::Translate(axis * (t - self.start_t)))
            }
            (GizmoMode::Rotate, Handle::Axis(a)) => {
                let axis = *g.axes.get(a)?;
                let now = ray_plane(ray, g.origin, axis)?;
                let (v0, v1) = ((self.start - g.origin).normalize_or_zero(), (now - g.origin).normalize_or_zero());
                if v0 == Vec3::ZERO || v1 == Vec3::ZERO {
                    return None;
                }
                let angle = v0.cross(v1).dot(axis).atan2(v0.dot(v1));
                Some(Delta::Rotate(Quat::from_axis_angle(axis, angle)))
            }
            (GizmoMode::Scale, Handle::Axis(a)) => {
                let axis = *g.axes.get(a)?;
                let t = closest_on_line(g.origin, axis, ray)?;
                // Distances out along the axis, from the gizmo's origin; grabbing at the tip
                // and pulling out to twice the distance doubles the scale.
                let (d0, d1) = (self.start_t.abs().max(g.size * 0.2), t.abs().max(g.size * 0.02));
                let mut f = Vec3::ONE;
                if let Some(slot) = [&mut f.x, &mut f.y, &mut f.z].into_iter().nth(a) {
                    *slot = (d1 / d0).clamp(0.01, 100.0);
                }
                Some(Delta::Scale(f))
            }
            (GizmoMode::Scale, Handle::Uniform) => {
                let now = ray_plane(ray, g.origin, self.plane_normal)?;
                // Pointer up by half the gizmo's size is a factor e; down, 1/e.
                let k = ((now - self.start).dot(self.up) / g.size * 2.0).exp().clamp(0.01, 100.0);
                Some(Delta::Scale(Vec3::splat(k)))
            }
            _ => None,
        }
    }
}
