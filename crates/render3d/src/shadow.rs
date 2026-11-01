//! Shadow maps for the lights that cast them.
//!
//! Each shadow-casting light gets depth maps of the scene seen from the light: one orthographic map
//! for a sun, one perspective map along a spot's cone, six faces of a cube for a point light. A map
//! holds the distance from the light's plane (view depth), so the test is one subtraction whatever
//! the projection. Like Unity's `shadowBias`, the bias is a depth offset in scene units; a texel-sized
//! normal offset on top keeps flat surfaces from shadowing themselves.

use photocraft_scene3d::LightKind;
use photocraft_scene3d::glam::{Mat4, Vec3, Vec4};

use crate::{Draw, LightW, Prepared};

/// Shadow-casting lights per render; more would cost memory for little (the rest light without shadows).
const MAX_SHADOW_LIGHTS: usize = 4;
/// Texels along one edge of a map.
const SUN_SIZE: usize = 1536;
const SPOT_SIZE: usize = 1024;
const POINT_SIZE: usize = 768;
/// A cube face sees a little more than 90°, so filtering across a seam has data on both sides.
const CUBE_FOV: f32 = 92.0;
/// Virt-a-Mate's fixed biases (its Point Bias slider only reaches point lights) and near planes.
const SPOT_BIAS: f32 = 0.001;
const SUN_BIAS: f32 = 0.02;
const POINT_NEAR: f32 = 0.1;
const SPOT_NEAR: f32 = 0.5;

/// One depth map: view depth of the nearest caster per texel (infinity where nothing is).
struct Face {
    view: Mat4,
    proj: Mat4,
    size: usize,
    depth: Vec<f32>,
    /// World size of one texel: at view depth 1 for a perspective face, absolute for an orthographic one.
    texel: f32,
    ortho: bool,
}

pub(crate) struct ShadowMap {
    faces: Vec<Face>,
    cube: bool,
    /// Suns filter wide (soft shadows), spots and points narrow (hard), as Virt-a-Mate sets them.
    soft: bool,
    bias: f32,
}

/// A mesh vertex in the light's frame: clip position and view depth.
type LVert = (Vec4, f32);

fn up_for(dir: Vec3) -> Vec3 {
    if dir.dot(Vec3::Y).abs() > 0.99 { Vec3::Z } else { Vec3::Y }
}

/// Clips a triangle against the near plane (clip z ≥ 0) into 0, 1 or 2 triangles.
fn clip_near(tri: [LVert; 3]) -> ([[LVert; 3]; 2], usize) {
    let n_in = tri.iter().filter(|v| v.0.z >= 0.0).count();
    match n_in {
        0 => ([tri, tri], 0),
        3 => ([tri, tri], 1),
        _ => {
            let mut poly: Vec<LVert> = Vec::with_capacity(4);
            for i in 0..3 {
                let (a, b) = (tri[i], tri[(i + 1) % 3]);
                let (ia, ib) = (a.0.z >= 0.0, b.0.z >= 0.0);
                if ia {
                    poly.push(a);
                }
                if ia != ib {
                    let t = a.0.z / (a.0.z - b.0.z);
                    poly.push((a.0.lerp(b.0, t), a.1 + (b.1 - a.1) * t));
                }
            }
            match poly.as_slice() {
                [a, b, c] => ([[*a, *b, *c], tri], 1),
                [a, b, c, d] => ([[*a, *b, *c], [*a, *c, *d]], 2),
                _ => ([tri, tri], 0),
            }
        }
    }
}

impl Face {
    /// Renders the casters' depth into a new map; `None` when the memory is not there.
    fn build(view: Mat4, proj: Mat4, size: usize, texel: f32, ortho: bool, casters: &[&Draw]) -> Option<Face> {
        let mut depth: Vec<f32> = Vec::new();
        depth.try_reserve_exact(size * size).ok()?;
        depth.resize(size * size, f32::INFINITY);
        let vp = proj * view;
        let fs = size as f64;
        for d in casters {
            let verts: Vec<LVert> = d.positions.iter().map(|p| (vp * p.extend(1.0), -(view.transform_point3(*p)).z)).collect();
            for &[i0, i1, i2] in d.indices.as_chunks::<3>().0 {
                let (Some(&a), Some(&b), Some(&c)) = (verts.get(i0 as usize), verts.get(i1 as usize), verts.get(i2 as usize)) else { continue };
                let (clipped, n) = clip_near([a, b, c]);
                for t in clipped.iter().take(n) {
                    raster_depth(&mut depth, size, fs, t);
                }
            }
        }
        Some(Face { view, proj, size, depth, texel, ortho })
    }

    /// Whether the texel at `(x, y)` (clamped to the map) holds something nearer than `depth`.
    fn blocked(&self, x: i64, y: i64, depth: f32) -> bool {
        let m = self.size as i64 - 1;
        let (x, y) = (x.clamp(0, m) as usize, y.clamp(0, m) as usize);
        self.depth.get(y * self.size + x).is_some_and(|&s| s < depth)
    }
}

/// Draws one clipped triangle's view depth into the map (nearest wins).
fn raster_depth(depth: &mut [f32], size: usize, fs: f64, t: &[LVert; 3]) {
    let mut s = [(0.0f64, 0.0f64, 0.0f64, 0.0f64); 3]; // x, y, 1/w, depth/w
    for (o, (c, dv)) in s.iter_mut().zip(t) {
        let w = f64::from(c.w);
        if w <= 1e-9 || !w.is_finite() {
            return;
        }
        let (nx, ny) = (f64::from(c.x) / w, f64::from(c.y) / w);
        let (x, y) = ((nx * 0.5 + 0.5) * fs, (0.5 - ny * 0.5) * fs);
        if !x.is_finite() || !y.is_finite() || !dv.is_finite() {
            return;
        }
        *o = (x, y, 1.0 / w, f64::from(*dv) / w);
    }
    let [a, b, c] = s;
    let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if area.abs() < 1e-12 {
        return;
    }
    let max = fs - 1.0;
    let (x0, x1) = (a.0.min(b.0).min(c.0).floor().max(0.0), a.0.max(b.0).max(c.0).ceil().min(max));
    let (y0, y1) = (a.1.min(b.1).min(c.1).floor().max(0.0), a.1.max(b.1).max(c.1).ceil().min(max));
    if x1 < x0 || y1 < y0 {
        return;
    }
    for y in (y0 as usize)..=(y1 as usize) {
        let py = y as f64 + 0.5;
        for x in (x0 as usize)..=(x1 as usize) {
            let px = x as f64 + 0.5;
            let w0 = ((b.0 - px) * (c.1 - py) - (b.1 - py) * (c.0 - px)) / area;
            let w1 = ((c.0 - px) * (a.1 - py) - (c.1 - py) * (a.0 - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let inv = w0 * a.2 + w1 * b.2 + w2 * c.2;
            if inv <= 0.0 {
                continue;
            }
            let z = ((w0 * a.3 + w1 * b.3 + w2 * c.3) / inv) as f32;
            if let Some(slot) = depth.get_mut(y * size + x)
                && z < *slot
            {
                *slot = z;
            }
        }
    }
}

/// Centre and radius of a sphere holding every caster.
fn bounds(casters: &[&Draw]) -> Option<(Vec3, f32)> {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for p in casters.iter().flat_map(|d| d.positions.iter()).filter(|p| p.is_finite()) {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    Some(((lo + hi) * 0.5, ((hi - lo).length() * 0.5).max(1e-3)))
}

/// What one face needs to be built.
struct FaceSpec {
    view: Mat4,
    proj: Mat4,
    size: usize,
    texel: f32,
    ortho: bool,
}

fn specs(l: &LightW, casters: &[&Draw]) -> Option<Vec<FaceSpec>> {
    let range = l.range.max(1e-3);
    match l.kind {
        LightKind::Directional => {
            let (center, r) = bounds(casters)?;
            let eye = center - l.dir * (r * 1.1);
            let view = Mat4::look_at_rh(eye, center, up_for(l.dir));
            let proj = Mat4::orthographic_rh(-r, r, -r, r, 1e-3, r * 2.3);
            Some(vec![FaceSpec { view, proj, size: SUN_SIZE, texel: 2.0 * r / SUN_SIZE as f32, ortho: true }])
        }
        LightKind::Spot { angle } => {
            let a = if angle.is_finite() { angle } else { 45.0 };
            let fov = (a + 4.0).clamp(2.0, 170.0).to_radians();
            let view = Mat4::look_at_rh(l.pos, l.pos + l.dir, up_for(l.dir));
            let proj = Mat4::perspective_rh(fov, 1.0, SPOT_NEAR, range.max(SPOT_NEAR * 2.0));
            Some(vec![FaceSpec { view, proj, size: SPOT_SIZE, texel: 2.0 * (fov * 0.5).tan() / SPOT_SIZE as f32, ortho: false }])
        }
        LightKind::Point => {
            let fov = CUBE_FOV.to_radians();
            let proj = Mat4::perspective_rh(fov, 1.0, POINT_NEAR, range.max(POINT_NEAR * 2.0));
            let texel = 2.0 * (fov * 0.5).tan() / POINT_SIZE as f32;
            Some(
                CUBE_DIRS
                    .iter()
                    .map(|&dir| FaceSpec { view: Mat4::look_at_rh(l.pos, l.pos + dir, up_for(dir)), proj, size: POINT_SIZE, texel, ortho: false })
                    .collect(),
            )
        }
    }
}

/// Cube face directions, in the order [`ShadowMap::face_index`] picks them.
const CUBE_DIRS: [Vec3; 6] = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];

impl ShadowMap {
    fn new(l: &LightW, casters: &[&Draw]) -> Option<ShadowMap> {
        let specs = specs(l, casters)?;
        let build = |s: &FaceSpec| Face::build(s.view, s.proj, s.size, s.texel, s.ortho, casters);
        #[cfg(not(target_arch = "wasm32"))]
        let faces: Vec<Option<Face>> = {
            use rayon::prelude::*;
            specs.par_iter().map(build).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let faces: Vec<Option<Face>> = specs.iter().map(build).collect();
        let faces: Vec<Face> = faces.into_iter().collect::<Option<_>>()?;
        let (soft, bias) = match l.kind {
            LightKind::Directional => (true, SUN_BIAS),
            LightKind::Spot { .. } => (false, SPOT_BIAS),
            LightKind::Point => (false, if l.point_bias.is_finite() { l.point_bias.clamp(0.0, 0.03) } else { 0.001 }),
        };
        Some(ShadowMap { faces, cube: matches!(l.kind, LightKind::Point), soft, bias })
    }

    fn face_index(&self, l: &LightW, pos: Vec3) -> usize {
        if !self.cube {
            return 0;
        }
        let v = pos - l.pos;
        let (ax, ay, az) = (v.x.abs(), v.y.abs(), v.z.abs());
        if ax >= ay && ax >= az {
            usize::from(v.x < 0.0)
        } else if ay >= az {
            2 + usize::from(v.y < 0.0)
        } else {
            4 + usize::from(v.z < 0.0)
        }
    }

    /// How much of the light `pos` loses to casters, 0 (lit) to 1 (fully in shadow). `n` is the unit
    /// normal of the surface at `pos`, on the side the light is on.
    pub(crate) fn occlusion(&self, l: &LightW, pos: Vec3, n: Vec3) -> f32 {
        let Some(face) = self.faces.get(self.face_index(l, pos)) else { return 0.0 };
        let depth_of = |p: Vec3| -face.view.transform_point3(p).z;
        let texel = if face.ortho { face.texel } else { depth_of(pos).max(0.0) * face.texel };
        // Normal offset (shift the lookup off the surface) plus the light's depth bias and a texel.
        let p = pos + n * (texel * 1.5);
        let dv = depth_of(p) - (self.bias + texel);
        let clip = face.proj * face.view * p.extend(1.0);
        if clip.w <= 1e-6 || !clip.w.is_finite() {
            return 0.0;
        }
        let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
        if nx.abs() > 1.0 || ny.abs() > 1.0 || clip.z < 0.0 {
            return 0.0;
        }
        let fs = face.size as f32;
        let (fx, fy) = ((nx * 0.5 + 0.5) * fs - 0.5, (0.5 - ny * 0.5) * fs - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        if self.soft {
            let mut hits = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    hits += i32::from(face.blocked(x0 + dx + i64::from(tx >= 0.5), y0 + dy + i64::from(ty >= 0.5), dv));
                }
            }
            hits as f32 / 9.0
        } else {
            let s = |dx: i64, dy: i64| f32::from(u8::from(face.blocked(x0 + dx, y0 + dy, dv)));
            let top = s(0, 0) * (1.0 - tx) + s(1, 0) * tx;
            let bot = s(0, 1) * (1.0 - tx) + s(1, 1) * tx;
            top * (1.0 - ty) + bot * ty
        }
    }
}

/// Builds the maps of the lights that cast shadows. Only pixel lights do, as in Unity.
pub(crate) fn attach(prep: &mut Prepared) {
    let Prepared { draws, lights, .. } = prep;
    let casters: Vec<&Draw> = draws.iter().filter(|d| !d.lines && !d.blend).collect();
    if casters.is_empty() {
        return;
    }
    for l in lights.iter_mut().filter(|l| l.pixel && l.cast_shadows && l.shadow_strength > 0.0).take(MAX_SHADOW_LIGHTS) {
        l.shadow = ShadowMap::new(l, &casters);
    }
}
