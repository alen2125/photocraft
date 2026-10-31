//! Clothing worn by a figure. VaM's clothes are not skinned themselves: each cloth vertex is bound
//! to the nearest spot of the body surface (a triangle, barycentric weights, a distance along its
//! normal) and follows it. Here the same binding is made once, when the item is put on; after that
//! a cloth vertex's rest place is that spot on the *morphed* body, and it takes the skin weights of
//! the three body vertices around it, so the ordinary skinning poses it with the body.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use photocraft_model3d::{MaterialData, MeshData, ModelData};
use serde::{Deserialize, Serialize};

use crate::scene::{Node, NodeKind, Scene, SceneError};

/// Most vertices all the strand hair of a figure may have in all (each is skinned every frame).
pub const HAIR_VERTEX_BUDGET: usize = 700_000;

/// Most vertices one item may have, and most items one figure may wear.
pub const MAX_CLOTH_VERTICES: usize = 1_000_000;
pub const MAX_CLOTH_ITEMS: usize = 64;

/// An item a figure wears, as saved in the scene: where its files are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClothRef {
    pub asset: u32,
    /// The item's `.vam` reference (`Package.N:/Custom/Clothing/...`).
    pub reference: String,
    pub name: String,
}

/// How the game tied one item vertex to the body (see `DAZSkinWrapStore`).
#[derive(Clone, Copy, Debug, Default)]
pub struct WrapVert {
    pub vertices: [u32; 3],
    pub normal: f32,
    pub tangent1: f32,
    pub tangent2: f32,
    pub normal_dot: f32,
    pub tangent1_dot: f32,
    pub tangent2_dot: f32,
}

/// Strand hair worn on a figure: the guide strands are the item's points (`ClothInput::positions`, each
/// strand a run of them), and the hair is drawn as thin lines grown round each guide.
#[derive(Clone, Debug, Default)]
pub struct HairSpec {
    /// Start and length of each strand in `ClothInput::positions`.
    pub strands: Vec<(u32, u32)>,
    /// Hairs per guide strand at full quality (the game's `hairMultiplier`).
    pub copies: u32,
    /// How far the hairs may lie from their guide at the root, the middle and the tip (metres).
    pub spread: [f32; 3],
}

/// An item's mesh as read from its file, in the game's axes.
#[derive(Clone, Debug, Default)]
pub struct ClothInput {
    /// Set for strand hair: the item has no triangles of its own.
    pub hair: Option<HairSpec>,
    /// The game's own tie of each vertex to the body, when the file has it (else the nearest body
    /// surface is used); and the item's surface offset and thickness setting.
    pub wrap: Vec<WrapVert>,
    pub surface_offset: f32,
    pub thickness: f32,
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// The base vertex each vertex stands for (copies along a texture seam share one).
    pub uv_base: Vec<u32>,
    /// Material name and triangles (vertex indices) of each piece.
    pub pieces: Vec<(String, Vec<u32>)>,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    /// A point on the triangle (barycentric weights) and a distance along its normal.
    Nearest { w: [f32; 3], offset: f32 },
    /// The game's tie: along the triangle's tangent towards its centre, the second tangent and the
    /// normal (`sn`, `s2`: which way each points on this vertex).
    Wrapped { t1: f32, t2: f32, n: f32, sn: f32, s2: f32 },
}

#[derive(Clone, Copy, Debug)]
struct Bind {
    tri: [u32; 3],
    /// How much of its skin each of the three body vertices lends.
    skin_w: [f32; 3],
    kind: Kind,
}

const LOOSE: Bind = Bind { tri: [0; 3], skin_w: [1.0, 0.0, 0.0], kind: Kind::Nearest { w: [1.0, 0.0, 0.0], offset: 0.0 } };

/// Where a wrapped vertex sits on the (morphed or posed) body.
fn wrapped_point(tri: [u32; 3], t1: f32, t2: f32, n: f32, sn: f32, s2: f32, at: impl Fn(u32) -> Vec3) -> Vec3 {
    let (v1, v2, v3) = (at(tri[0]), at(tri[1]), at(tri[2]));
    let t1_dir = (v1 + v2 + v3) * 0.33333 - v1;
    let n_dir = tri_normal(v1, v2, v3) * sn;
    // The figure's z axis is mirrored, which turns a cross product round (`s2` settles it).
    let t2_dir = t1_dir.cross(n_dir) * s2;
    v1 + t1_dir * t1 + t2_dir * t2 + n_dir * n
}

pub(crate) struct Cloth {
    pub(crate) item: ClothRef,
    uvs: Vec<[f32; 2]>,
    base_of: Vec<u32>,
    pieces: Vec<(String, Vec<u32>)>,
    binds: Vec<Bind>,
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
    hair: Option<HairSpec>,
    /// For hair: the body triangles under the roots, drawn as a scalp in the hair's colour.
    cap: Vec<[u32; 3]>,
}

fn mirror(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], -p[2])
}

/// The point of triangle `abc` nearest to `p`, as barycentric weights (Ericson, Real-Time Collision Detection).
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> [f32; 3] {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return [0.0, 0.0, 1.0];
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [1.0 - w, 0.0, w];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return [0.0, 1.0 - w, w];
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    [1.0 - v - w, v, w]
}

fn tri_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    (b - a).cross(c - a).normalize_or_zero()
}

/// Binds every point to its nearest body triangle.
fn bind_points(points: &[Vec3], body: &[Vec3], tris: &[[u32; 3]]) -> Vec<Bind> {
    const CELL: f32 = 0.04;
    let cell = |p: Vec3| ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32, (p.z / CELL).floor() as i32);
    let at = |i: u32| body.get(i as usize).copied().unwrap_or(Vec3::ZERO);
    let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
    for (t, tri) in tris.iter().enumerate() {
        let c = (at(tri[0]) + at(tri[1]) + at(tri[2])) / 3.0;
        grid.entry(cell(c)).or_default().push(t as u32);
    }
    points
        .iter()
        .map(|&p| {
            let (cx, cy, cz) = cell(p);
            let mut best: Option<(f32, usize, [f32; 3])> = None;
            for r in 1..=12i32 {
                // Only the new shell of cells each round.
                for dx in -r..=r {
                    for dy in -r..=r {
                        for dz in -r..=r {
                            if dx.abs().max(dy.abs()).max(dz.abs()) != r {
                                continue;
                            }
                            for &t in grid.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                                let Some(tri) = tris.get(t as usize) else { continue };
                                let (a, b, c) = (at(tri[0]), at(tri[1]), at(tri[2]));
                                let w = closest_on_triangle(p, a, b, c);
                                let q = a * w[0] + b * w[1] + c * w[2];
                                let d = p.distance_squared(q);
                                if best.is_none_or(|(bd, _, _)| d < bd) {
                                    best = Some((d, t as usize, w));
                                }
                            }
                        }
                    }
                }
                // Triangles are small: once the best is well inside the searched shells, stop.
                if let Some((d, _, _)) = best
                    && d.sqrt() + 0.05 < r as f32 * CELL
                {
                    break;
                }
            }
            match best.and_then(|(_, t, w)| tris.get(t).map(|tri| (*tri, w))) {
                Some((tri, w)) => {
                    let (a, b, c) = (at(tri[0]), at(tri[1]), at(tri[2]));
                    let q = a * w[0] + b * w[1] + c * w[2];
                    Bind { tri, skin_w: w, kind: Kind::Nearest { w, offset: (p - q).dot(tri_normal(a, b, c)) } }
                }
                None => LOOSE,
            }
        })
        .collect()
}

impl Cloth {
    /// The cloth's vertices on a body whose vertex positions are `body`.
    fn wrapped(&self, body: &[[f32; 3]]) -> Vec<[f32; 3]> {
        let at = |i: u32| body.get(i as usize).map_or(Vec3::ZERO, |p| Vec3::from(*p));
        self.binds
            .iter()
            .map(|b| {
                let (pa, pb, pc) = (at(b.tri[0]), at(b.tri[1]), at(b.tri[2]));
                match b.kind {
                    Kind::Nearest { w, offset } => (pa * w[0] + pb * w[1] + pc * w[2] + tri_normal(pa, pb, pc) * offset).to_array(),
                    Kind::Wrapped { t1, t2, n, sn, s2 } => wrapped_point(b.tri, t1, t2, n, sn, s2, at).to_array(),
                }
            })
            .collect()
    }

    /// Smooth normals; copies of one base vertex share theirs.
    fn normals(&self, positions: &[[f32; 3]]) -> Vec<[f32; 3]> {
        let mut acc: HashMap<u32, Vec3> = HashMap::new();
        let pos = |i: u32| positions.get(i as usize).map_or(Vec3::ZERO, |p| Vec3::from(*p));
        for (_, tris) in &self.pieces {
            for t in tris.as_chunks::<3>().0 {
                let face = (pos(t[1]) - pos(t[0])).cross(pos(t[2]) - pos(t[0]));
                for &v in t {
                    *acc.entry(self.base_of.get(v as usize).copied().unwrap_or(v)).or_default() += face;
                }
            }
        }
        (0..positions.len() as u32)
            .map(|v| {
                let n = acc.get(&self.base_of.get(v as usize).copied().unwrap_or(v)).copied().unwrap_or(Vec3::Y).normalize_or_zero();
                if n == Vec3::ZERO { [0.0, 1.0, 0.0] } else { n.to_array() }
            })
            .collect()
    }
}

/// The body triangles that lie under the roots of the strands (within three centimetres of
/// one): VaM draws the scalp there in the hair's colour, so the skin does not show between strands.
fn cap_triangles(points: &[Vec3], strands: &[(u32, u32)], body: &[Vec3], tris: &[[u32; 3]]) -> Vec<[u32; 3]> {
    const CELL: f32 = 0.03;
    let cell = |p: Vec3| ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32, (p.z / CELL).floor() as i32);
    let mut grid: HashMap<(i32, i32, i32), Vec<Vec3>> = HashMap::new();
    for &(start, len) in strands {
        if len > 0
            && let Some(&root) = points.get(start as usize)
        {
            grid.entry(cell(root)).or_default().push(root);
        }
    }
    let at = |i: u32| body.get(i as usize).copied().unwrap_or(Vec3::ZERO);
    tris.iter()
        .filter(|t| {
            let c = (at(t[0]) + at(t[1]) + at(t[2])) / 3.0;
            let (cx, cy, cz) = cell(c);
            (-1..=1).any(|dx| (-1..=1).any(|dy| (-1..=1).any(|dz| grid.get(&(cx + dx, cy + dy, cz + dz)).is_some_and(|roots| roots.iter().any(|r| r.distance(c) < CELL)))))
        })
        .copied()
        .collect()
}

/// A number in [0, 1) from three small integers (a fixed pseudo-random choice, the same every time).
fn unit_hash(a: u32, b: u32, c: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA6B) ^ c.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// What the hair mesh of one item is made of.
struct HairMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Cloth {
    /// Hairs round the guide strands, built from the guides' places on the (morphed) body. Each hair
    /// lies at a fixed random distance and angle from its guide (the first one runs along it), the
    /// distance following the spread from the root to the tip. A hair is a line: its vertices'
    /// normals hold the direction of the strand, u is a random pick of the hair's tone, v runs from
    /// the root to the tip.
    fn hair_mesh(&self, spec: &HairSpec, points: &[[f32; 3]], copies: u32) -> HairMesh {
        let mut m = HairMesh { positions: Vec::new(), normals: Vec::new(), uvs: Vec::new(), joints: Vec::new(), weights: Vec::new(), indices: Vec::new() };
        let at = |i: usize| points.get(i).map_or(Vec3::ZERO, |p| Vec3::from(*p));
        for (si, &(start, len)) in spec.strands.iter().enumerate() {
            let (start, len) = (start as usize, len as usize);
            if len < 2 || start + len > points.len() {
                continue;
            }
            // The frame along the strand, once.
            let frames: Vec<(Vec3, Vec3, Vec3)> = (0..len)
                .map(|i| {
                    let t = (at(start + (i + 1).min(len - 1)) - at(start + i.saturating_sub(1))).normalize_or_zero();
                    let up = if t.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
                    let n1 = t.cross(up).normalize_or_zero();
                    (t, n1, t.cross(n1))
                })
                .collect();
            // A strand moves as one with the place it grows from (the head), so every point of it has
            // the root's skin weights: drawing then blends one matrix per strand, not per point.
            let (j, w) = (self.joints.get(start).copied().unwrap_or([0; 4]), self.weights.get(start).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]));
            for k in 0..copies.max(1) {
                let (r1, r2, r3) = (unit_hash(si as u32, k, 1), unit_hash(si as u32, k, 2), unit_hash(si as u32, k, 3));
                let phi = r1 * std::f32::consts::TAU;
                // The first hair runs along the guide itself.
                let dist = if k == 0 { 0.0 } else { r2.sqrt() };
                let base = m.positions.len() as u32;
                for (i, &(t, n1, n2)) in frames.iter().enumerate() {
                    let along = i as f32 / (len - 1) as f32;
                    let spread = if along < 0.5 { spec.spread[0] + (spec.spread[1] - spec.spread[0]) * (along / 0.5) } else { spec.spread[1] + (spec.spread[2] - spec.spread[1]) * ((along - 0.5) / 0.5) };
                    m.positions.push((at(start + i) + (n1 * phi.cos() + n2 * phi.sin()) * (dist * spread)).to_array());
                    m.normals.push(if t == Vec3::ZERO { [0.0, -1.0, 0.0] } else { t.to_array() });
                    m.uvs.push([r3, along]);
                    m.joints.push(j);
                    m.weights.push(w);
                    if i + 1 < len {
                        m.indices.extend([base + i as u32, base + i as u32 + 1]);
                    }
                }
            }
        }
        m
    }
}

/// The material name a cloth piece's material has in the figure's model.
pub fn cloth_material_name(item: &str, material: &str) -> String {
    format!("{item}:{material}")
}

impl Scene {
    /// Puts an item on a figure: binds it to the figure's body and adds its meshes.
    pub fn add_cloth(&mut self, asset: u32, reference: &str, input: ClothInput) -> Result<(), SceneError> {
        if self.clothing.iter().filter(|c| c.asset == asset).count() >= MAX_CLOTH_ITEMS {
            return Err(SceneError::Parse("too many items on one figure".into()));
        }
        if input.positions.len() > MAX_CLOTH_VERTICES || input.uvs.len() != input.positions.len() && !input.uvs.is_empty() {
            return Err(SceneError::Parse("the item's mesh is too big or inconsistent".into()));
        }
        let base = match self.morph_cache.base.get(&asset) {
            Some(b) => b.clone(),
            None => self.models.0.get(&asset).cloned().ok_or_else(|| SceneError::Parse(format!("model of asset {asset} is not loaded")))?,
        };
        let body = base.meshes.first().ok_or_else(|| SceneError::Parse("the figure has no mesh".into()))?;
        if body.skin.is_none() || body.joints.len() != body.positions.len() || body.weights.len() != body.positions.len() {
            return Err(SceneError::Parse("the figure is not skinned".into()));
        }
        let rest: Vec<Vec3> = body.positions.iter().map(|p| Vec3::from(*p)).collect();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        for m in &base.meshes {
            tris.extend(m.indices.as_chunks::<3>().0.iter().copied());
        }
        let cloth_pos: Vec<Vec3> = input.positions.iter().map(|p| mirror(*p)).collect();
        if cloth_pos.iter().any(|p| !p.is_finite()) {
            return Err(SceneError::Parse("the item's mesh has bad numbers".into()));
        }
        let base_of: Vec<u32> = (0..cloth_pos.len()).map(|v| input.uv_base.get(v).copied().filter(|b| *b != u32::MAX).unwrap_or(v as u32)).collect();
        let nearest_binds = |only: &[u32]| {
            let points: Vec<Vec3> = only.iter().map(|&v| cloth_pos[v as usize]).collect();
            bind_points(&points, &rest, &tris)
        };
        let binds: Vec<Bind> = if input.wrap.len() == cloth_pos.len() {
            // The game's own tie: the sign of the normal and of the second tangent is settled by which
            // pair puts the vertex back where the item has it at rest.
            let mut missing: Vec<u32> = Vec::new();
            let mut out: Vec<Bind> = Vec::with_capacity(cloth_pos.len());
            for (v, w) in input.wrap.iter().enumerate() {
                let tri = w.vertices;
                if tri.iter().any(|&t| t as usize >= rest.len()) {
                    missing.push(v as u32);
                    out.push(LOOSE);
                    continue;
                }
                let at = |i: u32| rest.get(i as usize).copied().unwrap_or(Vec3::ZERO);
                let t1 = w.tangent1 + w.tangent1_dot * input.thickness;
                let t2 = w.tangent2 + w.tangent2_dot * input.thickness;
                let n = w.normal + input.surface_offset + w.normal_dot * input.thickness;
                let mut best = (f32::MAX, 1.0f32, 1.0f32);
                for sn in [1.0f32, -1.0] {
                    for s2 in [1.0f32, -1.0] {
                        let d = wrapped_point(tri, t1, t2, n, sn, s2, at).distance_squared(cloth_pos[v]);
                        if d < best.0 {
                            best = (d, sn, s2);
                        }
                    }
                }
                let skin_w = closest_on_triangle(cloth_pos[v], at(tri[0]), at(tri[1]), at(tri[2]));
                out.push(Bind { tri, skin_w, kind: Kind::Wrapped { t1, t2, n, sn: best.1, s2: best.2 } });
            }
            if !missing.is_empty() {
                for (v, b) in missing.iter().zip(nearest_binds(&missing)) {
                    out[*v as usize] = b;
                }
            }
            out
        } else {
            // One bind per base vertex, shared by its copies.
            let mut unique: Vec<u32> = Vec::new();
            let mut slot: HashMap<u32, usize> = HashMap::new();
            for (v, b) in base_of.iter().enumerate() {
                slot.entry(*b).or_insert_with(|| {
                    unique.push(v as u32);
                    unique.len() - 1
                });
            }
            let unique_binds = nearest_binds(&unique);
            base_of.iter().map(|b| slot.get(b).and_then(|&s| unique_binds.get(s)).copied().unwrap_or(LOOSE)).collect()
        };
        // Skin weights: the three body vertices' influences, by barycentric weight, best four.
        let mut joints = Vec::with_capacity(binds.len());
        let mut weights = Vec::with_capacity(binds.len());
        for b in &binds {
            let mut acc: Vec<(u16, f32)> = Vec::new();
            for (k, &v) in b.tri.iter().enumerate() {
                let (Some(js), Some(ws)) = (body.joints.get(v as usize), body.weights.get(v as usize)) else { continue };
                for (j, w) in js.iter().zip(ws) {
                    if *w <= 0.0 {
                        continue;
                    }
                    match acc.iter_mut().find(|(aj, _)| aj == j) {
                        Some(e) => e.1 += w * b.skin_w[k],
                        None => acc.push((*j, w * b.skin_w[k])),
                    }
                }
            }
            acc.sort_by(|a, b| b.1.total_cmp(&a.1));
            acc.truncate(4);
            let total: f32 = acc.iter().map(|e| e.1).sum();
            let (mut j4, mut w4) = ([0u16; 4], [0.0f32; 4]);
            for (i, (j, w)) in acc.iter().enumerate() {
                j4[i] = *j;
                w4[i] = if total > 0.0 { w / total } else { 0.0 };
            }
            if total <= 0.0 {
                w4[0] = 1.0;
            }
            joints.push(j4);
            weights.push(w4);
        }
        // The mirrored axis flips the winding.
        let pieces: Vec<(String, Vec<u32>)> = input
            .pieces
            .iter()
            .map(|(m, t)| (m.clone(), t.as_chunks::<3>().0.iter().filter(|t| t.iter().all(|&v| (v as usize) < cloth_pos.len())).flat_map(|t| [t[0], t[2], t[1]]).collect()))
            .collect();
        let item = ClothRef { asset, reference: reference.to_string(), name: input.name.clone() };
        let uvs = if input.uvs.len() == cloth_pos.len() { input.uvs } else { vec![[0.0; 2]; cloth_pos.len()] };
        let cap = input.hair.as_ref().map(|h| cap_triangles(&cloth_pos, &h.strands, &rest, &tris)).unwrap_or_default();
        let cloth = Cloth { item: item.clone(), uvs, base_of, pieces, binds, joints, weights, hair: input.hair, cap };
        self.clothing.retain(|c| !(c.asset == asset && c.reference == reference));
        let list = self.morph_cache.cloths.entry(asset).or_default();
        list.retain(|c| c.item.reference != reference);
        list.push(Arc::new(cloth));
        self.clothing.push(item);
        self.rebuild_cloth_nodes(asset)?;
        self.apply_morphs(asset)
    }

    /// Takes an item off.
    pub fn remove_cloth(&mut self, asset: u32, reference: &str) -> Result<(), SceneError> {
        self.clothing.retain(|c| !(c.asset == asset && c.reference == reference));
        if let Some(list) = self.morph_cache.cloths.get_mut(&asset) {
            list.retain(|c| c.item.reference != reference);
        }
        self.rebuild_cloth_nodes(asset)?;
        self.apply_morphs(asset)
    }

    /// Sets how much of the hair is drawn (0 to 1) and rebuilds the figures that wear hair.
    pub fn set_hair_quality(&mut self, quality: f32) -> Result<(), SceneError> {
        let q = if quality.is_finite() { quality.clamp(0.0, 1.0) } else { 1.0 };
        if (q - self.hair_quality).abs() < 1e-4 {
            return Ok(());
        }
        self.hair_quality = q;
        let assets: Vec<u32> = self.morph_cache.cloths.iter().filter(|(_, l)| l.iter().any(|c| c.hair.is_some())).map(|(a, _)| *a).collect();
        for a in assets {
            self.apply_morphs(a)?;
        }
        Ok(())
    }

    /// Whether the item's mesh is loaded onto the figure.
    pub fn cloth_loaded(&self, asset: u32, reference: &str) -> bool {
        self.morph_cache.cloths.get(&asset).is_some_and(|l| l.iter().any(|c| c.item.reference == reference))
    }

    /// The items a figure wears (also those not loaded yet, after opening a scene file).
    pub fn worn(&self, asset: u32) -> Vec<&ClothRef> {
        self.clothing.iter().filter(|c| c.asset == asset).collect()
    }

    /// Mesh nodes for each piece of each worn item, after those of the figure's own meshes.
    fn rebuild_cloth_nodes(&mut self, asset: u32) -> Result<(), SceneError> {
        let base = self.morph_cache.base.get(&asset).cloned().or_else(|| self.models.0.get(&asset).cloned());
        let own = base.as_ref().map_or(0, |b| b.meshes.len()) as u32;
        let template = self.nodes.iter().find_map(|n| match &n.kind {
            NodeKind::Mesh(m) if m.asset == asset && m.mesh < own && !m.joints.is_empty() => Some((n.parent, m.joints.clone())),
            _ => None,
        });
        self.nodes.retain(|n| !matches!(&n.kind, NodeKind::Mesh(m) if m.asset == asset && m.mesh >= own));
        let Some((parent, joints)) = template else { return Ok(()) };
        let mut index = own;
        let cloths: Vec<Arc<Cloth>> = self.morph_cache.cloths.get(&asset).cloned().unwrap_or_default();
        for c in &cloths {
            for (mat, _) in &c.pieces {
                self.add(Node { name: format!("{} ({mat})", c.item.name), parent, kind: NodeKind::Mesh(crate::scene::ModelMesh { asset, mesh: index, joints: joints.clone() }), ..Node::default() })?;
                index += 1;
            }
        }
        Ok(())
    }

    /// Appends the worn items' meshes to a freshly built model, on the morphed body positions.
    pub(crate) fn append_cloth(&self, asset: u32, base: &ModelData, body_positions: &[[f32; 3]], model: &mut ModelData) {
        let Some(list) = self.morph_cache.cloths.get(&asset) else { return };
        let Some(body) = base.meshes.first() else { return };
        // Quality 1 grows every hair the game does (`hairMultiplier` per guide), quality 0 only the guides.
        let q = self.hair_quality.clamp(0.0, 1.0);
        let hair_points: usize = list.iter().filter_map(|c| c.hair.as_ref()).map(|h| h.strands.iter().map(|s| s.1 as usize).sum::<usize>()).sum();
        for c in list {
            let positions = c.wrapped(body_positions);
            if let Some(spec) = &c.hair {
                let wanted = 1.0 + (spec.copies.max(1) - 1) as f32 * q;
                // All the hair a figure wears shares one budget of vertices.
                let cap = (HAIR_VERTEX_BUDGET / hair_points.max(1)).max(1) as f32;
                let h = c.hair_mesh(spec, &positions, wanted.round().min(cap) as u32);
                model.materials.push(MaterialData {
                    name: cloth_material_name(&c.item.name, "Hair"),
                    base_color: [1.0; 4],
                    base_color_texture: None,
                    emissive: [0.0; 3],
                    alpha_mode: photocraft_model3d::AlphaMode::Opaque,
                    alpha_cutoff: 0.5,
                    double_sided: true,
                    unlit: false,
                    normal_texture: None,
                    normal_scale: 1.0,
                    spec_normal_scale: 1.0,
                    specular_texture: None,
                    specular: Some(0.1),
                    shininess: Some(60.0),
                });
                model.meshes.push(MeshData {
                    name: format!("{} (Hair)", c.item.name),
                    node: body.node,
                    skin: body.skin,
                    positions: h.positions,
                    normals: h.normals,
                    uvs: h.uvs,
                    joints: h.joints,
                    weights: h.weights,
                    indices: h.indices,
                    material: Some(model.materials.len() - 1),
                    lines: true,
                });
                // The scalp under the roots: the body's own triangles there, lifted off the skin a little.
                let body_normals: Vec<[f32; 3]> = model.meshes.first().map(|m| m.normals.clone()).unwrap_or_default();
                let mut slot: HashMap<u32, u32> = HashMap::new();
                let mut used: Vec<usize> = Vec::new();
                let indices: Vec<u32> = c
                    .cap
                    .iter()
                    .flatten()
                    .map(|&v| {
                        *slot.entry(v).or_insert_with(|| {
                            used.push(v as usize);
                            (used.len() - 1) as u32
                        })
                    })
                    .collect();
                let lifted = |v: usize| {
                    let (p, n) = (Vec3::from(body_positions.get(v).copied().unwrap_or([0.0; 3])), Vec3::from(body_normals.get(v).copied().unwrap_or([0.0; 3])));
                    (p + n.normalize_or_zero() * 0.0006).to_array()
                };
                let mut scalp = model.materials.last().cloned().unwrap_or_else(|| model.materials[0].clone());
                scalp.name = cloth_material_name(&c.item.name, "Scalp");
                model.materials.push(scalp);
                model.meshes.push(MeshData {
                    name: format!("{} (Scalp)", c.item.name),
                    node: body.node,
                    skin: body.skin,
                    positions: used.iter().map(|&v| lifted(v)).collect(),
                    normals: used.iter().map(|&v| body_normals.get(v).copied().unwrap_or([0.0, 1.0, 0.0])).collect(),
                    // The root colour: the first row of the hair picture, its opaque middle.
                    uvs: vec![[0.5, 0.0]; used.len()],
                    joints: used.iter().map(|&v| body.joints.get(v).copied().unwrap_or([0; 4])).collect(),
                    weights: used.iter().map(|&v| body.weights.get(v).copied().unwrap_or([1.0, 0.0, 0.0, 0.0])).collect(),
                    indices,
                    material: Some(model.materials.len() - 1),
                    lines: false,
                });
                continue;
            }
            let normals = c.normals(&positions);
            for (mat, tris) in &c.pieces {
                model.materials.push(MaterialData {
                    name: cloth_material_name(&c.item.name, mat),
                    base_color: [0.8, 0.8, 0.8, 1.0],
                    base_color_texture: None,
                    emissive: [0.0; 3],
                    alpha_mode: photocraft_model3d::AlphaMode::Opaque,
                    alpha_cutoff: 0.5,
                    double_sided: true,
                    unlit: false,
                    normal_texture: None,
                    normal_scale: 1.0,
                    spec_normal_scale: 1.0,
                    specular_texture: None,
                    specular: None,
                    shininess: None,
                });
                // Each piece keeps only the vertices it uses, so skinning it costs what it draws.
                let mut slot: HashMap<u32, u32> = HashMap::new();
                let mut used: Vec<usize> = Vec::new();
                let indices: Vec<u32> = tris
                    .iter()
                    .map(|&v| {
                        *slot.entry(v).or_insert_with(|| {
                            used.push(v as usize);
                            (used.len() - 1) as u32
                        })
                    })
                    .collect();
                let pick = |list: &[[f32; 3]]| used.iter().map(|&v| list.get(v).copied().unwrap_or([0.0; 3])).collect::<Vec<_>>();
                model.meshes.push(MeshData {
                    name: format!("{} ({mat})", c.item.name),
                    node: body.node,
                    skin: body.skin,
                    positions: pick(&positions),
                    normals: pick(&normals),
                    uvs: used.iter().map(|&v| c.uvs.get(v).copied().unwrap_or([0.0; 2])).collect(),
                    joints: used.iter().map(|&v| c.joints.get(v).copied().unwrap_or([0; 4])).collect(),
                    weights: used.iter().map(|&v| c.weights.get(v).copied().unwrap_or([1.0, 0.0, 0.0, 0.0])).collect(),
                    indices,
                    material: Some(model.materials.len() - 1),
                    lines: false,
                });
            }
        }
    }
}
