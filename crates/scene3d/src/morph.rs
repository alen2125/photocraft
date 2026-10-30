//! Morphs on a model: the values the scene keeps, and the model they make. A morph moves vertices
//! (offsets per base vertex of the figure) and bone centres (formulas), and others drive it: its
//! value is the sum of what is set and what other morphs ask for. The scene keeps the model as it
//! was loaded and swaps the morphed one in whenever a value changes, so rendering, picking and
//! physics need to know nothing about morphs.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use photocraft_model3d::morph::{Axis, FormulaKind, Morph, MorphInfo};
use photocraft_model3d::{ModelData, TextureData};
use serde::{Deserialize, Serialize};

use crate::scene::{NodeKind, Scene, SceneError};

/// Most morphs one scene keeps data for.
pub const MAX_MORPHS: usize = 8192;
/// How many times driven values are propagated (a morph may drive one that drives another).
const DRIVE_PASSES: usize = 5;

/// A morph's value on one model, as the scene file keeps it (zero values are not kept).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MorphValue {
    pub asset: u32,
    pub id: String,
    pub value: f32,
}

impl Default for MorphValue {
    fn default() -> Self {
        Self { asset: 0, id: String::new(), value: 0.0 }
    }
}

/// A picture and/or a tint for every material of a model that has this name. The picture is
/// named by the reference the look used (`Package.N:/Custom/.../skin.jpg`); its pixels are
/// supplied separately ([`Scene::add_texture_data`]) and are not part of the scene file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MaterialOverride {
    pub asset: u32,
    pub material: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tint: Option<[f32; 3]>,
    /// Pictures by uid, like `texture`: a normal map and a specular (or gloss) map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specular_map: Option<String>,
    /// How strongly the normal map shows, the highlight strength and its sharpness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bump: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specular: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shininess: Option<f32>,
}

/// Morph data and the unmorphed models, by asset. Not part of the scene file, never compared.
#[derive(Clone, Default)]
pub struct MorphCache {
    /// The model as it was loaded, kept once the first morph is applied.
    base: HashMap<u32, Arc<ModelData>>,
    data: HashMap<(u32, String), Arc<Morph>>,
    /// Decoded pictures by the reference that names them.
    textures: HashMap<String, Arc<photocraft_model3d::TextureData>>,
    /// Per asset, the vertices that copy each base vertex (built when first needed).
    copies: HashMap<u32, Arc<Vec<Vec<u32>>>>,
    /// Per asset, for each base vertex of the genital graft, the body vertices it follows.
    graft: HashMap<u32, Arc<Vec<GraftLink>>>,
    /// Per asset, the world offset of each node's centre the current model was built with.
    offsets: HashMap<u32, Vec<Vec3>>,
}

/// A graft vertex (its base index) and the nearby body vertices (base index, weight) it follows.
type GraftLink = (u32, Vec<(u32, f32)>);

impl PartialEq for MorphCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for MorphCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MorphCache({} morphs)", self.data.len())
    }
}

/// An offset in VaM's axes into ours (the figure's z axis is mirrored).
fn mirror(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], -v[2])
}

impl Scene {
    /// Makes a morph's data available for a model. Replaces one with the same id.
    pub fn add_morph_data(&mut self, asset: u32, morph: Arc<Morph>) -> Result<(), SceneError> {
        if !self.assets.iter().any(|a| a.id == asset) {
            return Err(SceneError::Parse(format!("scene has no asset {asset}")));
        }
        if self.morph_cache.data.len() >= MAX_MORPHS && !self.morph_cache.data.contains_key(&(asset, morph.info.id.clone())) {
            return Err(SceneError::Parse(format!("more than {MAX_MORPHS} morphs")));
        }
        self.morph_cache.data.insert((asset, morph.info.id.clone()), morph);
        Ok(())
    }

    /// The morphs available for a model, in id order.
    pub fn morph_infos(&self, asset: u32) -> Vec<&MorphInfo> {
        let mut v: Vec<&MorphInfo> = self.morph_cache.data.iter().filter(|((a, _), _)| *a == asset).map(|(_, m)| &m.info).collect();
        v.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.display_name.cmp(&b.display_name)));
        v
    }

    /// Supplies the pixels of a picture a material override names.
    pub fn add_texture_data(&mut self, uid: &str, texture: Arc<photocraft_model3d::TextureData>) {
        self.morph_cache.textures.insert(uid.to_string(), texture);
    }

    /// The pixels of a picture, if they are known.
    pub fn texture_data(&self, uid: &str) -> Option<Arc<photocraft_model3d::TextureData>> {
        self.morph_cache.textures.get(uid).cloned()
    }

    /// Whether the pixels of a picture are known.
    pub fn has_texture_data(&self, uid: &str) -> bool {
        self.morph_cache.textures.contains_key(uid)
    }

    /// Puts a picture and/or a tint on the materials of a model called `material`; both `None`
    /// takes the override away. Rebuilds the model.
    pub fn set_material_override(&mut self, asset: u32, material: &str, texture: Option<&str>, tint: Option<[f32; 3]>) -> Result<(), SceneError> {
        if !self.assets.iter().any(|a| a.id == asset) {
            return Err(SceneError::Parse(format!("scene has no asset {asset}")));
        }
        self.material_overrides.retain(|o| !(o.asset == asset && o.material == material));
        if texture.is_some() || tint.is_some() {
            self.material_overrides.push(MaterialOverride { asset, material: material.to_string(), texture: texture.map(str::to_string), tint: tint.filter(|t| t.iter().all(|c| c.is_finite())), ..MaterialOverride::default() });
        }
        self.apply_morphs(asset)
    }

    /// A morph's own value (0 when it is not set).
    pub fn morph_value(&self, asset: u32, id: &str) -> f32 {
        self.morphs.iter().find(|m| m.asset == asset && m.id == id).map_or(0.0, |m| m.value)
    }

    /// Sets a morph's value (clamped to its range) and rebuilds the model. Zero clears it.
    pub fn set_morph(&mut self, asset: u32, id: &str, value: f32) -> Result<(), SceneError> {
        let Some(info) = self.morph_cache.data.get(&(asset, id.to_string())).map(|m| m.info.clone()) else {
            return Err(SceneError::Parse(format!("no morph '{id}' loaded for asset {asset}")));
        };
        let (lo, hi) = if info.min <= info.max { (info.min, info.max) } else { (info.max, info.min) };
        let value = if value.is_finite() { value.clamp(lo.min(-10.0), hi.max(10.0)) } else { 0.0 };
        self.morphs.retain(|m| !(m.asset == asset && m.id == id));
        if value != 0.0 {
            self.morphs.push(MorphValue { asset, id: id.to_string(), value });
        }
        self.apply_morphs(asset)
    }

    /// Sets many morph values at once (a look) and rebuilds the model a single time. Values for
    /// morphs that are not loaded are returned as `Err` entries; the rest are set. A morph not
    /// listed keeps its value unless `clear_others`.
    pub fn set_morph_values(&mut self, asset: u32, values: &[(String, f32)], clear_others: bool) -> Result<Vec<String>, SceneError> {
        if !self.models.0.contains_key(&asset) {
            return Err(SceneError::Parse(format!("model of asset {asset} is not loaded")));
        }
        if clear_others {
            self.morphs.retain(|m| m.asset != asset);
        }
        let mut unknown = Vec::new();
        for (id, value) in values {
            let Some(info) = self.morph_cache.data.get(&(asset, id.clone())).map(|m| m.info.clone()) else {
                unknown.push(id.clone());
                continue;
            };
            let (lo, hi) = if info.min <= info.max { (info.min, info.max) } else { (info.max, info.min) };
            let value = if value.is_finite() { value.clamp(lo.min(-10.0), hi.max(10.0)) } else { 0.0 };
            self.morphs.retain(|m| !(m.asset == asset && m.id == *id));
            if value != 0.0 {
                self.morphs.push(MorphValue { asset, id: id.clone(), value });
            }
        }
        self.apply_morphs(asset)?;
        Ok(unknown)
    }

    /// Clears every morph of a model.
    pub fn reset_morphs(&mut self, asset: u32) -> Result<(), SceneError> {
        self.morphs.retain(|m| m.asset != asset);
        self.apply_morphs(asset)
    }

    /// The value every loaded morph ends up with: what is set, then what active morphs drive.
    fn effective_values(&self, asset: u32) -> HashMap<String, f32> {
        let mut eff: HashMap<String, f32> = self.morphs.iter().filter(|m| m.asset == asset).map(|m| (m.id.clone(), m.value)).collect();
        for _ in 0..DRIVE_PASSES {
            let mut changed = false;
            let snapshot = eff.clone();
            for ((a, id), morph) in &self.morph_cache.data {
                if *a != asset {
                    continue;
                }
                let v = snapshot.get(id).copied().unwrap_or(0.0);
                if v == 0.0 {
                    continue;
                }
                for f in morph.info.formulas.iter().filter(|f| f.kind == FormulaKind::MorphValue) {
                    if !self.morph_cache.data.contains_key(&(asset, f.target.clone())) {
                        continue;
                    }
                    let driven = f.multiplier * v;
                    if eff.get(&f.target).copied().unwrap_or(0.0) != driven && driven.is_finite() {
                        eff.insert(f.target.clone(), driven);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        eff
    }

    /// Rebuilds the morphed model of an asset from the one it was loaded as.
    pub fn apply_morphs(&mut self, asset: u32) -> Result<(), SceneError> {
        let base = match self.morph_cache.base.get(&asset) {
            Some(b) => b.clone(),
            None => {
                let Some(m) = self.models.0.get(&asset).cloned() else { return Err(SceneError::Parse(format!("model of asset {asset} is not loaded"))) };
                self.morph_cache.base.insert(asset, m.clone());
                m
            }
        };
        let eff = self.effective_values(asset);
        let mut model = (*base).clone();
        let mut bone_world = vec![Vec3::ZERO; model.nodes.len()];
        let copies = self.copies_of(asset, &base);
        // The meshes share one vertex list: the offsets go onto one copy of it, then onto all.
        let mut positions: Vec<[f32; 3]> = base.meshes.first().map(|m| m.positions.clone()).unwrap_or_default();
        let mut changed_vertices = false;
        let node_of: HashMap<&str, usize> = base.nodes.iter().enumerate().map(|(i, n)| (n.name.as_str(), i)).collect();
        for ((a, id), morph) in &self.morph_cache.data {
            if *a != asset {
                continue;
            }
            let v = eff.get(id).copied().unwrap_or(0.0);
            if v == 0.0 {
                continue;
            }
            if !base.vertex_base.is_empty() {
                for d in &morph.deltas {
                    let Some(list) = copies.get(d.vertex as usize) else { continue };
                    let off = mirror(d.delta) * v;
                    for &j in list {
                        if let Some(p) = positions.get_mut(j as usize) {
                            p[0] += off.x;
                            p[1] += off.y;
                            p[2] += off.z;
                        }
                    }
                }
                changed_vertices = true;
            }
            for f in &morph.info.formulas {
                let FormulaKind::BoneCenter(axis) = f.kind else { continue };
                let Some(&i) = node_of.get(f.target.as_str()) else { continue };
                let amount = f.multiplier * v;
                if !amount.is_finite() {
                    continue;
                }
                // The figure's z axis is mirrored.
                match axis {
                    Axis::X => bone_world[i].x += amount,
                    Axis::Y => bone_world[i].y += amount,
                    Axis::Z => bone_world[i].z -= amount,
                }
            }
        }
        if changed_vertices && let Some(start) = base.graft_start {
            // The genital graft is welded to the body along a seam: it follows the body's own
            // morphs through the nearest body vertices (the seam vertices sit on body vertices).
            let links = self.graft_links(asset, &base, &copies, start);
            let rest = base.meshes.first().map(|m| m.positions.as_slice()).unwrap_or(&[]);
            let moved = |b: u32, positions: &[[f32; 3]]| -> Vec3 {
                let Some(&j) = copies.get(b as usize).and_then(|l| l.first()) else { return Vec3::ZERO };
                match (positions.get(j as usize), rest.get(j as usize)) {
                    (Some(p), Some(r)) => Vec3::from(*p) - Vec3::from(*r),
                    _ => Vec3::ZERO,
                }
            };
            let follow: Vec<(u32, Vec3)> = links.iter().map(|(g, near)| (*g, near.iter().map(|(b, w)| moved(*b, &positions) * *w).sum::<Vec3>())).collect();
            for (g, d) in follow {
                for &j in copies.get(g as usize).into_iter().flatten() {
                    if let Some(p) = positions.get_mut(j as usize) {
                        p[0] += d.x;
                        p[1] += d.y;
                        p[2] += d.z;
                    }
                }
            }
        }
        if changed_vertices {
            for mesh in &mut model.meshes {
                if mesh.positions.len() == positions.len() {
                    mesh.positions.clone_from(&positions);
                }
            }
            recompute_normals(&mut model, &base);
        }
        // Bones: the node's local offset is its own world offset minus its parent's, and the skin's
        // inverse bind matrix moves with the bone's world position.
        for (i, n) in base.nodes.iter().enumerate() {
            let parent = n.parent.map_or(Vec3::ZERO, |p| bone_world[p]);
            let local = bone_world[i] - parent;
            if let Some(mn) = model.nodes.get_mut(i) {
                mn.translation = [n.translation[0] + local.x, n.translation[1] + local.y, n.translation[2] + local.z];
            }
        }
        for skin in &mut model.skins {
            for (k, &j) in skin.joints.iter().enumerate() {
                let (Some(m), Some(d)) = (skin.inverse_bind.get_mut(k), bone_world.get(j)) else { continue };
                // A pure translation by -p: the new bone position p + d gives -p - d.
                m[3][0] -= d.x;
                m[3][1] -= d.y;
                m[3][2] -= d.z;
            }
        }
        // The scene's own bone nodes follow by the change since the last build.
        let before = self.morph_cache.offsets.get(&asset).cloned().unwrap_or_else(|| vec![Vec3::ZERO; bone_world.len()]);
        let parent_of: Vec<Option<usize>> = base.nodes.iter().map(|n| n.parent).collect();
        let delta_local = |i: usize, now: &[Vec3], was: &[Vec3]| -> Vec3 {
            let p = parent_of.get(i).copied().flatten();
            (now[i] - was.get(i).copied().unwrap_or(Vec3::ZERO)) - p.map_or(Vec3::ZERO, |p| now[p] - was.get(p).copied().unwrap_or(Vec3::ZERO))
        };
        for n in &mut self.nodes {
            let (NodeKind::Bone, Some((a, idx))) = (&n.kind, n.source) else { continue };
            if a != asset {
                continue;
            }
            let i = idx as usize;
            if i >= bone_world.len() {
                continue;
            }
            let d = delta_local(i, &bone_world, &before);
            for c in 0..3 {
                n.transform.translation[c] += d[c];
                if let Some(rest) = n.rest.as_mut() {
                    rest.translation[c] += d[c];
                }
            }
        }
        self.morph_cache.offsets.insert(asset, bone_world);
        self.apply_material_overrides(asset, &mut model);
        self.models.0.insert(asset, Arc::new(model));
        Ok(())
    }

    /// Puts the overrides' pictures and tints on a freshly built model.
    fn apply_material_overrides(&self, asset: u32, model: &mut ModelData) {
        let mut index: HashMap<String, usize> = HashMap::new();
        for o in self.material_overrides.iter().filter(|o| o.asset == asset) {
            let find = |uid: &Option<String>| uid.as_ref().and_then(|u| self.morph_cache.textures.get(u).map(|t| (u.clone(), t.clone())));
            let (picture, normal, spec) = (find(&o.texture), find(&o.normal), find(&o.specular_map));
            for mi in 0..model.materials.len() {
                if model.materials.get(mi).is_none_or(|m| m.name != o.material) {
                    continue;
                }
                let mut put = |model: &mut ModelData, hit: &Option<(String, Arc<TextureData>)>| -> Option<usize> {
                    let (uid, tex) = hit.as_ref()?;
                    Some(*index.entry(uid.clone()).or_insert_with(|| {
                        model.textures.push(Some((**tex).clone()));
                        model.textures.len() - 1
                    }))
                };
                let (pi, ni, si) = (put(model, &picture), put(model, &normal), put(model, &spec));
                let Some(m) = model.materials.get_mut(mi) else { continue };
                if let Some(i) = pi {
                    m.base_color_texture = Some(i);
                    m.base_color = [1.0, 1.0, 1.0, m.base_color[3]];
                }
                if let Some(i) = ni {
                    m.normal_texture = Some(i);
                    m.normal_scale = o.bump.filter(|b| b.is_finite()).unwrap_or(1.0).clamp(0.0, 4.0);
                }
                if let Some(i) = si {
                    m.specular_texture = Some(i);
                }
                m.specular = o.specular.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0)).or(m.specular);
                m.shininess = o.shininess.filter(|v| v.is_finite()).map(|v| v.clamp(1.0, 512.0)).or(m.shininess);
                if let Some(t) = o.tint {
                    m.base_color = [t[0], t[1], t[2], m.base_color[3]];
                }
            }
        }
    }

    /// The model of an asset as it was loaded, before any morph.
    pub fn morph_cache_base(&self, asset: u32) -> Option<Arc<ModelData>> {
        self.morph_cache.base.get(&asset).cloned()
    }

    /// Forgets the unmorphed model of an asset (a new one was attached).
    pub(crate) fn reset_morph_base(&mut self, asset: u32) {
        self.morph_cache.base.remove(&asset);
        self.morph_cache.graft.remove(&asset);
        self.morph_cache.copies.remove(&asset);
        self.morph_cache.offsets.remove(&asset);
    }

    /// For each base vertex of the genital graft, its nearest body vertices with weights, built once.
    fn graft_links(&mut self, asset: u32, base: &ModelData, copies: &[Vec<u32>], start: u32) -> Arc<Vec<GraftLink>> {
        if let Some(l) = self.morph_cache.graft.get(&asset) {
            return l.clone();
        }
        let rest = base.meshes.first().map(|m| m.positions.as_slice()).unwrap_or(&[]);
        let at = |b: usize| copies.get(b).and_then(|l| l.first()).and_then(|&j| rest.get(j as usize)).map(|p| Vec3::from(*p));
        const CELL: f32 = 0.03;
        let cell = |p: Vec3| ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32, (p.z / CELL).floor() as i32);
        let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for b in 0..(start as usize).min(copies.len()) {
            if let Some(p) = at(b) {
                grid.entry(cell(p)).or_default().push(b as u32);
            }
        }
        let mut links = Vec::new();
        for g in (start as usize)..copies.len() {
            let Some(p) = at(g) else { continue };
            let (cx, cy, cz) = cell(p);
            let mut best: Vec<(f32, u32)> = Vec::new();
            for r in [1, 2, 4] {
                best.clear();
                for dx in -r..=r {
                    for dy in -r..=r {
                        for dz in -r..=r {
                            for &b in grid.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                                if let Some(q) = at(b as usize) {
                                    best.push((p.distance_squared(q), b));
                                }
                            }
                        }
                    }
                }
                if best.len() >= 6 {
                    break;
                }
            }
            best.sort_by(|a, b| a.0.total_cmp(&b.0));
            best.truncate(6);
            let weights: Vec<f32> = best.iter().map(|(d2, _)| 1.0 / (d2 + 1e-7)).collect();
            let total: f32 = weights.iter().sum();
            if total > 0.0 {
                links.push((g as u32, best.iter().zip(&weights).map(|((_, b), w)| (*b, w / total)).collect()));
            }
        }
        let arc = Arc::new(links);
        self.morph_cache.graft.insert(asset, arc.clone());
        arc
    }

    /// The vertices that copy each base vertex of a model, built once.
    fn copies_of(&mut self, asset: u32, base: &ModelData) -> Arc<Vec<Vec<u32>>> {
        if let Some(c) = self.morph_cache.copies.get(&asset) {
            return c.clone();
        }
        let top = base.vertex_base.iter().copied().filter(|&b| b != u32::MAX).max().map_or(0, |m| m as usize + 1);
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); top.min(photocraft_model3d::MAX_VERTICES)];
        for (j, &b) in base.vertex_base.iter().enumerate() {
            if let Some(l) = lists.get_mut(b as usize) {
                l.push(j as u32);
            }
        }
        let arc = Arc::new(lists);
        self.morph_cache.copies.insert(asset, arc.clone());
        arc
    }
}

/// Smooth vertex normals after the vertices moved: faces accumulate onto their base vertices, so a
/// seam between two copies of a vertex shades as one surface.
fn recompute_normals(model: &mut ModelData, base: &ModelData) {
    let n = base.vertex_base.len();
    if n == 0 {
        return;
    }
    let mut acc = vec![Vec3::ZERO; n];
    let Some(first) = model.meshes.first() else { return };
    let pos = |i: usize| first.positions.get(i).map(|p| Vec3::from(*p));
    for mesh in &model.meshes {
        for t in mesh.indices.as_chunks::<3>().0 {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let (Some(pa), Some(pb), Some(pc)) = (pos(a), pos(b), pos(c)) else { continue };
            let face = (pb - pa).cross(pc - pa);
            for v in [a, b, c] {
                if let Some(&bi) = base.vertex_base.get(v)
                    && let Some(slot) = acc.get_mut(bi as usize)
                {
                    *slot += face;
                }
            }
        }
    }
    for mesh in &mut model.meshes {
        for (j, normal) in mesh.normals.iter_mut().enumerate() {
            let Some(&bi) = base.vertex_base.get(j) else { continue };
            let a = acc.get(bi as usize).copied().unwrap_or(Vec3::ZERO).normalize_or_zero();
            if a != Vec3::ZERO {
                *normal = a.to_array();
            }
        }
    }
}
