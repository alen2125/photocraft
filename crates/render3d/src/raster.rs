//! Triangle setup, rasterisation, shading, resolve and ray picking.

use crate::{Draw, Image, LightW, Prepared, RenderError, RenderOptions, TexRef, to_srgb};
use photocraft_scene3d::LightKind;
use photocraft_scene3d::NodeId;
use photocraft_scene3d::glam::{Vec3, Vec4};

/// Most sample steps one hair segment is walked in (a segment that long fills the picture).
const MAX_SEGMENT_STEPS: usize = 8192;

#[derive(Clone, Copy)]
struct Vert {
    clip: Vec4,
    pos: Vec3,
    nrm: Vec3,
    uv: [f32; 2],
}

fn lerp_vert(a: Vert, b: Vert, t: f32) -> Vert {
    Vert {
        clip: a.clip.lerp(b.clip, t),
        pos: a.pos.lerp(b.pos, t),
        nrm: a.nrm.lerp(b.nrm, t),
        uv: [a.uv[0] + (b.uv[0] - a.uv[0]) * t, a.uv[1] + (b.uv[1] - a.uv[1]) * t],
    }
}

/// Clips a triangle against the near plane (clip z ≥ 0) into 0, 1 or 2 triangles.
fn clip_near(tri: [Vert; 3]) -> ([[Vert; 3]; 2], usize) {
    let n_in = tri.iter().filter(|v| v.clip.z >= 0.0).count();
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

/// Rows `y0..y0 + rows` of the sample buffers. Bands are independent, so they can be drawn in
/// parallel, and the result is the same whatever the number of threads.
struct Band<'a> {
    y0: usize,
    rows: usize,
    sw: usize,
    depth: &'a mut [f32],
    color: &'a mut [[f32; 3]],
}

/// Sample rows per band.
const BAND_ROWS: usize = 16;

/// A clipped triangle in screen space, with everything the rasteriser needs.
struct SetupTri {
    draw: usize,
    v: [Vert; 3],
    s: [Screen; 3],
    area: f64,
    /// Inclusive sample ranges the triangle can touch.
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
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

/// Bilinear texture lookup (repeat wrap): linear RGB and alpha.
fn sample(t: Option<&TexRef>, lut: &[f32; 256], uv: [f32; 2]) -> Option<(Vec3, f32)> {
    let t = t?;
    let tex = t.model.textures.get(t.index)?.as_ref()?;
    let (w, h) = (tex.width as i64, tex.height as i64);
    if w == 0 || h == 0 || !uv[0].is_finite() || !uv[1].is_finite() {
        return None;
    }
    let (fx, fy) = (uv[0] * w as f32 - 0.5, uv[1] * h as f32 - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let texel = |x: i64, y: i64| -> (Vec3, f32) {
        let (x, y) = (x.rem_euclid(w) as usize, y.rem_euclid(h) as usize);
        let i = (y * w as usize + x) * 4;
        match tex.rgba.get(i..i + 4) {
            Some(p) => (Vec3::new(lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]), f32::from(p[3]) / 255.0),
            None => (Vec3::ONE, 1.0),
        }
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (a, b, c, e) = (texel(x0, y0), texel(x0 + 1, y0), texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
    let top = (a.0.lerp(b.0, tx), a.1 + (b.1 - a.1) * tx);
    let bot = (c.0.lerp(e.0, tx), c.1 + (e.1 - c.1) * tx);
    Some((top.0.lerp(bot.0, ty), top.1 + (bot.1 - top.1) * ty))
}

/// The pixel's colour, or `None` when its alpha falls under the material's cutoff.
fn shade(d: &Draw, prep: &Prepared, pos: Vec3, mut n: Vec3, uv: [f32; 2], tangents: Option<(Vec3, Vec3)>) -> Option<(Vec3, f32)> {
    let (tex_rgb, tex_a) = sample(d.texture.as_ref(), &prep.lut, uv).unwrap_or((Vec3::ONE, 1.0));
    if let Some(cut) = d.cutoff
        && d.alpha * tex_a < cut
    {
        return None;
    }
    let albedo = d.albedo * tex_rgb;
    let opacity = if d.blend { (d.alpha * tex_a).clamp(0.0, 1.0) } else { 1.0 };
    if d.unlit {
        return Some((albedo + d.emissive, opacity));
    }
    n = n.normalize_or_zero();
    let v = (prep.eye - pos).normalize_or_zero();
    // Two-sided: planes, cut-open solids and thin cloth light correctly from behind.
    if n.dot(v) < 0.0 {
        n = -n;
    }
    // A normal map bends the smooth normal in the surface's own frame (tangent, bitangent, normal):
    // one bend (`normal_scale`) lights the surface, another (`spec_normal_scale`) shapes the shine.
    let normal_map = tangents.and_then(|(t, b)| sample(d.normal_map.as_ref(), &prep.raw, uv).map(|(m, _)| (t, b, m)));
    let bend = |scale: f32| -> Vec3 {
        let Some((t, b, m)) = normal_map else { return n };
        let t = (t - n * n.dot(t)).normalize_or_zero();
        let b = (b - n * n.dot(b) - t * t.dot(b)).normalize_or_zero();
        if t == Vec3::ZERO || b == Vec3::ZERO {
            return n;
        }
        let (x, y, z) = ((m.x * 2.0 - 1.0) * scale, (m.y * 2.0 - 1.0) * scale, (m.z * 2.0 - 1.0).max(0.05));
        let bent = (t * x + b * y + n * z).normalize_or_zero();
        if bent == Vec3::ZERO { n } else { bent }
    };
    let n_light = bend(d.normal_scale);
    let n_shine = if (d.spec_normal_scale - d.normal_scale).abs() < 1e-4 { n_light } else { bend(d.spec_normal_scale) };
    let spec_scale = sample(d.spec_map.as_ref(), &prep.raw, uv).map_or(1.0, |(s, _)| s.x);
    let mut diffuse = prep.ambient;
    let mut spec = Vec3::ZERO;
    for l in &prep.lights {
        let (to_light, atten) = light_vec(l, pos);
        let ndl = n_light.dot(to_light);
        if ndl <= 0.0 || atten <= 0.0 {
            continue;
        }
        diffuse += l.radiance * (ndl * atten);
        let h = (to_light + v).normalize_or_zero();
        spec += l.radiance * (n_shine.dot(h).max(0.0).powf(d.shininess) * d.specular * spec_scale * atten);
    }
    Some((albedo * diffuse + spec + d.emissive, opacity))
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

/// Projects a clipped triangle to the sample grid; `None` when it covers nothing.
fn setup_triangle(draw: usize, tri: &[Vert; 3], sw: usize, sh: usize) -> Option<SetupTri> {
    let (fw, fh) = (sw as f64, sh as f64);
    let (a, b, c) = (to_screen(&tri[0], fw, fh)?, to_screen(&tri[1], fw, fh)?, to_screen(&tri[2], fw, fh)?);
    let area = edge(&b, &c, a.x, a.y);
    if area.abs() < 1e-12 {
        return None;
    }
    let x1 = (a.x.max(b.x).max(c.x).ceil().min(fw - 1.0)).max(-1.0);
    let y1 = (a.y.max(b.y).max(c.y).ceil().min(fh - 1.0)).max(-1.0);
    if x1 < 0.0 || y1 < 0.0 {
        return None;
    }
    let x0 = a.x.min(b.x).min(c.x).floor().max(0.0) as usize;
    let y0 = a.y.min(b.y).min(c.y).floor().max(0.0) as usize;
    Some(SetupTri { draw, v: *tri, s: [a, b, c], area, x0, x1: x1 as usize, y0, y1: y1 as usize })
}

/// The directions along which a triangle's texture coordinates grow: tangent (u) and bitangent
/// (against v, since pictures run top-down and normal maps are green-up).
fn tangent_frame(tri: &[Vert; 3]) -> Option<(Vec3, Vec3)> {
    let (e1, e2) = (tri[1].pos - tri[0].pos, tri[2].pos - tri[0].pos);
    let (du1, dv1) = (tri[1].uv[0] - tri[0].uv[0], tri[1].uv[1] - tri[0].uv[1]);
    let (du2, dv2) = (tri[2].uv[0] - tri[0].uv[0], tri[2].uv[1] - tri[0].uv[1]);
    let det = du1 * dv2 - du2 * dv1;
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let r = 1.0 / det;
    let t = (e1 * dv2 - e2 * dv1) * r;
    let b = (e2 * du1 - e1 * du2) * r;
    (t.is_finite() && b.is_finite()).then_some((t, -b))
}

/// A hair segment in screen space, with the colour of each end.
struct SetupSeg {
    a: Screen,
    b: Screen,
    ca: Vec3,
    cb: Vec3,
    y0: usize,
    y1: usize,
}

/// The colour of one hair vertex: the picture's colour along the strand lit the way Kajiya and Kay
/// lit hair: the light's share is the strand's angle to it (a strand lies in many surfaces), and the
/// highlight is a ring round the strand, sharp where the half vector is square to it.
fn hair_colour(d: &Draw, prep: &Prepared, pos: Vec3, tangent: Vec3, uv: [f32; 2]) -> Vec3 {
    let (tex_rgb, _) = sample(d.texture.as_ref(), &prep.lut, uv).unwrap_or((Vec3::ONE, 1.0));
    let albedo = d.albedo * tex_rgb;
    let t = tangent.normalize_or_zero();
    let v = (prep.eye - pos).normalize_or_zero();
    let mut diffuse = prep.ambient;
    let mut spec = Vec3::ZERO;
    for l in &prep.lights {
        let (to_light, atten) = light_vec(l, pos);
        if atten <= 0.0 {
            continue;
        }
        let tl = t.dot(to_light);
        diffuse += l.radiance * ((1.0 - tl * tl).max(0.0).sqrt() * 0.8 + 0.2) * atten;
        let h = (to_light + v).normalize_or_zero();
        let th = t.dot(h);
        spec += l.radiance * ((1.0 - th * th).max(0.0).sqrt().powf(d.shininess) * d.specular * atten);
    }
    albedo * diffuse + spec
}

/// The screen-space segments of one hair mesh, near-plane clipped, with their colours (made on all cores).
fn setup_lines(prep: &Prepared, d: &Draw, sw: usize, sh: usize) -> Vec<SetupSeg> {
    let clip: Vec<Vec4> = d.positions.iter().map(|p| prep.view_proj * p.extend(1.0)).collect();
    let colour = |(i, p): (usize, &Vec3)| hair_colour(d, prep, *p, d.normals.get(i).copied().unwrap_or(Vec3::Y), d.uvs.get(i).copied().unwrap_or([0.0; 2]));
    let (fw, fh) = (sw as f64, sh as f64);
    let segment = |pair: &[u32; 2], colours: &[Vec3]| -> Option<SetupSeg> {
        let [i0, i1] = *pair;
        let (mut c0, mut c1) = (*clip.get(i0 as usize)?, *clip.get(i1 as usize)?);
        let (mut k0, mut k1) = (*colours.get(i0 as usize)?, *colours.get(i1 as usize)?);
        match (c0.z >= 0.0, c1.z >= 0.0) {
            (false, false) => return None,
            (true, false) => {
                let t = c0.z / (c0.z - c1.z);
                c1 = c0.lerp(c1, t);
                k1 = k0.lerp(k1, t);
            }
            (false, true) => {
                let t = c0.z / (c0.z - c1.z);
                c0 = c0.lerp(c1, t);
                k0 = k0.lerp(k1, t);
            }
            (true, true) => {}
        }
        let screen = |c: Vec4| to_screen(&Vert { clip: c, pos: Vec3::ZERO, nrm: Vec3::ZERO, uv: [0.0; 2] }, fw, fh);
        let (a, b) = (screen(c0)?, screen(c1)?);
        let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
        if hi < 0.0 || lo >= fh || a.x.max(b.x) < 0.0 || a.x.min(b.x) >= fw {
            return None;
        }
        Some(SetupSeg { a, b, ca: k0, cb: k1, y0: lo.floor().max(0.0) as usize, y1: (hi.floor() + 1.0).min(fh - 1.0) as usize })
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        let colours: Vec<Vec3> = d.positions.par_iter().enumerate().map(colour).collect();
        d.indices.as_chunks::<2>().0.par_iter().filter_map(|p| segment(p, &colours)).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let colours: Vec<Vec3> = d.positions.iter().enumerate().map(colour).collect();
        d.indices.as_chunks::<2>().0.iter().filter_map(|p| segment(p, &colours)).collect()
    }
}

/// Walks a hair segment one sample at a time, two samples wide (across its major axis): depth tested
/// and written, the colour lerped along it. A single sample would drop out between neighbours and
/// leave the hair as sparse dots.
fn draw_segment(band: &mut Band, s: &SetupSeg) {
    let (dx, dy) = (s.b.x - s.a.x, s.b.y - s.a.y);
    let steps = (dx.abs().max(dy.abs()).ceil().max(1.0) as usize).min(MAX_SEGMENT_STEPS);
    let (y_lo, y_hi) = (band.y0 as i64, (band.y0 + band.rows) as i64);
    let steep = dy.abs() > dx.abs();
    for k in 0..=steps {
        let t = k as f64 / steps as f64;
        let (x0, y0) = ((s.a.x + dx * t).floor() as i64, (s.a.y + dy * t).floor() as i64);
        let z = s.a.z + (s.b.z - s.a.z) * t;
        if !(0.0..=1.0).contains(&z) {
            continue;
        }
        let colour = s.ca.lerp(s.cb, t as f32).to_array();
        for (x, y) in [(x0, y0), if steep { (x0 + 1, y0) } else { (x0, y0 + 1) }] {
            if x < 0 || x >= band.sw as i64 || y < y_lo || y >= y_hi {
                continue;
            }
            let i = (y - y_lo) as usize * band.sw + x as usize;
            let (Some(dz), Some(dc)) = (band.depth.get_mut(i), band.color.get_mut(i)) else { continue };
            if z as f32 >= *dz {
                continue;
            }
            *dz = z as f32;
            *dc = colour;
        }
    }
}

fn draw_triangle(band: &mut Band, d: &Draw, prep: &Prepared, t: &SetupTri) {
    let [a, b, c] = t.s;
    let tri = &t.v;
    let rows = t.y0.max(band.y0)..=t.y1.min(band.y0 + band.rows - 1);
    let tangents = if d.normal_map.is_some() { tangent_frame(tri) } else { None };
    for y in rows {
        let py = y as f64 + 0.5;
        for x in t.x0..=t.x1 {
            let px = x as f64 + 0.5;
            let w0 = edge(&b, &c, px, py) / t.area;
            let w1 = edge(&c, &a, px, py) / t.area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a.z + w1 * b.z + w2 * c.z;
            let i = (y - band.y0) * band.sw + x;
            let Some(&zd) = band.depth.get(i) else { continue };
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
            let uv = [tri[0].uv[0] * p0 + tri[1].uv[0] * p1 + tri[2].uv[0] * p2, tri[0].uv[1] * p0 + tri[1].uv[1] * p1 + tri[2].uv[1] * p2];
            // A dropped (transparent) texel leaves the depth buffer alone.
            let Some((col, opacity)) = shade(d, prep, pos, nrm, uv, tangents) else { continue };
            if d.blend {
                // Mixed over what is there, and the depth is left alone, so what is behind still draws.
                if opacity > 0.004
                    && let Some(dc) = band.color.get_mut(i)
                {
                    *dc = (Vec3::from(*dc) * (1.0 - opacity) + col * opacity).to_array();
                }
                continue;
            }
            if let (Some(dz), Some(dc)) = (band.depth.get_mut(i), band.color.get_mut(i)) {
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

/// Runs `f` on every band, on all cores where there are threads.
#[cfg(not(target_arch = "wasm32"))]
fn run_bands(bands: Vec<Band>, f: impl Fn(&mut Band) + Sync + Send) {
    use rayon::prelude::*;
    bands.into_par_iter().for_each(|mut b| f(&mut b));
}

#[cfg(target_arch = "wasm32")]
fn run_bands(bands: Vec<Band>, f: impl Fn(&mut Band) + Sync + Send) {
    for mut b in bands {
        f(&mut b);
    }
}

/// The screen-space triangles of one mesh, near-plane clipped, in index order.
fn setup_draw(prep: &Prepared, di: usize, d: &Draw, sw: usize, sh: usize) -> Vec<SetupTri> {
    if d.lines {
        return Vec::new();
    }
    let verts: Vec<Vert> = d
        .positions
        .iter()
        .zip(&d.normals)
        .enumerate()
        .map(|(i, (pos, nrm))| Vert { clip: prep.view_proj * pos.extend(1.0), pos: *pos, nrm: *nrm, uv: d.uvs.get(i).copied().unwrap_or([0.0; 2]) })
        .collect();
    let mut out = Vec::with_capacity(d.indices.len() / 3);
    for &[i0, i1, i2] in d.indices.as_chunks::<3>().0 {
        let (Some(&v0), Some(&v1), Some(&v2)) = (verts.get(i0 as usize), verts.get(i1 as usize), verts.get(i2 as usize)) else { continue };
        let (clipped, count) = clip_near([v0, v1, v2]);
        out.extend(clipped.iter().take(count).filter_map(|c| setup_triangle(di, c, sw, sh)));
    }
    out
}

pub(crate) fn render(prep: &Prepared, opts: &RenderOptions, ss: u32, samples: u64) -> Result<Image, RenderError> {
    let (w, h) = (opts.width as usize, opts.height as usize);
    let ss = ss as usize;
    let (sw, sh) = (w * ss, h * ss);
    let n = usize::try_from(samples).map_err(|_| RenderError::TooBig)?;
    let mut depth = alloc(n, f32::INFINITY)?;
    let mut color = alloc(n, prep.background.to_array())?;

    // Projection and near-plane clipping happen once, meshes in parallel, kept in draw order.
    #[cfg(not(target_arch = "wasm32"))]
    let per_draw: Vec<Vec<SetupTri>> = {
        use rayon::prelude::*;
        prep.draws.par_iter().enumerate().map(|(di, d)| setup_draw(prep, di, d, sw, sh)).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let per_draw: Vec<Vec<SetupTri>> = prep.draws.iter().enumerate().map(|(di, d)| setup_draw(prep, di, d, sw, sh)).collect();
    let tris: Vec<SetupTri> = per_draw.into_iter().flatten().collect();
    // Hair: lines, in parallel by mesh too.
    #[cfg(not(target_arch = "wasm32"))]
    let segs: Vec<SetupSeg> = {
        use rayon::prelude::*;
        prep.draws.par_iter().filter(|d| d.lines).map(|d| setup_lines(prep, d, sw, sh)).collect::<Vec<_>>().into_iter().flatten().collect()
    };
    #[cfg(target_arch = "wasm32")]
    let segs: Vec<SetupSeg> = prep.draws.iter().filter(|d| d.lines).flat_map(|d| setup_lines(prep, d, sw, sh)).collect();

    let bands: Vec<Band> = depth
        .chunks_mut(sw * BAND_ROWS)
        .zip(color.chunks_mut(sw * BAND_ROWS))
        .enumerate()
        .map(|(i, (d, c))| Band { y0: i * BAND_ROWS, rows: d.len() / sw, sw, depth: d, color: c })
        .collect();
    run_bands(bands, |band| {
        // Opaque things first, then what blends over them, each in draw order.
        for blend in [false, true] {
            for t in &tris {
                if t.y1 < band.y0 || t.y0 >= band.y0 + band.rows {
                    continue;
                }
                if let Some(d) = prep.draws.get(t.draw).filter(|d| d.blend == blend) {
                    draw_triangle(band, d, prep, t);
                }
            }
            // The hair goes in with the opaque things.
            if !blend {
                for sg in &segs {
                    if sg.y1 >= band.y0 && sg.y0 < band.y0 + band.rows {
                        draw_segment(band, sg);
                    }
                }
            }
        }
    });
    resolve(&depth, &color, sw, w, h, ss, opts.transparent)
}

fn resolve(depth: &[f32], color: &[[f32; 3]], sw: usize, w: usize, h: usize, ss: usize, transparent: bool) -> Result<Image, RenderError> {
    let mut rgba: Vec<u8> = alloc(w * h * 4, 0u8)?;
    let per = (ss * ss) as f32;
    let row = |y: usize, out: &mut [u8]| {
        for (x, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (mut sum, mut covered) = (Vec3::ZERO, 0u32);
            for sy in 0..ss {
                for sx in 0..ss {
                    let i = (y * ss + sy) * sw + x * ss + sx;
                    if let (Some(c), Some(z)) = (color.get(i), depth.get(i)) {
                        if transparent && !z.is_finite() {
                            continue;
                        }
                        sum += Vec3::from(*c);
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                *px = [0, 0, 0, 0];
                continue;
            }
            let c = sum / covered as f32;
            let alpha = if transparent { covered as f32 / per } else { 1.0 };
            let q = |v: f32| (to_srgb(v) * 255.0 + 0.5) as u8;
            *px = [q(c.x), q(c.y), q(c.z), (alpha * 255.0 + 0.5) as u8];
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, out)| row(y, out));
    }
    #[cfg(target_arch = "wasm32")]
    for (y, out) in rgba.chunks_mut(w * 4).enumerate() {
        row(y, out);
    }
    Ok(Image { width: w as u32, height: h as u32, rgba })
}

/// Nearest triangle hit by the world-space ray, two-sided (Möller–Trumbore).
pub(crate) fn ray_pick(prep: &Prepared, origin: Vec3, dir: Vec3) -> Option<NodeId> {
    let mut best: Option<(f32, NodeId)> = None;
    for d in prep.draws.iter().filter(|d| !d.lines) {
        for tri in d.indices.as_chunks::<3>().0 {
            let get = |i: u32| d.positions.get(i as usize).copied();
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
