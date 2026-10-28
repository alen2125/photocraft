//! glTF/GLB parsing into [`ModelData`].

use crate::{
    AlphaMode, MAX_FILE_BYTES, MAX_NODES, MAX_TEXTURE_BYTES, MAX_TEXTURE_EDGE, MAX_VERTICES, MaterialData, MeshData, ModelData, ModelError, ModelFormat,
    ModelNode, SkinData, TextureData,
};

fn parse_err(e: impl std::fmt::Display) -> ModelError {
    ModelError::Parse(e.to_string())
}

/// Reads a `.glb`, `.vrm` or self-contained `.gltf` from memory.
pub fn load(name: &str, bytes: &[u8]) -> Result<ModelData, ModelError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ModelError::TooLarge(bytes.len()));
    }
    let (doc, buffers, images) = gltf::import_slice(bytes).map_err(parse_err)?;
    let count = doc.nodes().len();
    if count > MAX_NODES {
        return Err(ModelError::TooManyNodes(count));
    }

    let mut nodes: Vec<ModelNode> = doc
        .nodes()
        .map(|n| {
            let (translation, rotation, scale) = n.transform().decomposed();
            ModelNode {
                name: n.name().map_or_else(|| format!("Node {}", n.index()), str::to_string),
                parent: None,
                children: n.children().map(|c| c.index()).filter(|&c| c < count).collect(),
                translation,
                rotation,
                scale,
            }
        })
        .collect();
    for i in 0..nodes.len() {
        let kids = nodes.get(i).map(|n| n.children.clone()).unwrap_or_default();
        for k in kids {
            if let Some(child) = nodes.get_mut(k) {
                // A node with two parents is invalid glTF; the first one wins.
                child.parent.get_or_insert(i);
            }
        }
    }
    let roots: Vec<usize> = doc.default_scene().or_else(|| doc.scenes().next()).map(|s| s.nodes().map(|n| n.index()).collect()).unwrap_or_else(|| {
        nodes.iter().enumerate().filter(|(_, n)| n.parent.is_none()).map(|(i, _)| i).collect()
    });

    let skins: Vec<SkinData> = doc
        .skins()
        .map(|s| {
            let reader = s.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let joints: Vec<usize> = s.joints().map(|j| j.index()).collect();
            let mut inverse_bind: Vec<[[f32; 4]; 4]> = reader.read_inverse_bind_matrices().map(|it| it.collect()).unwrap_or_default();
            inverse_bind.resize(joints.len(), identity());
            SkinData { joints, inverse_bind }
        })
        .collect();

    let mut texture_bytes = 0usize;
    let textures: Vec<Option<TextureData>> = images
        .iter()
        .map(|img| {
            let (w, h) = (img.width, img.height);
            let size = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
            if w == 0 || h == 0 || w > MAX_TEXTURE_EDGE || h > MAX_TEXTURE_EDGE || texture_bytes.checked_add(size)? > MAX_TEXTURE_BYTES {
                return None;
            }
            let rgba = to_rgba8(img)?;
            texture_bytes += size;
            Some(TextureData { width: w, height: h, rgba })
        })
        .collect();

    let materials: Vec<MaterialData> = doc.materials().map(|m| material(&m, textures.len())).collect();

    let mut meshes = Vec::new();
    let mut vertex_total = 0usize;
    for node in doc.nodes() {
        let Some(mesh) = node.mesh() else { continue };
        let skin = node.skin().map(|s| s.index()).filter(|&s| s < skins.len());
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(positions) = reader.read_positions().map(|p| p.collect::<Vec<[f32; 3]>>()) else { continue };
            vertex_total = vertex_total.saturating_add(positions.len());
            if vertex_total > MAX_VERTICES {
                return Err(ModelError::TooManyVertices);
            }
            let indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            if indices.iter().any(|&i| i as usize >= positions.len()) {
                return Err(ModelError::Parse("mesh index points past its vertices".into()));
            }
            let uvs: Vec<[f32; 2]> = reader.read_tex_coords(0).map(|t| t.into_f32().collect()).unwrap_or_default();
            let mut joints: Vec<[u16; 4]> = reader.read_joints(0).map(|j| j.into_u16().collect()).unwrap_or_default();
            let mut weights: Vec<[f32; 4]> = reader.read_weights(0).map(|w| w.into_f32().collect()).unwrap_or_default();
            let skinned = skin.is_some() && joints.len() == positions.len() && weights.len() == positions.len();
            if !skinned {
                joints.clear();
                weights.clear();
            }
            let normals = match reader.read_normals().map(|n| n.collect::<Vec<[f32; 3]>>()) {
                Some(n) if n.len() == positions.len() => n,
                _ => flat_normals(&positions, &indices),
            };
            meshes.push(MeshData {
                name: mesh.name().map_or_else(|| format!("Mesh {}", mesh.index()), str::to_string),
                node: node.index(),
                skin: if skinned { skin } else { None },
                uvs: if uvs.len() == positions.len() { uvs } else { Vec::new() },
                positions,
                normals,
                joints,
                weights,
                indices,
                material: prim.material().index().filter(|&m| m < materials.len()),
            });
        }
    }
    if meshes.is_empty() {
        return Err(ModelError::NoMeshes);
    }

    let mut data = ModelData {
        name: name.to_string(),
        format: ModelFormat::Gltf,
        nodes,
        roots,
        meshes,
        skins,
        materials,
        textures,
        humanoid: Vec::new(),
        springs: Vec::new(),
        collider_groups: Vec::new(),
        controls: Vec::new(),
        joint_limits: Vec::new(),
        control_bones: Vec::new(),
        pose_controls: Vec::new(),
        physics: Default::default(),
    };
    crate::vrm::read(&doc, &mut data);
    Ok(data)
}

fn identity() -> [[f32; 4]; 4] {
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

fn material(m: &gltf::Material, image_count: usize) -> MaterialData {
    let pbr = m.pbr_metallic_roughness();
    MaterialData {
        name: m.name().unwrap_or("Material").to_string(),
        base_color: pbr.base_color_factor(),
        base_color_texture: pbr.base_color_texture().map(|t| t.texture().source().index()).filter(|&i| i < image_count),
        emissive: m.emissive_factor(),
        alpha_mode: match m.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        },
        alpha_cutoff: m.alpha_cutoff().unwrap_or(0.5),
        double_sided: m.double_sided(),
        unlit: m.unlit(),
    }
}

fn to_rgba8(img: &gltf::image::Data) -> Option<Vec<u8>> {
    use gltf::image::Format as F;
    let n = (img.width as usize).checked_mul(img.height as usize)?;
    let p = &img.pixels;
    let mut out = Vec::with_capacity(n.checked_mul(4)?);
    match img.format {
        F::R8G8B8A8 => out.extend_from_slice(p.get(..n * 4)?),
        F::R8G8B8 => p.get(..n * 3)?.as_chunks::<3>().0.iter().for_each(|c| out.extend([c[0], c[1], c[2], 255])),
        F::R8G8 => p.get(..n * 2)?.as_chunks::<2>().0.iter().for_each(|c| out.extend([c[0], c[0], c[0], c[1]])),
        F::R8 => p.get(..n)?.iter().for_each(|&v| out.extend([v, v, v, 255])),
        // 16-bit and float textures are rare for colour maps; keep the high byte.
        F::R16G16B16A16 => p.get(..n * 8)?.as_chunks::<8>().0.iter().for_each(|c| out.extend([c[1], c[3], c[5], c[7]])),
        F::R16G16B16 => p.get(..n * 6)?.as_chunks::<6>().0.iter().for_each(|c| out.extend([c[1], c[3], c[5], 255])),
        F::R16G16 => p.get(..n * 4)?.as_chunks::<4>().0.iter().for_each(|c| out.extend([c[1], c[1], c[1], c[3]])),
        F::R16 => p.get(..n * 2)?.as_chunks::<2>().0.iter().for_each(|c| out.extend([c[1], c[1], c[1], 255])),
        _ => return None,
    }
    (out.len() == n * 4).then_some(out)
}

/// Per-vertex normals averaged from face normals, for files that ship none.
fn flat_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut acc = vec![[0.0f32; 3]; positions.len()];
    for tri in indices.as_chunks::<3>().0 {
        let get = |i: u32| positions.get(i as usize).copied();
        let (Some(a), Some(b), Some(c)) = (get(tri[0]), get(tri[1]), get(tri[2])) else { continue };
        let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        for i in tri {
            if let Some(v) = acc.get_mut(*i as usize) {
                for k in 0..3 {
                    v[k] += n[k];
                }
            }
        }
    }
    for v in &mut acc {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        *v = if len > 1e-20 && len.is_finite() { [v[0] / len, v[1] / len, v[2] / len] } else { [0.0, 1.0, 0.0] };
    }
    acc
}
