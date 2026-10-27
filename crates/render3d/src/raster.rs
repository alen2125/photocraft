//! Triangle setup, rasterisation, shading, resolve and ray picking.

use crate::{Draw, Image, LightW, Prepared, RenderError, RenderOptions, to_srgb};
use photocraft_scene3d::LightKind;
use photocraft_scene3d::NodeId;
use photocraft_scene3d::glam::{Vec3, Vec4};

#[derive(Clone, Copy)]
struct Vert {
    clip: Vec4,
    pos: Vec3,
    nrm: Vec3,
}

fn lerp_vert(a: Vert, b: Vert, t: f32) -> Vert {
    Vert { clip: a.clip.lerp(b.clip, t), pos: a.pos.lerp(b.pos, t), nrm: a.nrm.lerp(b.nrm, t) }
}

/// Clips a triangle against the near plane (clip z ≥ 0) into 0, 1 or 2 triangles.
fn clip_near(tri: [Vert; 3]) -> ([[Vert; 3]; 2], usize) {
    let inside: Vec<bool> = tri.iter().map(|v| v.clip.z >= 0.0).collect();
    let n_in = inside.iter().filter(|b| **b).count();
    match n_in {
        0 => ([tri, tri], 0),
        3 => ([tri, tri], 1),
        _ => {
            // Walk the edges, keeping inside vertices and the plane crossings.
            let mut poly: Vec<Vert> = Vec::with_capacity(4);
            for i in 0..3 {
                let (a, b) = (tri[i], tri[(i + 1) % 3]);
                let (ia, ib) = (a.clip.z >= 0.0, b.clip.z >= 0.0);
                if ia {
                    poly.push(a);
                }
                if ia != ib {
                    let t = a.clip.z / (a.clip.z - b.clip.z);
                    poly.push(lerp_vert(a, b, t));
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

struct Target {
    sw: usize,
    sh: usize,
    depth: Vec<f32>,
    color: Vec<[f32; 3]>,
}

#[derive(Clone, Copy)]
struct Screen {
    x: f64,
    y: f64,
    z: f64,
    inv_w: f64,
}

fn to_screen(v: &Vert, sw: f64, sh: f64) -> Option<Screen> {
    let w = f64::from(v.clip.w);
    if w <= 0.0 || !w.is_finite() {
        return None;
    }
    let (nx, ny, nz) = (f64::from(v.clip.x) / w, f64::from(v.clip.y) / w, f64::from(v.clip.z) / w);
    let s = Screen { x: (nx * 0.5 + 0.5) * sw, y: (1.0 - (ny * 0.5 + 0.5)) * sh, z: nz, inv_w: 1.0 / w };
    (s.x.is_finite() && s.y.is_finite() && s.z.is_finite()).then_some(s)
}

fn edge(a: &Screen, b: &Screen, px: f64, py: f64) -> f64 {
    (px - a.x) * (b.y - a.y) - (py - a.y) * (b.x - a.x)
}

fn shade(d: &Draw, prep: &Prepared, pos: Vec3, mut n: Vec3) -> Vec3 {
    if d.unlit {
        return d.albedo + d.emissive;
    }
    n = n.normalize_or_zero();
    let v = (prep.eye - pos).normalize_or_zero();
    // Two-sided: planes and cut-open solids light correctly from behind.
    if n.dot(v) < 0.0 {
        n = -n;
    }
    let mut diffuse = prep.ambient;
    let mut spec = Vec3::ZERO;
    for l in &prep.lights {
        let (to_light, atten) = light_vec(l, pos);
        let ndl = n.dot(to_light);
        if ndl <= 0.0 || atten <= 0.0 {
            continue;
        }
        diffuse += l.radiance * (ndl * atten);
        let h = (to_light + v).normalize_or_zero();
        spec += l.radiance * (n.dot(h).max(0.0).powf(d.shininess) * d.specular * atten);
    }
    d.albedo * diffuse + spec + d.emissive
}

/// Unit vector from the surface to the light, and the light's strength there (0–1).
fn light_vec(l: &LightW, pos: Vec3) -> (Vec3, f32) {
    match l.kind {
        LightKind::Directional => (-l.dir, 1.0),
        LightKind::Point | LightKind::Spot { .. } => {
            let d = l.pos - pos;
            let dist = d.length();
            if dist < 1e-6 {
                return (Vec3::Y, 0.0);
            }
            let to = d / dist;
            let fall = (1.0 - (dist / l.range).powi(2)).clamp(0.0, 1.0);
            let mut a = fall * fall / (1.0 + dist * dist * 0.1);
            if let LightKind::Spot { angle } = l.kind {
                let half = (if angle.is_finite() { angle.clamp(1.0, 179.0) } else { 45.0 } * 0.5).to_radians();
                let cos_outer = half.cos();
                let cos_inner = (half * 0.8).cos();
                let c = (-to).dot(l.dir);
                a *= ((c - cos_outer) / (cos_inner - cos_outer).max(1e-4)).clamp(0.0, 1.0);
            }
            (to, a)
        }
    }
}

fn draw_triangle(t: &mut Target, d: &Draw, prep: &Prepared, tri: &[Vert; 3]) {
    let (sw, sh) = (t.sw as f64, t.sh as f64);
    let (Some(a), Some(b), Some(c)) = (to_screen(&tri[0], sw, sh), to_screen(&tri[1], sw, sh), to_screen(&tri[2], sw, sh)) else { return };
    let area = edge(&b, &c, a.x, a.y);
    if area.abs() < 1e-12 {
        return;
    }
    let x0 = a.x.min(b.x).min(c.x).floor().max(0.0) as usize;
    let y0 = a.y.min(b.y).min(c.y).floor().max(0.0) as usize;
    let x1 = (a.x.max(b.x).max(c.x).ceil().min(sw - 1.0)).max(-1.0);
    let y1 = (a.y.max(b.y).max(c.y).ceil().min(sh - 1.0)).max(-1.0);
    if x1 < 0.0 || y1 < 0.0 {
        return;
    }
    let (x1, y1) = (x1 as usize, y1 as usize);
    for y in y0..=y1 {
        let py = y as f64 + 0.5;
        for x in x0..=x1 {
            let px = x as f64 + 0.5;
            let w0 = edge(&b, &c, px, py) / area;
            let w1 = edge(&c, &a, px, py) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a.z + w1 * b.z + w2 * c.z;
            let i = y * t.sw + x;
            let Some(&zd) = t.depth.get(i) else { continue };
            if !(0.0..=1.0).contains(&z) || z as f32 >= zd {
                continue;
            }
            let (u0, u1, u2) = (w0 * a.inv_w, w1 * b.inv_w, w2 * c.inv_w);
            let s = u0 + u1 + u2;
            if s <= 0.0 {
                continue;
            }
            let (p0, p1, p2) = ((u0 / s) as f32, (u1 / s) as f32, (u2 / s) as f32);
            let pos = tri[0].pos * p0 + tri[1].pos * p1 + tri[2].pos * p2;
            let nrm = tri[0].nrm * p0 + tri[1].nrm * p1 + tri[2].nrm * p2;
            let col = shade(d, prep, pos, nrm);
            if let (Some(dz), Some(dc)) = (t.depth.get_mut(i), t.color.get_mut(i)) {
                *dz = z as f32;
                *dc = col.to_array();
            }
        }
    }
}

fn alloc<T: Clone>(n: usize, v: T) -> Result<Vec<T>, RenderError> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| RenderError::OutOfMemory)?;
    out.resize(n, v);
    Ok(out)
}

pub(crate) fn render(prep: &Prepared, opts: &RenderOptions, ss: u32, samples: u64) -> Result<Image, RenderError> {
    let (w, h) = (opts.width as usize, opts.height as usize);
    let ss = ss as usize;
    let n = usize::try_from(samples).map_err(|_| RenderError::TooBig)?;
    let mut t = Target { sw: w * ss, sh: h * ss, depth: alloc(n, f32::INFINITY)?, color: alloc(n, prep.background.to_array())? };
    for d in &prep.draws {
        let verts: Vec<Vert> = d
            .mesh
            .positions
            .iter()
            .zip(&d.mesh.normals)
            .map(|(p, nrm)| {
                let pos = d.world.transform_point3(Vec3::from(*p));
                Vert { clip: prep.view_proj * pos.extend(1.0), pos, nrm: d.normal * Vec3::from(*nrm) }
            })
            .collect();
        for &[i0, i1, i2] in d.mesh.indices.as_chunks::<3>().0 {
            let (Some(&v0), Some(&v1), Some(&v2)) = (verts.get(i0 as usize), verts.get(i1 as usize), verts.get(i2 as usize)) else { continue };
            let (clipped, count) = clip_near([v0, v1, v2]);
            for c in clipped.iter().take(count) {
                draw_triangle(&mut t, d, prep, c);
            }
        }
    }
    resolve(&t, w, h, ss, opts.transparent)
}

fn resolve(t: &Target, w: usize, h: usize, ss: usize, transparent: bool) -> Result<Image, RenderError> {
    let mut rgba: Vec<u8> = Vec::new();
    rgba.try_reserve_exact(w * h * 4).map_err(|_| RenderError::OutOfMemory)?;
    let per = (ss * ss) as f32;
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut covered) = (Vec3::ZERO, 0u32);
            for sy in 0..ss {
                for sx in 0..ss {
                    let i = (y * ss + sy) * t.sw + x * ss + sx;
                    if let (Some(c), Some(z)) = (t.color.get(i), t.depth.get(i)) {
                        if transparent && !z.is_finite() {
                            continue;
                        }
                        sum += Vec3::from(*c);
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                rgba.extend([0, 0, 0, 0]);
                continue;
            }
            let c = sum / covered as f32;
            let alpha = if transparent { covered as f32 / per } else { 1.0 };
            let q = |v: f32| (to_srgb(v) * 255.0 + 0.5) as u8;
            rgba.extend([q(c.x), q(c.y), q(c.z), (alpha * 255.0 + 0.5) as u8]);
        }
    }
    Ok(Image { width: w as u32, height: h as u32, rgba })
}

/// Nearest triangle hit by the world-space ray, two-sided (Möller–Trumbore).
pub(crate) fn ray_pick(prep: &Prepared, origin: Vec3, dir: Vec3) -> Option<NodeId> {
    let mut best: Option<(f32, NodeId)> = None;
    for d in &prep.draws {
        for tri in d.mesh.indices.as_chunks::<3>().0 {
            let get = |i: u32| d.mesh.positions.get(i as usize).map(|p| d.world.transform_point3(Vec3::from(*p)));
            let (Some(v0), Some(v1), Some(v2)) = (get(tri[0]), get(tri[1]), get(tri[2])) else { continue };
            let (e1, e2) = (v1 - v0, v2 - v0);
            let p = dir.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-12 {
                continue;
            }
            let inv = 1.0 / det;
            let s = origin - v0;
            let u = s.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = dir.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(q) * inv;
            if t > 0.0 && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, d.node));
            }
        }
    }
    best.map(|(_, id)| id)
}
