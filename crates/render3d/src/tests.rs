use super::*;
use photocraft_scene3d::{Node, Primitive, Transform};

fn lit_cube_scene() -> Scene {
    let mut s = Scene::new();
    s.camera.position = [0.0, 0.0, 4.0];
    s.camera.target = [0.0, 0.0, 0.0];
    s.add_primitive(Primitive::cube(), Transform::default()).unwrap();
    s
}

fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
}

#[test]
fn centre_shows_the_cube_and_corner_shows_the_sky() {
    let s = lit_cube_scene();
    let img = render(&s, &RenderOptions { ssaa: 1, ..RenderOptions::new(64, 64) }).unwrap();
    // `background` is already sRGB, so the sky pixel is just that colour in 8 bits.
    let bg = [(s.background[0] * 255.0).round() as i32];
    let corner = px(&img, 1, 1);
    // The sky is the (sRGB round-tripped) background colour and opaque.
    assert!((i32::from(corner[0]) - bg[0]).abs() <= 2, "{corner:?}");
    assert_eq!(corner[3], 255);
    assert_ne!(px(&img, 32, 32)[..3], corner[..3]);
}

#[test]
fn rendering_is_deterministic() {
    let s = lit_cube_scene();
    let o = RenderOptions::new(48, 40);
    assert_eq!(render(&s, &o).unwrap(), render(&s, &o).unwrap());
}

#[test]
fn transparent_background_is_clear_outside_and_opaque_inside() {
    let mut s = lit_cube_scene();
    // Turned, so no silhouette edge lines up with the sample grid.
    s.nodes.last_mut().unwrap().transform.rotation = [20.0, 30.0, 10.0];
    let img = render(&s, &RenderOptions { transparent: true, ssaa: 2, ..RenderOptions::new(64, 64) }).unwrap();
    assert_eq!(px(&img, 1, 1)[3], 0);
    assert_eq!(px(&img, 32, 32)[3], 255);
    // Anti-aliased silhouette: some edge pixel is partly covered.
    assert!(img.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[3] < 255));
}

#[test]
fn a_light_changes_the_shading() {
    let mut dark = lit_cube_scene();
    dark.nodes.retain(|n| !matches!(n.kind, NodeKind::Light(_)));
    let lit = lit_cube_scene();
    let o = RenderOptions { ssaa: 1, ..RenderOptions::new(32, 32) };
    assert_ne!(px(&render(&dark, &o).unwrap(), 16, 16), px(&render(&lit, &o).unwrap(), 16, 16));
}

#[test]
fn hidden_nodes_are_not_drawn_and_children_inherit_hiding() {
    let mut s = lit_cube_scene();
    let o = RenderOptions { transparent: true, ssaa: 1, ..RenderOptions::new(32, 32) };
    let shown = render(&s, &o).unwrap();
    let id = s.nodes.iter().find(|n| matches!(n.kind, NodeKind::Primitive { .. })).unwrap().id;
    s.node_mut(id).unwrap().visible = false;
    let child = s.add(Node { parent: Some(id), kind: s.node(id).unwrap().kind.clone(), ..Node::default() }).unwrap();
    assert!(!s.is_visible(child));
    let hidden = render(&s, &o).unwrap();
    assert_eq!(px(&shown, 16, 16)[3], 255);
    assert_eq!(px(&hidden, 16, 16)[3], 0);
}

#[test]
fn camera_inside_geometry_and_degenerate_cameras_do_not_panic() {
    let mut s = lit_cube_scene();
    s.camera.position = [0.0, 0.0, 0.0]; // inside the cube: near-plane clipping path
    let o = RenderOptions { ssaa: 1, ..RenderOptions::new(16, 16) };
    render(&s, &o).unwrap();
    s.camera.target = s.camera.position; // zero-length view direction
    render(&s, &o).unwrap();
    s.camera.position = [f32::NAN, 0.0, 0.0];
    s.camera.fov = f32::INFINITY;
    s.camera.near = -5.0;
    s.camera.far = 0.0;
    render(&s, &o).unwrap();
    s.camera = Default::default();
    s.camera.target = [s.camera.position[0], s.camera.position[1] + 5.0, s.camera.position[2]]; // straight up
    render(&s, &o).unwrap();
}

#[test]
fn bad_sizes_are_errors() {
    let s = Scene::new();
    assert_eq!(render(&s, &RenderOptions::new(0, 10)), Err(RenderError::BadSize(0, 10)));
    assert_eq!(render(&s, &RenderOptions::new(MAX_EDGE + 1, 10)), Err(RenderError::BadSize(MAX_EDGE + 1, 10)));
    assert_eq!(render(&s, &RenderOptions { ssaa: 4, ..RenderOptions::new(MAX_EDGE, MAX_EDGE) }), Err(RenderError::TooBig));
}

#[test]
fn hostile_materials_and_transforms_render() {
    let mut s = lit_cube_scene();
    let id = s.add_primitive(Primitive::sphere(), Transform { translation: [f32::NAN; 3], rotation: [1e30; 3], scale: [0.0, 1e30, f32::NAN] }).unwrap();
    if let Some(NodeKind::Primitive { material, .. }) = s.node_mut(id).map(|n| &mut n.kind) {
        material.shininess = f32::NAN;
        material.color = [f32::INFINITY, -4.0, f32::NAN];
    }
    render(&s, &RenderOptions { ssaa: 1, ..RenderOptions::new(16, 16) }).unwrap();
}

#[test]
fn picking_finds_the_front_node() {
    let mut s = lit_cube_scene();
    let front = s.add_primitive(Primitive::sphere(), Transform::at(0.0, 0.0, 1.5)).unwrap();
    assert_eq!(pick(&s, 64, 64, 32.0, 32.0), Some(front));
    assert_eq!(pick(&s, 64, 64, 0.0, 0.0), None);
    assert_eq!(pick(&s, 0, 64, 1.0, 1.0), None);
    assert_eq!(pick(&s, 64, 64, f32::NAN, 1.0), None);
}

#[test]
fn spot_light_only_lights_inside_its_cone() {
    let mut s = Scene::default();
    s.ambient = [0.0; 3];
    s.camera.position = [0.0, 6.0, 0.001];
    s.camera.target = [0.0, 0.0, 0.0];
    s.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    s.add(Node {
        transform: Transform { translation: [0.0, 4.0, 0.0], rotation: [-90.0, 0.0, 0.0], scale: [1.0; 3] },
        kind: NodeKind::Light(Light { kind: LightKind::Spot { angle: 30.0 }, intensity: 3.0, range: 30.0, ..Light::default() }),
        ..Node::default()
    })
    .unwrap();
    let img = render(&s, &RenderOptions { ssaa: 1, ..RenderOptions::new(64, 64) }).unwrap();
    let centre = px(&img, 32, 32);
    let edge = px(&img, 2, 2);
    assert!(centre[0] > 40, "centre should be lit: {centre:?}");
    assert!(edge[0] < 5, "outside the cone stays dark: {edge:?}");
}

#[test]
fn project_puts_the_origin_at_the_picture_centre_and_rejects_nodes_behind_the_camera() {
    let mut s = lit_cube_scene();
    let id = s.add_primitive(Primitive::cube(), Transform::default()).unwrap();
    let (x, y) = project(&s, 100, 50, id).unwrap();
    assert!((x - 50.0).abs() < 0.01 && (y - 25.0).abs() < 0.01, "{x},{y}");
    let behind = s.add_primitive(Primitive::cube(), Transform::at(0.0, 0.0, 10.0)).unwrap();
    assert_eq!(project(&s, 100, 50, behind), None);
    assert_eq!(project(&s, 100, 50, 9999), None);
    assert_eq!(project(&s, 0, 50, id), None);
}
