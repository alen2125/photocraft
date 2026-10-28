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
        }],
        textures: vec![Some(TextureData { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 255, 0, 0] })],
        humanoid: vec![],
        springs: vec![],
        collider_groups: vec![],
        controls: vec![],
        joint_limits: vec![],
        control_bones: vec![],
        pose_controls: vec![],
        physics: Default::default(),
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
