use std::sync::Arc;

use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::model3d::{AlphaMode, MaterialData, MeshData, ModelData, ModelFormat, ModelNode, SkinData, TextureData};
use photocraft_scene3d::{Node, NodeKind, rotate_world};

use super::*;

fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
}

/// A 2×2-unit quad textured with two texels side by side: opaque red on the left, transparent
/// green on the right. `skinned` binds it to one bone at the origin; otherwise a plain node.
fn quad_model(skinned: bool, mode: AlphaMode) -> Arc<ModelData> {
    let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let node = |name: &str| ModelNode { name: name.into(), parent: None, children: vec![], translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] };
    Arc::new(ModelData {
        name: "quad.glb".into(),
        format: ModelFormat::Gltf,
        nodes: vec![node("bone"), node("mesh")],
        roots: vec![0, 1],
        meshes: vec![MeshData {
            name: "Quad".into(),
            node: 1,
            skin: skinned.then_some(0),
            positions: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            joints: if skinned { vec![[0; 4]; 4] } else { vec![] },
            weights: if skinned { vec![[1.0, 0.0, 0.0, 0.0]; 4] } else { vec![] },
            indices: vec![0, 1, 2, 0, 2, 3],
            material: Some(0),
        }],
        skins: if skinned { vec![SkinData { joints: vec![0], inverse_bind: vec![id] }] } else { vec![] },
        materials: vec![MaterialData {
            name: "m".into(),
            base_color: [1.0; 4],
            base_color_texture: Some(0),
            emissive: [0.0; 3],
            alpha_mode: mode,
            alpha_cutoff: 0.5,
            double_sided: true,
            unlit: false,
            normal_texture: None,
            normal_scale: 1.0,
            spec_normal_scale: 1.0,
            specular_texture: None,
            specular: None,
            shininess: None,
        }],
        textures: vec![Some(TextureData { width: 2, height: 1, rgba: std::sync::Arc::new(vec![255, 0, 0, 255, 0, 255, 0, 0]) })],
        humanoid: vec![],
        springs: vec![],
        collider_groups: vec![],
        controls: vec![],
        joint_limits: vec![],
        control_bones: vec![],
        pose_controls: vec![],
        physics: Default::default(),
        vertex_base: Vec::new(),
        graft_start: None,
    })
}

fn bright_scene(model: Arc<ModelData>) -> Scene {
    let mut s = Scene::default();
    s.ambient = [1.0; 3];
    s.background = [0.0; 3];
    s.camera.position = [0.0, 0.0, 4.0];
    s.camera.target = [0.0; 3];
    s.add_model(model, None).unwrap();
    s
}

fn opts() -> RenderOptions {
    RenderOptions { ssaa: 1, transparent: true, ..RenderOptions::new(64, 64) }
}

#[test]
fn texture_colour_shows_and_transparent_texels_are_cut_out() {
    for skinned in [false, true] {
        let s = bright_scene(quad_model(skinned, AlphaMode::Mask));
        let img = render(&s, &opts()).unwrap();
        let left = px(&img, 22, 32);
        let right = px(&img, 42, 32);
        assert!(left[0] > 200 && left[1] < 40 && left[3] == 255, "skinned={skinned}: red texel, got {left:?}");
        assert_eq!(right[3], 0, "skinned={skinned}: transparent texel is not drawn, got {right:?}");
    }
}

#[test]
fn an_opaque_material_ignores_texture_alpha() {
    let s = bright_scene(quad_model(false, AlphaMode::Opaque));
    let img = render(&s, &opts()).unwrap();
    assert_eq!(px(&img, 42, 32)[3], 255);
}

#[test]
fn posing_a_bone_moves_the_skinned_mesh() {
    let mut s = bright_scene(quad_model(true, AlphaMode::Opaque));
    let before = render(&s, &opts()).unwrap();
    let bone = s.nodes.iter().find(|n| n.name == "bone").unwrap().id;
    // Turn the quad edge-on to the camera: it all but disappears.
    rotate_world(&mut s, bone, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)).unwrap();
    let after = render(&s, &opts()).unwrap();
    let count = |i: &Image| i.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count();
    assert!(count(&before) > 800 && count(&after) < 60, "{} -> {}", count(&before), count(&after));
}

#[test]
fn model_meshes_can_be_picked_and_bones_projected() {
    let s = bright_scene(quad_model(true, AlphaMode::Opaque));
    let mesh = s.nodes.iter().find(|n| matches!(n.kind, NodeKind::Mesh(_))).unwrap().id;
    assert_eq!(pick(&s, 64, 64, 32.0, 32.0), Some(mesh));
    assert_eq!(pick(&s, 64, 64, 1.0, 1.0), None);
    let pts = project_points(&s, 100, 50, &[Vec3::ZERO, Vec3::new(0.0, 0.0, 50.0), Vec3::NAN]);
    let (x, y) = pts[0].unwrap();
    assert!((x - 50.0).abs() < 0.01 && (y - 25.0).abs() < 0.01);
    assert_eq!((pts[1], pts[2]), (None, None));
    let (o, d) = pixel_ray(&s, 100, 50, 50.0, 25.0).unwrap();
    assert!((d - Vec3::NEG_Z).length() < 1e-3 && o.z > 3.0, "{o:?} {d:?}");
    assert_eq!(pixel_ray(&s, 0, 50, 1.0, 1.0), None);
}

#[test]
fn broken_model_references_are_skipped_not_fatal() {
    let mut s = bright_scene(quad_model(true, AlphaMode::Opaque));
    // Point the mesh at a mesh that does not exist and give another a wrong joint count.
    for n in &mut s.nodes {
        if let NodeKind::Mesh(m) = &mut n.kind {
            m.joints.push(9999);
        }
    }
    s.add(Node { kind: NodeKind::Mesh(photocraft_scene3d::ModelMesh { asset: 77, mesh: 0, joints: vec![] }), ..Node::default() }).unwrap();
    s.add(Node { kind: NodeKind::Mesh(photocraft_scene3d::ModelMesh { asset: 1, mesh: 40, joints: vec![] }), ..Node::default() }).unwrap();
    render(&s, &opts()).unwrap();
}

#[test]
fn a_scene_without_its_model_loaded_still_renders_the_rest() {
    let s = bright_scene(quad_model(true, AlphaMode::Opaque));
    let mut back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(back.missing_assets().len(), 1);
    let img = render(&back, &opts()).unwrap();
    assert_eq!(px(&img, 32, 32)[3], 0, "no model, nothing drawn");
    back.attach_model(back.assets[0].id, quad_model(true, AlphaMode::Opaque)).unwrap();
    assert_eq!(px(&render(&back, &opts()).unwrap(), 32, 32)[3], 255);
}

/// The quad lit by one directional light that shines along the quad's plane from the right, so a
/// flat quad gets almost no light and a normal map that tilts the surface towards it gets some.
fn lit_quad(normal: Option<[u8; 4]>, spec: Option<[u8; 4]>) -> Image {
    let mut model = (*quad_model(false, AlphaMode::Opaque)).clone();
    let tex = |p: [u8; 4]| Some(TextureData { width: 1, height: 1, rgba: Arc::new(p.to_vec()) });
    model.textures[0] = tex([255, 255, 255, 255]);
    if let Some(n) = normal {
        model.textures.push(tex(n));
        model.materials[0].normal_texture = Some(model.textures.len() - 1);
    }
    if let Some(s) = spec {
        model.textures.push(tex(s));
        model.materials[0].specular_texture = Some(model.textures.len() - 1);
    }
    let mut s = bright_scene(Arc::new(model));
    s.ambient = [0.0; 3];
    s.nodes.retain(|n| !matches!(n.kind, NodeKind::Light(_)));
    let mut light = Node { kind: NodeKind::Light(photocraft_scene3d::Light { kind: photocraft_scene3d::LightKind::Directional, intensity: 2.0, ..Default::default() }), ..Node::default() };
    // Directional lights shine along their own -Z: turn it to travel along -X (from the right).
    light.transform.rotation = [0.0, 90.0, 0.0];
    s.add(light).unwrap();
    render(&s, &opts()).unwrap()
}

#[test]
fn a_normal_map_changes_the_shading_and_a_flat_one_does_not() {
    let plain = px(&lit_quad(None, None), 32, 32);
    let flat = px(&lit_quad(Some([128, 128, 255, 255]), None), 32, 32);
    let tilted = px(&lit_quad(Some([255, 128, 128, 255]), None), 32, 32);
    assert!(plain[0] < 20, "a light along the plane barely lights a flat quad: {plain:?}");
    assert!(flat[0].abs_diff(plain[0]) <= 30, "a flat map changes nothing: {flat:?} vs {plain:?}");
    assert!(tilted[0] > plain[0] + 80, "a map that tilts the surface towards the light lights it: {tilted:?} vs {plain:?}");
}

#[test]
fn a_specular_map_scales_the_highlight() {
    let lit = |spec| {
        let mut model = (*quad_model(false, AlphaMode::Opaque)).clone();
        model.textures[0] = Some(TextureData { width: 1, height: 1, rgba: Arc::new(vec![0, 0, 0, 255]) });
        model.materials[0].specular = Some(0.15);
        model.materials[0].shininess = Some(8.0);
        if let Some(s) = spec {
            model.textures.push(Some(TextureData { width: 1, height: 1, rgba: Arc::new(s) }));
            model.materials[0].specular_texture = Some(model.textures.len() - 1);
        }
        let mut s = bright_scene(Arc::new(model));
        s.ambient = [0.0; 3];
        s.nodes.retain(|n| !matches!(n.kind, NodeKind::Light(_)));
        let mut light = Node { kind: NodeKind::Light(photocraft_scene3d::Light { kind: photocraft_scene3d::LightKind::Directional, intensity: 2.0, ..Default::default() }), ..Node::default() };
        light.transform.rotation = [0.0, 0.0, 0.0];
        s.add(light).unwrap();
        px(&render(&s, &opts()).unwrap(), 32, 32)
    };
    let (full, none, half) = (lit(None), lit(Some(vec![0, 0, 0, 255])), lit(Some(vec![128, 0, 0, 255])));
    assert!(full[0] > 60 && none[0] < 5, "{full:?} {none:?}");
    assert!(half[0] > none[0] && half[0] < full[0], "{half:?}");
}

#[test]
fn a_blending_material_mixes_over_what_is_behind_it_and_leaves_the_depth_alone() {
    let mut behind = (*quad_model(false, AlphaMode::Opaque)).clone();
    behind.textures[0] = Some(TextureData { width: 1, height: 1, rgba: Arc::new(vec![0, 0, 255, 255]) });
    let mut glass = (*quad_model(false, AlphaMode::Blend)).clone();
    glass.textures[0] = Some(TextureData { width: 1, height: 1, rgba: Arc::new(vec![255, 0, 0, 128]) });
    let mut s = bright_scene(Arc::new(behind));
    // The glass is drawn first in the node list's order of models but sits in front.
    let front = s.add_model(Arc::new(glass), None).unwrap();
    s.node_mut(front).unwrap().transform.translation = [0.0, 0.0, 0.5];
    let c = px(&render(&s, &opts()).unwrap(), 32, 32);
    assert!(c[0] > 60 && c[2] > 60 && c[1] < 20, "half red over blue is a mix of both: {c:?}");
}

#[test]
fn the_two_normal_strengths_shape_the_light_and_the_shine_separately() {
    // The map tilts the surface towards a light along the plane; with no strength for the light the
    // quad stays dark, with a strength it is lit. The shine uses its own number.
    let lit = |light_scale: f32, shine_scale: f32| {
        let mut model = (*quad_model(false, AlphaMode::Opaque)).clone();
        model.textures[0] = Some(TextureData { width: 1, height: 1, rgba: Arc::new(vec![255, 255, 255, 255]) });
        model.textures.push(Some(TextureData { width: 1, height: 1, rgba: Arc::new(vec![255, 128, 128, 255]) }));
        let m = &mut model.materials[0];
        (m.normal_texture, m.normal_scale, m.spec_normal_scale) = (Some(1), light_scale, shine_scale);
        let mut s = bright_scene(Arc::new(model));
        s.ambient = [0.0; 3];
        s.nodes.retain(|n| !matches!(n.kind, NodeKind::Light(_)));
        let mut light = Node { kind: NodeKind::Light(photocraft_scene3d::Light { kind: photocraft_scene3d::LightKind::Directional, intensity: 2.0, ..Default::default() }), ..Node::default() };
        light.transform.rotation = [0.0, 90.0, 0.0];
        s.add(light).unwrap();
        px(&render(&s, &opts()).unwrap(), 32, 32)
    };
    let (flat, bent) = (lit(0.0, 1.0), lit(1.0, 1.0));
    assert!(flat[0] < 40 && bent[0] > flat[0] + 60, "strength 0 keeps the surface unlit, 1 lights it: {flat:?} {bent:?}");
}
