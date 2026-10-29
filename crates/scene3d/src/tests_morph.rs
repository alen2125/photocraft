use std::sync::Arc;

use glam::Vec3;
use photocraft_model3d::morph::{Axis, FormulaKind, Morph, MorphDelta, MorphFormula, MorphInfo};
use photocraft_model3d::{MeshData, ModelData, ModelFormat, ModelNode, SkinData};

use super::*;

fn mnode(name: &str, parent: Option<usize>, children: &[usize], t: [f32; 3]) -> ModelNode {
    ModelNode { name: name.into(), parent, children: children.to_vec(), translation: t, rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
}

/// A two-bone figure (hip, head one unit above) with four vertices; vertex 3 copies vertex 0 (a seam).
fn figure() -> Arc<ModelData> {
    let ibm = |y: f32| [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, -y, 0.0, 1.0]];
    let mesh = |name: &str, indices: Vec<u32>| MeshData {
        name: name.into(),
        node: 2,
        skin: Some(0),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        joints: vec![[0; 4]; 4],
        weights: vec![[1.0, 0.0, 0.0, 0.0]; 4],
        indices,
        ..MeshData::default()
    };
    Arc::new(ModelData {
        name: "fig.vrm".into(),
        format: ModelFormat::Gltf,
        nodes: vec![mnode("hip", None, &[1], [0.0; 3]), mnode("head", Some(0), &[], [0.0, 1.0, 0.0]), mnode("body", None, &[], [0.0; 3])],
        roots: vec![0, 2],
        meshes: vec![mesh("A", vec![0, 1, 2]), mesh("B", vec![3, 1, 2])],
        skins: vec![SkinData { joints: vec![0, 1], inverse_bind: vec![ibm(0.0), ibm(1.0)] }],
        materials: vec![],
        textures: vec![],
        humanoid: vec![],
        springs: vec![],
        collider_groups: vec![],
        controls: vec![],
        joint_limits: vec![],
        control_bones: vec![],
        pose_controls: vec![],
        physics: Default::default(),
        vertex_base: vec![0, 1, 2, 0],
        graft_start: None,
    })
}

fn morph(id: &str, deltas: &[(u32, [f32; 3])], formulas: Vec<MorphFormula>) -> Arc<Morph> {
    Arc::new(Morph {
        info: MorphInfo { id: id.into(), display_name: id.into(), group: "G".into(), region: "R".into(), min: -1.0, max: 1.0, num_deltas: deltas.len(), formulas },
        deltas: deltas.iter().map(|&(vertex, delta)| MorphDelta { vertex, delta }).collect(),
    })
}

fn scene_with_figure() -> (Scene, u32) {
    let mut s = Scene::default();
    s.add_model(figure(), None).unwrap();
    (s, 1)
}

#[test]
fn a_morph_moves_every_copy_of_its_base_vertex_with_the_axis_mirrored_and_scaled_by_its_value() {
    let (mut s, a) = scene_with_figure();
    s.add_morph_data(a, morph("Wide", &[(0, [0.0, 0.0, 0.5]), (1, [0.2, 0.0, 0.0])], vec![])).unwrap();
    s.set_morph(a, "Wide", 0.5).unwrap();
    let m = s.model(a).unwrap().clone();
    for mesh in &m.meshes {
        assert_eq!(mesh.positions.len(), 4);
    }
    let p = &m.meshes[0].positions;
    assert!((Vec3::from(p[0]) - Vec3::new(0.0, 0.0, -0.25)).length() < 1e-6, "z is mirrored: {:?}", p[0]);
    assert!((Vec3::from(p[3]) - Vec3::from(p[0])).length() < 1e-6, "the seam copy moves with its twin");
    assert!((Vec3::from(p[1]) - Vec3::new(1.1, 0.0, 0.0)).length() < 1e-6);
    assert_eq!(m.meshes[1].positions, m.meshes[0].positions, "all meshes share the vertex list");
    assert_eq!(s.morph_value(a, "Wide"), 0.5);
    // Back to zero: the model is the one that was loaded.
    s.set_morph(a, "Wide", 0.0).unwrap();
    assert_eq!(s.model(a).unwrap().meshes[0].positions, figure().meshes[0].positions);
    assert!(s.morphs.is_empty(), "zero values are not kept");
}

#[test]
fn bone_formulas_move_the_bone_its_node_and_its_inverse_bind_matrix_together() {
    let (mut s, a) = scene_with_figure();
    let f = |axis, target: &str, m| MorphFormula { kind: FormulaKind::BoneCenter(axis), target: target.into(), multiplier: m };
    s.add_morph_data(a, morph("Tall", &[], vec![f(Axis::Y, "head", 0.2), f(Axis::Z, "head", 0.1), f(Axis::Y, "hip", 0.05), f(Axis::X, "nobody", 9.0)])).unwrap();
    let head = s.nodes.iter().find(|n| n.name == "head").unwrap().id;
    let head_before = s.world_matrix(head).unwrap().transform_point3(Vec3::ZERO);
    s.set_morph(a, "Tall", 1.0).unwrap();
    let w = s.world_matrix(head).unwrap().transform_point3(Vec3::ZERO);
    assert!((w - head_before - Vec3::new(0.0, 0.2, -0.1)).length() < 1e-5, "head moved by its own offset only (the hip's is its own): {w:?}");
    let m = s.model(a).unwrap();
    assert!((Vec3::from(m.nodes[1].translation) - Vec3::new(0.0, 1.15, -0.1)).length() < 1e-5, "local offset is own minus parent's");
    assert!((m.skins[0].inverse_bind[1][3][1] - (-1.2)).abs() < 1e-5 && (m.skins[0].inverse_bind[1][3][2] - 0.1).abs() < 1e-5, "{:?}", m.skins[0].inverse_bind[1]);
    // Taking the morph away moves the node back by the difference.
    s.set_morph(a, "Tall", 0.0).unwrap();
    assert!((s.world_matrix(head).unwrap().transform_point3(Vec3::ZERO) - head_before).length() < 1e-5, "back where it was");
}

#[test]
fn a_morph_can_drive_another_and_values_clamp_and_missing_ones_are_errors() {
    let (mut s, a) = scene_with_figure();
    let drive = MorphFormula { kind: FormulaKind::MorphValue, target: "Child".into(), multiplier: -2.0 };
    s.add_morph_data(a, morph("Master", &[], vec![drive])).unwrap();
    s.add_morph_data(a, morph("Child", &[(1, [1.0, 0.0, 0.0])], vec![])).unwrap();
    s.set_morph(a, "Master", 0.25).unwrap();
    let x = s.model(a).unwrap().meshes[0].positions[1][0];
    assert!((x - 0.5).abs() < 1e-6, "child = -2 * 0.25 = -0.5, so x = 1 - 0.5: {x}");
    assert!(s.set_morph(a, "Nope", 1.0).is_err());
    assert!(s.set_morph(99, "Master", 1.0).is_err());
    s.set_morph(a, "Master", f32::NAN).unwrap();
    assert_eq!(s.morph_value(a, "Master"), 0.0);
    s.set_morph(a, "Master", 1.0e9).unwrap();
    assert!(s.model(a).unwrap().meshes[0].positions.iter().all(|p| p.iter().all(|c| c.is_finite())));
}

#[test]
fn morph_values_survive_the_scene_file_and_apply_again_when_the_model_returns() {
    let (mut s, a) = scene_with_figure();
    s.add_morph_data(a, morph("Wide", &[(1, [0.2, 0.0, 0.0])], vec![])).unwrap();
    s.set_morph(a, "Wide", 1.0).unwrap();
    let json = s.to_json().unwrap();
    let mut back = Scene::from_json(&json).unwrap();
    assert_eq!(back.morphs, s.morphs);
    // Models and morph data come from elsewhere after a load; then the morph applies.
    back.attach_model(a, figure()).unwrap();
    assert!((back.model(a).unwrap().meshes[0].positions[1][0] - 1.0).abs() < 1e-6, "no data yet: the base model");
    back.add_morph_data(a, morph("Wide", &[(1, [0.2, 0.0, 0.0])], vec![])).unwrap();
    back.apply_morphs(a).unwrap();
    assert!((back.model(a).unwrap().meshes[0].positions[1][0] - 1.2).abs() < 1e-6);
}

#[test]
fn the_real_figure_takes_a_real_morph_without_tearing() {
    let dir = std::env::var("VAM_DIR").unwrap_or_else(|_| r"F:\Virt-a-Mate_1.22.0.3\VaM_1.22.0.3".into());
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/vam_base_female.vrm");
    let morphs = std::path::Path::new(&dir).join("Custom/Atom/Person/Morphs/female/Athletic");
    let (Ok(bytes), Ok(vmi), Ok(vmb)) = (std::fs::read(base), std::fs::read(morphs.with_extension("vmi")), std::fs::read(morphs.with_extension("vmb"))) else { return };
    let model = Arc::new(photocraft_model3d::load("vam.vrm", &bytes).unwrap());
    let before = model.meshes[0].positions.clone();
    let mut s = Scene::default();
    s.add_model(model, None).unwrap();
    s.add_morph_data(1, Arc::new(photocraft_model3d::morph::parse_morph(&vmi, &vmb).unwrap())).unwrap();
    let id = s.morph_infos(1)[0].id.clone();
    s.set_morph(1, &id, 1.0).unwrap();
    let after = s.model(1).unwrap().meshes[0].positions.clone();
    let moved = before.iter().zip(&after).filter(|(a, b)| Vec3::from(**a).distance(Vec3::from(**b)) > 1e-4).count();
    let far = before.iter().zip(&after).filter(|(a, b)| Vec3::from(**a).distance(Vec3::from(**b)) > 0.3).count();
    assert!(moved > 1000, "the morph reshaped the body: {moved} vertices moved");
    assert_eq!(far, 0, "no vertex flew off (the mirror and units are right)");
    // Seam copies stay together.
    let base_of = s.model(1).unwrap().vertex_base.clone();
    for (j, &b) in base_of.iter().enumerate().take(5000) {
        let b = b as usize;
        if b < j && Vec3::from(before[j]).distance(Vec3::from(before[b])) < 1e-6 {
            assert!(Vec3::from(after[j]).distance(Vec3::from(after[b])) < 1e-4, "copy {j} of {b} came apart");
        }
    }
    s.set_morph(1, &id, 0.0).unwrap();
    assert_eq!(s.model(1).unwrap().meshes[0].positions, before);
}

#[test]
fn a_real_genital_morph_lands_on_the_graft_vertices_only() {
    let dir = std::env::var("VAM_DIR").unwrap_or_else(|_| r"F:\Virt-a-Mate_1.22.0.3\VaM_1.22.0.3".into());
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/vam_base_female.vrm");
    let morphs = std::path::Path::new(&dir).join("Custom/Atom/Person/Morphs/female_genitalia");
    let (Ok(bytes), Ok(rd)) = (std::fs::read(base), std::fs::read_dir(&morphs)) else { return };
    let Some(vmi) = rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "vmi")) else { return };
    let Ok(vmb) = std::fs::read(vmi.with_extension("vmb")) else { return };
    let model = Arc::new(photocraft_model3d::load("vam.vrm", &bytes).unwrap());
    let start = model.graft_start.expect("the figure knows where its genital graft starts");
    let before = model.meshes[0].positions.clone();
    let mut m = photocraft_model3d::morph::parse_morph(&std::fs::read(&vmi).unwrap(), &vmb).unwrap();
    for d in &mut m.deltas {
        d.vertex += start;
    }
    let mut s = Scene::default();
    s.add_model(model, None).unwrap();
    let id = m.info.id.clone();
    s.add_morph_data(1, Arc::new(m)).unwrap();
    s.set_morph(1, &id, 1.0).unwrap();
    let after = s.model(1).unwrap().meshes[0].positions.clone();
    let base_of = s.model(1).unwrap().vertex_base.clone();
    let moved: Vec<usize> = (0..before.len()).filter(|&j| Vec3::from(before[j]).distance(Vec3::from(after[j])) > 1e-5).collect();
    assert!(!moved.is_empty(), "the genital morph reshaped something");
    assert!(moved.iter().all(|&j| base_of[j] >= start), "only graft vertices moved");
}
