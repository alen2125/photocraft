use std::sync::Arc;

use glam::{Quat, Vec3};
use photocraft_model3d::{MeshData, ModelData, ModelFormat, ModelNode, SkinData};

use super::*;

fn mnode(name: &str, parent: Option<usize>, children: &[usize], t: [f32; 3]) -> ModelNode {
    ModelNode { name: name.into(), parent, children: children.to_vec(), translation: t, rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
}

/// hips -> spine (1 up) -> head (1 up), a skinned mesh on node 3.
fn model(format: ModelFormat) -> Arc<ModelData> {
    let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    Arc::new(ModelData {
        name: "dir/hero.vrm".into(),
        format,
        nodes: vec![
            mnode("hips", None, &[1], [0.0; 3]),
            mnode("spine", Some(0), &[2], [0.0, 1.0, 0.0]),
            mnode("head", Some(1), &[], [0.0, 1.0, 0.0]),
            mnode("body", None, &[], [0.0; 3]),
        ],
        roots: vec![0, 3],
        meshes: vec![MeshData {
            name: "Body".into(),
            node: 3,
            skin: Some(0),
            positions: vec![[0.0; 3]; 3],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            joints: vec![[0; 4]; 3],
            weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            ..MeshData::default()
        }],
        skins: vec![SkinData { joints: vec![0, 1, 2], inverse_bind: vec![id; 3] }],
        materials: vec![],
        textures: vec![],
        humanoid: vec![("hips".into(), 0)],
        springs: vec![],
        collider_groups: vec![],
        controls: vec![],
        joint_limits: vec![],
        control_bones: vec![],
        pose_controls: vec![],
    })
}

fn bone(s: &Scene, name: &str) -> NodeId {
    s.nodes.iter().find(|n| n.name == name).unwrap().id
}

#[test]
fn adding_a_model_builds_bones_a_mesh_and_an_asset() {
    let mut s = Scene::default();
    let root = s.add_model(model(ModelFormat::Gltf), Some("dir/hero.vrm".into())).unwrap();
    assert_eq!(s.node(root).unwrap().name, "hero");
    assert_eq!(s.assets.len(), 1);
    assert_eq!(s.nodes.len(), 1 + 4 + 1);
    let hips = bone(&s, "hips");
    assert!(matches!(s.node(hips).unwrap().kind, NodeKind::Bone));
    assert_eq!(s.node(hips).unwrap().parent, Some(root));
    assert!(matches!(s.node(bone(&s, "body")).unwrap().kind, NodeKind::Empty));
    let mesh = s.nodes.iter().find_map(|n| if let NodeKind::Mesh(m) = &n.kind { Some(m.clone()) } else { None }).unwrap();
    assert_eq!(mesh.joints, vec![hips, bone(&s, "spine"), bone(&s, "head")]);
    // The head is two units above the hips in the world.
    let w = s.world_matrix(bone(&s, "head")).unwrap().transform_point3(Vec3::ZERO);
    assert!((w - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-5);
    s.validate().unwrap();
}

#[test]
fn vrm0_models_are_turned_to_face_the_camera() {
    let mut s = Scene::default();
    let root = s.add_model(model(ModelFormat::Vrm0), None).unwrap();
    assert_eq!(s.node(root).unwrap().transform.rotation, [0.0, 180.0, 0.0]);
}

#[test]
fn a_saved_scene_remembers_the_asset_but_not_the_geometry() {
    let mut s = Scene::default();
    s.add_model(model(ModelFormat::Gltf), Some("dir/hero.vrm".into())).unwrap();
    let mut back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(back.nodes, s.nodes);
    assert_eq!(back.missing_assets().len(), 1);
    assert_eq!(back.missing_assets()[0].path.as_deref(), Some("dir/hero.vrm"));
    back.attach_model(back.assets[0].id, model(ModelFormat::Gltf)).unwrap();
    assert!(back.missing_assets().is_empty());
    assert!(back.attach_model(999, model(ModelFormat::Gltf)).is_err());
}

#[test]
fn a_model_that_does_not_fit_adds_nothing() {
    let mut s = Scene::default();
    for _ in 0..MAX_NODES - 3 {
        s.add(Node::default()).unwrap();
    }
    let before = s.nodes.len();
    assert_eq!(s.add_model(model(ModelFormat::Gltf), None), Err(SceneError::TooManyNodes));
    assert_eq!((s.nodes.len(), s.assets.len()), (before, 0));
}

#[test]
fn world_matrices_agree_with_world_matrix() {
    let mut s = Scene::default();
    s.add_model(model(ModelFormat::Vrm0), None).unwrap();
    let all = s.world_matrices();
    assert_eq!(all.len(), s.nodes.len());
    for n in &s.nodes {
        let a = all[&n.id].to_cols_array();
        let b = s.world_matrix(n.id).unwrap().to_cols_array();
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5), "{}", n.name);
    }
}

#[test]
fn poses_capture_apply_and_reset() {
    let mut s = Scene::default();
    s.add_model(model(ModelFormat::Gltf), None).unwrap();
    let spine = bone(&s, "spine");
    s.node_mut(spine).unwrap().transform.rotation = [0.0, 0.0, 40.0];
    let i = s.capture_pose("lean");
    assert_eq!(s.capture_pose("lean"), i, "same name replaces");
    s.reset_pose();
    assert_eq!(s.node(spine).unwrap().transform.rotation, [0.0; 3]);
    s.apply_pose(i).unwrap();
    assert_eq!(s.node(spine).unwrap().transform.rotation, [0.0, 0.0, 40.0]);
    assert!(s.apply_pose(99).is_err());
    let back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(back.poses, s.poses);
}

#[test]
fn rotating_a_bone_moves_what_hangs_below_it() {
    let mut s = Scene::default();
    s.add_model(model(ModelFormat::Gltf), None).unwrap();
    let (hips, head) = (bone(&s, "hips"), bone(&s, "head"));
    rotate_world(&mut s, hips, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)).unwrap();
    let p = s.world_matrix(head).unwrap().transform_point3(Vec3::ZERO);
    assert!((p - Vec3::new(-2.0, 0.0, 0.0)).length() < 1e-4, "{p:?}");
    assert!(rotate_world(&mut s, 9999, Quat::IDENTITY).is_err());
    rotate_world(&mut s, hips, Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)).unwrap();
}

#[test]
fn ik_reaches_a_target_without_stretching_bones() {
    let mut s = Scene::default();
    s.add_model(model(ModelFormat::Gltf), None).unwrap();
    let head = bone(&s, "head");
    let r = solve_ik(&mut s, head, Vec3::new(1.2, 1.2, 0.0), 2, 30).unwrap();
    assert!(r.distance < 0.01, "{r:?}");
    let world = s.world_matrices();
    let (hips, spine) = (world[&bone(&s, "hips")].transform_point3(Vec3::ZERO), world[&bone(&s, "spine")].transform_point3(Vec3::ZERO));
    assert!((spine.distance(hips) - 1.0).abs() < 1e-3, "bone length kept");
    assert!((world[&head].transform_point3(Vec3::ZERO).distance(spine) - 1.0).abs() < 1e-3);
}

#[test]
fn ik_to_an_unreachable_target_points_at_it_and_stays_finite() {
    let mut s = Scene::default();
    s.add_model(model(ModelFormat::Gltf), None).unwrap();
    let head = bone(&s, "head");
    let r = solve_ik(&mut s, head, Vec3::new(50.0, 0.0, 0.0), 2, 30).unwrap();
    assert!(r.distance.is_finite() && r.distance > 40.0);
    let p = s.world_matrix(head).unwrap().transform_point3(Vec3::ZERO);
    assert!(p.x > 1.9 && p.is_finite(), "{p:?}");
    assert!(solve_ik(&mut s, head, Vec3::NAN, 2, 5).is_err());
    assert!(solve_ik(&mut s, 12345, Vec3::ZERO, 2, 5).is_err());
    assert!(solve_ik(&mut s, head, Vec3::ONE, 0, 0).is_ok());
}

#[test]
fn joint_limits_hold_during_ik_and_rotation() {
    let mut s = Scene::default();
    s.add_model(model(ModelFormat::Gltf), None).unwrap();
    let (hips, spine, head) = (bone(&s, "hips"), bone(&s, "spine"), bone(&s, "head"));
    let locked = RotationLimits { min: [0.0; 3], max: [0.0; 3] };
    s.node_mut(hips).unwrap().limits = Some(locked.clone());
    s.node_mut(spine).unwrap().limits = Some(RotationLimits { min: [0.0, 0.0, -10.0], max: [0.0, 0.0, 10.0] });
    solve_ik(&mut s, head, Vec3::new(2.0, 0.5, 0.0), 2, 30).unwrap();
    assert_eq!(s.node(hips).unwrap().transform.rotation, [0.0; 3]);
    let z = s.node(spine).unwrap().transform.rotation[2];
    assert!((-10.001..=10.001).contains(&z), "{z}");
    rotate_world(&mut s, hips, Quat::from_rotation_x(1.0)).unwrap();
    assert_eq!(s.node(hips).unwrap().transform.rotation, [0.0; 3]);
    assert_eq!(locked.clamp([f32::NAN, 5.0, -5.0]), [0.0; 3]);
}

#[test]
fn translate_to_puts_a_bone_at_a_world_position_through_its_parents() {
    let mut s = Scene::default();
    let root = s.add_model(model(ModelFormat::Vrm0), None).unwrap();
    s.node_mut(root).unwrap().transform.translation = [5.0, 0.0, 0.0];
    let (hips, head) = (bone(&s, "hips"), bone(&s, "head"));
    translate_to(&mut s, hips, Vec3::new(1.0, 2.0, 3.0)).unwrap();
    let p = s.world_matrix(hips).unwrap().transform_point3(Vec3::ZERO);
    assert!((p - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-4, "{p:?}");
    // The whole body came along: the head is still two units from the hips.
    let h = s.world_matrix(head).unwrap().transform_point3(Vec3::ZERO);
    assert!((h.distance(p) - 2.0).abs() < 1e-4);
    assert!(translate_to(&mut s, hips, Vec3::NAN).is_err());
    assert!(translate_to(&mut s, 4242, Vec3::ZERO).is_err());
}

#[test]
fn joints_outside_the_controls_list_are_minor_and_the_skeleton_skips_them() {
    let mut m = (*model(ModelFormat::Gltf)).clone();
    m.controls = vec![0, 2]; // hips and head are controls, the spine is not
    let mut s = Scene::default();
    s.add_model(Arc::new(m), None).unwrap();
    let (hips, spine, head) = (bone(&s, "hips"), bone(&s, "spine"), bone(&s, "head"));
    assert!(!s.node(hips).unwrap().minor && s.node(spine).unwrap().minor && !s.node(head).unwrap().minor);
    // Not a joint, so never minor.
    assert!(!s.node(bone(&s, "body")).unwrap().minor);
    let shown = s.bone_positions();
    assert_eq!(shown.len(), 2);
    assert!(shown.iter().all(|b| b.0 != spine));
    // The head's line goes to the hips, past the hidden spine; the spine is still posable.
    assert_eq!(shown.iter().find(|b| b.0 == head).unwrap().1, Some(hips));
    crate::rotate_world(&mut s, spine, Quat::from_rotation_z(0.5)).unwrap();
    let moved = s.world_matrix(head).unwrap().transform_point3(Vec3::ZERO);
    assert!((moved - Vec3::new(0.0, 2.0, 0.0)).length() > 0.1, "turning a minor joint still moves what hangs from it");
    // No list means every joint counts, and the flag survives a save.
    let mut all = Scene::default();
    all.add_model(model(ModelFormat::Gltf), None).unwrap();
    assert_eq!(all.bone_positions().len(), 3);
    let back = Scene::from_json(&s.to_json().unwrap()).unwrap();
    assert!(back.node(spine).unwrap().minor);
}
