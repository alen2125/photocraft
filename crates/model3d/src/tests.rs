use super::*;

use crate::testgen::skinned_triangle_glb as glb;

#[test]
fn reads_nodes_skin_mesh_and_material() {
    let m = load("tri.glb", &glb(false, 2)).unwrap();
    assert_eq!(m.format, ModelFormat::Gltf);
    assert_eq!(m.nodes.len(), 3);
    assert_eq!(m.nodes[1].parent, Some(0));
    assert_eq!(m.nodes[1].translation, [0.0, 1.0, 0.0]);
    assert_eq!(m.roots, vec![0, 2]);
    assert_eq!(m.skins.len(), 1);
    assert_eq!(m.skins[0].joints, vec![0, 1]);
    assert_eq!(m.skins[0].inverse_bind[1][3], [0.0, -1.0, 0.0, 1.0]);
    assert!(m.is_joint(1) && !m.is_joint(2));
    let mesh = &m.meshes[0];
    assert_eq!((mesh.node, mesh.skin, mesh.positions.len(), mesh.indices.len()), (2, Some(0), 3, 3));
    assert_eq!(mesh.joints.len(), 3);
    assert!((mesh.weights[2][0] - 0.5).abs() < 1e-6);
    // No normals in the file: they are computed (the triangle faces +Z).
    assert!(mesh.normals.iter().all(|n| n[2] > 0.99));
    let mat = &m.materials[0];
    assert_eq!((mat.alpha_mode, mat.alpha_cutoff), (AlphaMode::Mask, 0.3));
    assert!((mat.base_color[0] - 0.9).abs() < 1e-6);
}

#[test]
fn reads_vrm0_humanoid_and_spring_bones_and_drops_bad_references() {
    let m = load("tri.vrm", &glb(true, 2)).unwrap();
    assert_eq!(m.format, ModelFormat::Vrm0);
    assert_eq!(m.humanoid_node("hips"), Some(0));
    assert_eq!(m.humanoid_node("spine"), Some(1));
    assert_eq!(m.humanoid_node("bad"), None, "node 99 does not exist");
    assert_eq!(m.collider_groups.len(), 1);
    assert_eq!(m.collider_groups[0].spheres, vec![([0.0, 0.1, 0.0], 0.05)]);
    let s = &m.springs[0];
    assert_eq!((s.comment.as_str(), s.stiffness, s.drag_force), ("hair", 0.5, 0.4));
    assert_eq!(s.roots, vec![1], "bone 77 does not exist");
    assert_eq!(s.collider_groups, vec![0], "collider group 5 does not exist");
    assert_eq!(s.gravity_dir, [0.0, -1.0, 0.0]);
    assert_eq!(m.controls, vec![1], "the PC_controls list keeps real nodes only");
    assert!(load("tri.glb", &glb(false, 2)).unwrap().controls.is_empty(), "no extension, no list");
}

#[test]
fn hostile_files_are_errors_not_panics() {
    assert!(matches!(load("x", b""), Err(ModelError::Parse(_))));
    assert!(matches!(load("x", b"glTF not really"), Err(ModelError::Parse(_))));
    assert!(matches!(load("x", &[0u8; 64]), Err(ModelError::Parse(_))));
    let good = glb(false, 2);
    for cut in [11, 20, 100, good.len() - 1] {
        assert!(load("x", &good[..cut]).is_err(), "truncated at {cut}");
    }
    // An index past the vertices is refused rather than read.
    assert!(matches!(load("x", &glb(false, 40)), Err(ModelError::Parse(_))));
    // A text file that is not glTF.
    assert!(load("x", br#"{"hello":"world"}"#).is_err());
}

/// The real VRM from `samples/` (gitignored, so absent on other machines).
#[test]
fn a_real_vrm_loads_with_skeleton_textures_and_springs() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/8590256991748008892.vrm");
    let Ok(bytes) = std::fs::read(&path) else { return };
    let m = load("real.vrm", &bytes).unwrap();
    assert_eq!(m.format, ModelFormat::Vrm0);
    assert!(m.humanoid_node("hips").is_some() && m.humanoid_node("leftUpperArm").is_some());
    assert!(!m.skins.is_empty() && m.skins[0].joints.len() > 50);
    assert!(m.meshes.iter().any(|x| x.skin.is_some()));
    assert!(m.textures.iter().flatten().count() > 0);
    assert!(!m.springs.is_empty());
    for mesh in &m.meshes {
        assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.positions.len()));
        if let Some(skin) = mesh.skin {
            let skin = &m.skins[skin];
            assert!(mesh.joints.iter().flatten().all(|&j| (j as usize) < skin.joints.len()));
        }
    }
}
