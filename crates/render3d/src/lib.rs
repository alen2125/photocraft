//! Software renderer for `photocraft-scene3d` scenes: z-buffered triangle rasteriser with
//! perspective-correct normals, Blinn–Phong shading for directional, point and spot lights,
//! supersampled anti-aliasing and an optional transparent background (for rendering into a
//! layer). Deterministic, allocation-bounded and free of GPU and OS calls, so it runs in the
//! headless CLI, the tests and the browser build alike. A wgpu path can later replace the inner
//! loop without changing this API.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod raster;

use photocraft_scene3d::glam::{Mat3, Mat4, Vec3, Vec4};
use photocraft_scene3d::{Light, LightKind, Material, Mesh, NodeId, NodeKind, Scene};

/// Largest output edge in pixels.
pub const MAX_EDGE: u32 = 16_384;
/// Most colour samples (pixels × supersampling²) one render may allocate.
pub const MAX_SAMPLES: u64 = 32 * 1024 * 1024;
const MAX_LIGHTS: usize = 8;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RenderError {
    #[error("image size {0}×{1} is outside 1–{MAX_EDGE} pixels")]
    BadSize(u32, u32),
    #[error("render needs more than {MAX_SAMPLES} samples; lower the size or the anti-aliasing")]
    TooBig,
    #[error("not enough memory for the render buffers")]
    OutOfMemory,
}

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel edge (1–4); 2 means 4 samples per pixel.
    pub ssaa: u32,
    /// Leave the sky out: uncovered pixels get alpha 0.
    pub transparent: bool,
}

impl RenderOptions {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, ssaa: 2, transparent: false }
    }
}

/// 8-bit sRGB RGBA, straight (non-premultiplied) alpha, row-major from the top-left.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn to_linear(c: f32) -> f32 {
    let c = if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 };
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub(crate) fn to_srgb(c: f32) -> f32 {
    let c = if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 };
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn lin3(c: [f32; 3]) -> Vec3 {
    Vec3::new(to_linear(c[0]), to_linear(c[1]), to_linear(c[2]))
}

pub(crate) struct Draw {
    pub node: NodeId,
    pub mesh: Mesh,
    pub world: Mat4,
    pub normal: Mat3,
    pub albedo: Vec3,
    pub emissive: Vec3,
    pub shininess: f32,
    pub specular: f32,
    pub unlit: bool,
}

pub(crate) struct LightW {
    pub kind: LightKind,
    /// Colour × intensity, linear.
    pub radiance: Vec3,
    pub pos: Vec3,
    /// Direction the light travels (unit).
    pub dir: Vec3,
    pub range: f32,
}

pub(crate) struct Prepared {
    pub draws: Vec<Draw>,
    pub lights: Vec<LightW>,
    pub ambient: Vec3,
    pub background: Vec3,
    pub eye: Vec3,
    pub view_proj: Mat4,
}

fn material_draw(node: NodeId, mesh: Mesh, world: Mat4, m: &Material) -> Option<Draw> {
    if !world.to_cols_array().iter().all(|v| v.is_finite()) || world.determinant().abs() < 1e-18 {
        return None;
    }
    let normal = Mat3::from_mat4(world).inverse().transpose();
    if !normal.to_cols_array().iter().all(|v| v.is_finite()) {
        return None;
    }
    let sh = if m.shininess.is_finite() { m.shininess.clamp(1.0, 512.0) } else { 32.0 };
    let sp = if m.specular.is_finite() { m.specular.clamp(0.0, 1.0) } else { 0.0 };
    Some(Draw { node, mesh, world, normal, albedo: lin3(m.color), emissive: lin3(m.emissive), shininess: sh, specular: sp, unlit: m.unlit })
}

fn light_world(scene: &Scene, id: NodeId, l: &Light) -> Option<LightW> {
    let world = scene.world_matrix(id)?;
    let pos = world.transform_point3(Vec3::ZERO);
    let dir = world.transform_vector3(Vec3::NEG_Z).normalize_or_zero();
    if !pos.is_finite() || dir == Vec3::ZERO && !matches!(l.kind, LightKind::Point) {
        return None;
    }
    let k = if l.intensity.is_finite() { l.intensity.max(0.0) } else { 0.0 };
    let range = if l.range.is_finite() { l.range.max(1e-3) } else { 20.0 };
    Some(LightW { kind: l.kind, radiance: lin3(l.color) * k, pos, dir, range })
}

pub(crate) fn prepare(scene: &Scene, aspect: f32) -> Prepared {
    let mut draws = Vec::new();
    let mut lights = Vec::new();
    for n in &scene.nodes {
        if !scene.is_visible(n.id) {
            continue;
        }
        match &n.kind {
            NodeKind::Primitive { primitive, material } => {
                if let Some(world) = scene.world_matrix(n.id)
                    && let Some(d) = material_draw(n.id, primitive.mesh(), world, material)
                {
                    draws.push(d);
                }
            }
            NodeKind::Light(l) if lights.len() < MAX_LIGHTS => {
                if let Some(lw) = light_world(scene, n.id, l) {
                    lights.push(lw);
                }
            }
            _ => {}
        }
    }
    let c = &scene.camera;
    let finite = |v: [f32; 3]| v.iter().all(|x| x.is_finite());
    let (eye, target) = if finite(c.position) && finite(c.target) { (Vec3::from(c.position), Vec3::from(c.target)) } else { (Vec3::new(3.0, 2.0, 5.0), Vec3::ZERO) };
    let mut fwd = target - eye;
    if fwd.length() < 1e-6 {
        fwd = Vec3::NEG_Z;
    }
    let fwd = fwd.normalize();
    let up = if fwd.dot(Vec3::Y).abs() > 0.999 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_at_rh(eye, eye + fwd, up);
    let fov = if c.fov.is_finite() { c.fov.clamp(1.0, 170.0) } else { 45.0 };
    let near = if c.near.is_finite() { c.near.clamp(1e-4, 1e6) } else { 0.05 };
    let far = if c.far.is_finite() { c.far.max(near * 2.0).min(1e7) } else { 200.0 };
    let proj = Mat4::perspective_rh(fov.to_radians(), aspect, near, far);
    Prepared {
        draws,
        lights,
        ambient: lin3(scene.ambient),
        background: lin3(scene.background),
        eye,
        view_proj: proj * view,
    }
}

fn check(opts: &RenderOptions) -> Result<(u32, u64), RenderError> {
    let (w, h) = (opts.width, opts.height);
    if w == 0 || h == 0 || w > MAX_EDGE || h > MAX_EDGE {
        return Err(RenderError::BadSize(w, h));
    }
    let ss = opts.ssaa.clamp(1, 4);
    let samples = u64::from(w) * u64::from(ss) * u64::from(h) * u64::from(ss);
    if samples > MAX_SAMPLES {
        return Err(RenderError::TooBig);
    }
    Ok((ss, samples))
}

/// Renders the scene through its camera.
pub fn render(scene: &Scene, opts: &RenderOptions) -> Result<Image, RenderError> {
    let (ss, samples) = check(opts)?;
    let prep = prepare(scene, opts.width as f32 / opts.height as f32);
    raster::render(&prep, opts, ss, samples)
}

/// The node under pixel `(x, y)` of a `width`×`height` render, or `None` for empty space.
pub fn pick(scene: &Scene, width: u32, height: u32, x: f32, y: f32) -> Option<NodeId> {
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let prep = prepare(scene, width as f32 / height as f32);
    let inv = prep.view_proj.inverse();
    if !inv.to_cols_array().iter().all(|v| v.is_finite()) {
        return None;
    }
    let (nx, ny) = (x / width as f32 * 2.0 - 1.0, 1.0 - y / height as f32 * 2.0);
    let unproject = |z: f32| {
        let p = inv * Vec4::new(nx, ny, z, 1.0);
        (p.w != 0.0).then(|| p.truncate() / p.w)
    };
    let (near, far) = (unproject(0.0)?, unproject(1.0)?);
    let dir = (far - near).normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    raster::ray_pick(&prep, near, dir)
}

/// Pixel position of a node's origin in a `width`x`height` render, or `None` when it is behind
/// the camera, beyond the far plane or the node does not exist.
pub fn project(scene: &Scene, width: u32, height: u32, id: NodeId) -> Option<(f32, f32)> {
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return None;
    }
    let prep = prepare(scene, width as f32 / height as f32);
    let p = scene.world_matrix(id)?.transform_point3(Vec3::ZERO);
    let clip = prep.view_proj * p.extend(1.0);
    if !p.is_finite() || clip.w <= 0.0 || clip.z < 0.0 || clip.z > clip.w {
        return None;
    }
    let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
    let (x, y) = ((nx * 0.5 + 0.5) * width as f32, (1.0 - (ny * 0.5 + 0.5)) * height as f32);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

#[cfg(test)]
mod tests;
