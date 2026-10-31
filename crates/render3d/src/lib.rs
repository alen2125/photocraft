//! Software renderer for `photocraft-scene3d` scenes: z-buffered triangle rasteriser with
//! perspective-correct normals and texture coordinates, Blinn–Phong shading for directional,
//! point and spot lights, bone skinning for imported models, alpha-tested textures,
//! supersampled anti-aliasing and an optional transparent background (for rendering into a
//! layer). Deterministic, allocation-bounded and free of GPU and OS calls, so it runs in the
//! headless CLI, the tests and the browser build alike. A wgpu path can later replace the inner
//! loop without changing this API.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod raster;

use std::sync::Arc;

use photocraft_scene3d::glam::{Mat3, Mat4, Vec3, Vec4};
use photocraft_scene3d::model3d::{AlphaMode, ModelData};
use photocraft_scene3d::{Light, LightKind, Material, ModelMesh, NodeId, NodeKind, Scene};

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

/// A vertex's joints and their weights.
type Skinning = ([u16; 4], [f32; 4]);

/// A texture of an imported model.
pub(crate) struct TexRef {
    pub model: Arc<ModelData>,
    pub index: usize,
}

/// One mesh in world space, ready to rasterise.
pub(crate) struct Draw {
    pub node: NodeId,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    /// Empty when the mesh has no texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub albedo: Vec3,
    pub alpha: f32,
    pub texture: Option<TexRef>,
    /// Pixels with less alpha than this are dropped (`None` = opaque).
    pub cutoff: Option<f32>,
    pub emissive: Vec3,
    pub shininess: f32,
    pub specular: f32,
    pub unlit: bool,
    /// A tangent-space normal map (needs texture coordinates) and how strongly it bends the normal.
    pub normal_map: Option<TexRef>,
    pub normal_scale: f32,
    pub spec_normal_scale: f32,
    /// A map whose red channel scales the highlight.
    pub spec_map: Option<TexRef>,
    /// Drawn after the opaque things, mixed over them by its alpha (no depth write).
    pub blend: bool,
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
    /// sRGB byte → linear.
    pub lut: [f32; 256],
    /// Byte → 0–1 as it is (for maps that hold data, not colours).
    pub raw: [f32; 256],
}

fn finite_mat(m: &Mat4) -> bool {
    m.to_cols_array().iter().all(|v| v.is_finite())
}

fn primitive_draw(node: NodeId, mesh: &photocraft_scene3d::Mesh, world: Mat4, m: &Material) -> Option<Draw> {
    if !finite_mat(&world) || world.determinant().abs() < 1e-18 {
        return None;
    }
    let normal = Mat3::from_mat4(world).inverse().transpose();
    if !normal.to_cols_array().iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(Draw {
        node,
        positions: mesh.positions.iter().map(|p| world.transform_point3(Vec3::from(*p))).collect(),
        normals: mesh.normals.iter().map(|n| normal * Vec3::from(*n)).collect(),
        uvs: Vec::new(),
        indices: mesh.indices.clone(),
        albedo: lin3(m.color),
        alpha: 1.0,
        texture: None,
        cutoff: None,
        emissive: lin3(m.emissive),
        shininess: if m.shininess.is_finite() { m.shininess.clamp(1.0, 512.0) } else { 32.0 },
        specular: if m.specular.is_finite() { m.specular.clamp(0.0, 1.0) } else { 0.0 },
        unlit: m.unlit,
        normal_map: None,
        normal_scale: 1.0,
        spec_normal_scale: 1.0,
        spec_map: None,
        blend: false,
    })
}

/// A model mesh in world space: skinned by its joints, or placed by its node.
fn model_draw(scene: &Scene, world: &std::collections::HashMap<NodeId, Mat4>, node: NodeId, mm: &ModelMesh) -> Option<Draw> {
    let model = scene.model(mm.asset)?;
    let mesh = model.meshes.get(mm.mesh as usize)?;
    // A mesh with nothing to draw (a hidden material) is not skinned either.
    if mesh.indices.is_empty() {
        return None;
    }
    let n = mesh.positions.len();
    let (positions, normals) = match mesh.skin.and_then(|s| model.skins.get(s)) {
        Some(skin) if mm.joints.len() == skin.joints.len() && mesh.joints.len() == n && mesh.weights.len() == n => {
            let mats: Vec<Mat4> = mm
                .joints
                .iter()
                .zip(&skin.inverse_bind)
                .map(|(j, ibm)| world.get(j).map(|w| *w * Mat4::from_cols_array_2d(ibm)).unwrap_or(Mat4::IDENTITY))
                .collect();
            let mut positions = Vec::with_capacity(n);
            let mut normals = Vec::with_capacity(n);
            // Neighbouring vertices often have the same joints and weights (hair strands, rigid parts):
            // the blended matrix is made once for such a run.
            let mut last: Option<(Skinning, Mat4)> = None;
            for i in 0..n {
                let (js, ws) = (mesh.joints.get(i)?, mesh.weights.get(i)?);
                let blend = match last {
                    Some((key, m)) if key == (*js, *ws) => m,
                    _ => {
                        let mut blend = Mat4::ZERO;
                        let mut total = 0.0;
                        for k in 0..4 {
                            let w = ws[k];
                            if w > 0.0 && w.is_finite() && let Some(m) = mats.get(js[k] as usize) {
                                blend += *m * w;
                                total += w;
                            }
                        }
                        if total <= 1e-6 {
                            blend = Mat4::IDENTITY;
                        } else {
                            blend *= 1.0 / total;
                        }
                        last = Some(((*js, *ws), blend));
                        blend
                    }
                };
                positions.push(blend.transform_point3(Vec3::from(*mesh.positions.get(i)?)));
                normals.push(blend.transform_vector3(Vec3::from(*mesh.normals.get(i)?)));
            }
            (positions, normals)
        }
        Some(_) => return None,
        None => {
            let w = *world.get(&node)?;
            let normal = Mat3::from_mat4(w).inverse().transpose();
            (
                mesh.positions.iter().map(|p| w.transform_point3(Vec3::from(*p))).collect(),
                mesh.normals.iter().map(|v| normal * Vec3::from(*v)).collect(),
            )
        }
    };
    if positions.iter().any(|p| !p.is_finite()) {
        return None;
    }
    let mat = mesh.material.and_then(|m| model.materials.get(m));
    let (base, tex, cutoff, emissive) = match mat {
        Some(m) => (
            m.base_color,
            m.base_color_texture.filter(|&t| model.textures.get(t).is_some_and(Option::is_some)).map(|index| TexRef { model: model.clone(), index }),
            match m.alpha_mode {
                AlphaMode::Opaque => None,
                AlphaMode::Mask => Some(m.alpha_cutoff),
                AlphaMode::Blend => None,
            },
            // Exporters write emissive = 1 on the unlit materials they emit; unlit means no glow.
            if m.unlit { Vec3::ZERO } else { lin3(m.emissive) },
        ),
        None => ([0.8, 0.8, 0.8, 1.0], None, None, Vec3::ZERO),
    };
    let has_uv = mesh.uvs.len() == n;
    let tex = if has_uv { tex } else { None };
    let map = |t: Option<usize>| t.filter(|&t| has_uv && model.textures.get(t).is_some_and(Option::is_some)).map(|index| TexRef { model: model.clone(), index });
    let (normal_map, spec_map) = (mat.and_then(|m| map(m.normal_texture)), mat.and_then(|m| map(m.specular_texture)));
    let normal_scale = mat.map_or(1.0, |m| m.normal_scale);
    let spec_normal_scale = mat.map_or(1.0, |m| m.spec_normal_scale);
    let uses_uv = tex.is_some() || normal_map.is_some() || spec_map.is_some();
    let (specular, shininess) = (mat.and_then(|m| m.specular).unwrap_or(0.08), mat.and_then(|m| m.shininess).unwrap_or(24.0));
    Some(Draw {
        node,
        positions,
        normals,
        uvs: if uses_uv { mesh.uvs.clone() } else { Vec::new() },
        indices: mesh.indices.clone(),
        albedo: lin3([base[0], base[1], base[2]]),
        alpha: base[3].clamp(0.0, 1.0),
        texture: tex,
        cutoff,
        emissive,
        // Characters read better lit than as the flat unlit materials VRM exporters write.
        shininess,
        specular,
        unlit: false,
        normal_map,
        normal_scale: if normal_scale.is_finite() { normal_scale } else { 1.0 },
        spec_normal_scale: if spec_normal_scale.is_finite() { spec_normal_scale } else { normal_scale },
        spec_map,
        blend: mat.is_some_and(|m| m.alpha_mode == AlphaMode::Blend),
    })
}

fn light_world(world: &std::collections::HashMap<NodeId, Mat4>, id: NodeId, l: &Light) -> Option<LightW> {
    let world = world.get(&id)?;
    let pos = world.transform_point3(Vec3::ZERO);
    let dir = world.transform_vector3(Vec3::NEG_Z).normalize_or_zero();
    if !pos.is_finite() || dir == Vec3::ZERO && !matches!(l.kind, LightKind::Point) {
        return None;
    }
    let k = if l.intensity.is_finite() { l.intensity.max(0.0) } else { 0.0 };
    let range = if l.range.is_finite() { l.range.max(1e-3) } else { 20.0 };
    Some(LightW { kind: l.kind, radiance: lin3(l.color) * k, pos, dir, range })
}

/// Eye position and view-projection matrix of the scene's camera; sanitised so any camera works.
pub(crate) fn camera(scene: &Scene, aspect: f32) -> (Vec3, Mat4) {
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
    (eye, Mat4::perspective_rh(fov.to_radians(), aspect, near, far) * view)
}

pub(crate) fn prepare(scene: &Scene, aspect: f32) -> Prepared {
    let world = scene.world_matrices();
    let mut draws = Vec::new();
    let mut lights = Vec::new();
    // Skinning a character's meshes is the costly part; collect them and do it side by side.
    let mut model_jobs: Vec<(NodeId, &ModelMesh)> = Vec::new();
    for n in &scene.nodes {
        if !scene.is_visible(n.id) {
            continue;
        }
        match &n.kind {
            NodeKind::Primitive { primitive, material } => {
                if let Some(w) = world.get(&n.id)
                    && let Some(d) = primitive_draw(n.id, &primitive.mesh(), *w, material)
                {
                    draws.push(d);
                }
            }
            NodeKind::Mesh(mm) => model_jobs.push((n.id, mm)),
            NodeKind::Light(l) if lights.len() < MAX_LIGHTS => {
                if let Some(lw) = light_world(&world, n.id, l) {
                    lights.push(lw);
                }
            }
            _ => {}
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let skinned: Vec<Option<Draw>> = {
        use rayon::prelude::*;
        model_jobs.par_iter().map(|(id, mm)| model_draw(scene, &world, *id, mm)).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let skinned: Vec<Option<Draw>> = model_jobs.iter().map(|(id, mm)| model_draw(scene, &world, *id, mm)).collect();
    draws.extend(skinned.into_iter().flatten());
    let (eye, view_proj) = camera(scene, aspect);
    let mut lut = [0.0f32; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = to_linear(i as f32 / 255.0);
    }
    let mut raw = [0.0f32; 256];
    for (i, v) in raw.iter_mut().enumerate() {
        *v = i as f32 / 255.0;
    }
    Prepared { raw, draws, lights, ambient: lin3(scene.ambient), background: lin3(scene.background), eye, view_proj, lut }
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
    if !finite_mat(&inv) {
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

/// Pixel position of a world point in a `width`×`height` render, or `None` when it is behind the
/// camera or beyond the far plane.
pub fn project_point(scene: &Scene, width: u32, height: u32, p: Vec3) -> Option<(f32, f32)> {
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || !p.is_finite() {
        return None;
    }
    project_with(&camera(scene, width as f32 / height as f32).1, width, height, p)
}

fn project_with(view_proj: &Mat4, width: u32, height: u32, p: Vec3) -> Option<(f32, f32)> {
    let clip = *view_proj * p.extend(1.0);
    if clip.w <= 0.0 || clip.z < 0.0 || clip.z > clip.w {
        return None;
    }
    let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
    let (x, y) = ((nx * 0.5 + 0.5) * width as f32, (1.0 - (ny * 0.5 + 0.5)) * height as f32);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// Pixel position of a node's origin, or `None` when it is out of view or does not exist.
pub fn project(scene: &Scene, width: u32, height: u32, id: NodeId) -> Option<(f32, f32)> {
    let p = scene.world_matrix(id)?.transform_point3(Vec3::ZERO);
    project_point(scene, width, height, p)
}

/// Pixel positions of many world points at once (one camera set-up for all of them), for
/// drawing a skeleton. Points out of view are `None`.
pub fn project_points(scene: &Scene, width: u32, height: u32, points: &[Vec3]) -> Vec<Option<(f32, f32)>> {
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return vec![None; points.len()];
    }
    let view_proj = camera(scene, width as f32 / height as f32).1;
    points.iter().map(|p| if p.is_finite() { project_with(&view_proj, width, height, *p) } else { None }).collect()
}

/// A world-space ray through pixel `(x, y)`: origin and unit direction. Used to turn a pointer
/// drag into a position in space.
pub fn pixel_ray(scene: &Scene, width: u32, height: u32, x: f32, y: f32) -> Option<(Vec3, Vec3)> {
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let inv = camera(scene, width as f32 / height as f32).1.inverse();
    if !finite_mat(&inv) {
        return None;
    }
    let (nx, ny) = (x / width as f32 * 2.0 - 1.0, 1.0 - y / height as f32 * 2.0);
    let unproject = |z: f32| {
        let p = inv * Vec4::new(nx, ny, z, 1.0);
        (p.w != 0.0).then(|| p.truncate() / p.w)
    };
    let (near, far) = (unproject(0.0)?, unproject(1.0)?);
    let dir = (far - near).normalize_or_zero();
    (dir != Vec3::ZERO).then_some((near, dir))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_models;

#[cfg(test)]
mod tests_links;
