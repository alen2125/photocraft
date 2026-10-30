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
    // The pose-slider list keeps what is well formed and drops the rest.
    assert_eq!(m.control_bones.len(), 1);
    assert_eq!((m.control_bones[0].node, m.control_bones[0].order), (1, 2));
    assert_eq!(m.pose_controls.len(), 1, "a nameless slider and one with a bad range are dropped");
    let c = &m.pose_controls[0];
    assert_eq!((c.name.as_str(), c.label.as_str(), c.group.as_str(), c.min, c.max), ("Bend", "Bend it", "Hand/Left", -1.0, 1.0));
    assert_eq!(c.rot.len(), 1, "joints that do not exist or have short angles are dropped");
    assert_eq!((c.rot[0].node, c.rot[0].deg), (1, [0.0, 0.0, 45.0]));
    assert_eq!(c.links, vec![("Other".to_string(), 0.5)]);
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

#[test]
fn the_vam_figures_say_which_base_vertex_each_vertex_copies() {
    for who in ["female", "male"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../samples/vam_base_{who}.vrm"));
        let Ok(bytes) = std::fs::read(path) else { return };
        let m = crate::load("vam.vrm", &bytes).unwrap();
        let n = m.meshes.first().map_or(0, |mesh| mesh.positions.len());
        assert!(n > 20_000 && m.vertex_base.len() == n, "{who}: one base index per vertex: {} vs {n}", m.vertex_base.len());
        assert!(m.vertex_base.iter().all(|b| (*b as usize) < n));
        assert!(m.meshes.iter().all(|mesh| mesh.positions.len() == n), "every primitive shares the vertex list");
    }
}

#[test]
fn the_real_morph_packs_read_and_their_offsets_point_at_real_vertices() {
    for who in ["female", "male"] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        let (Ok(pack), Ok(figure)) = (std::fs::read(dir.join(format!("vam_base_{who}.pcmorphs"))), std::fs::read(dir.join(format!("vam_base_{who}.vrm")))) else { return };
        let model = crate::load("vam.vrm", &figure).unwrap();
        let t = std::time::Instant::now();
        let pack = crate::morph::MorphPack::parse(pack).unwrap();
        assert!(pack.entries.len() > 800, "{who}: {} morphs", pack.entries.len());
        let i = pack.find("Glutes Size").or_else(|| pack.find("Hip Size")).unwrap_or(0);
        let m = pack.load(i).unwrap();
        assert_eq!(m.deltas.len(), m.info.num_deltas);
        let n = model.vertex_base.len() as u32;
        assert!(m.deltas.iter().all(|d| d.vertex < n), "{who}: an offset past the figure's vertices");
        eprintln!("PACK {who}: {} morphs parsed in {:?}", pack.entries.len(), t.elapsed());
    }
}

#[test]
fn the_built_in_genital_morphs_are_marked_and_number_their_vertices_inside_the_graft() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples");
    let (Ok(pack), Ok(figure)) = (std::fs::read(dir.join("vam_base_female.pcmorphs")), std::fs::read(dir.join("vam_base_female.vrm"))) else { return };
    let model = crate::load("vam.vrm", &figure).unwrap();
    let graft = model.vertex_base.iter().filter(|&&b| b >= model.graft_start.unwrap_or(u32::MAX)).count() as u32;
    let pack = crate::morph::MorphPack::parse(pack).unwrap();
    let i = pack.find("Gen_Innie").expect("the pack has Gen_Innie");
    assert!(pack.entries[i].info.genital, "Gen_Innie is a genital morph");
    let m = pack.load(i).unwrap();
    assert!(m.deltas.iter().all(|d| d.vertex < graft), "its numbers are inside the graft ({graft} vertices)");
    assert!(pack.entries.iter().filter(|e| e.info.genital).count() > 50);
    let body = pack.find("Hip Size").unwrap();
    assert!(!pack.entries[body].info.genital);
}
