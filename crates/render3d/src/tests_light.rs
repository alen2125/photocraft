//! Render modes and shadows of lights (the Virt-a-Mate light settings).

use super::*;
use photocraft_scene3d::{LightRender, Node, Primitive, Transform};

const W: u32 = 96;

/// A floor with a cube floating above its middle, a light straight over the cube, a camera off to the side.
fn scene(kind: LightKind, light: impl FnOnce(&mut Light)) -> Scene {
    let mut s = Scene::default();
    s.ambient = [0.0; 3];
    s.camera.position = [0.0, 4.0, 8.0];
    s.camera.target = [0.0, 0.0, 0.0];
    s.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    s.add_primitive(Primitive::cube(), Transform::at(0.0, 2.0, 0.0)).unwrap();
    let mut l = Light { kind, intensity: 3.0, range: 30.0, render_mode: LightRender::ForcePixel, ..Light::default() };
    light(&mut l);
    let y = if matches!(kind, LightKind::Directional) { 5.0 } else { 6.0 };
    s.add(Node { transform: Transform { translation: [0.0, y, 0.0], rotation: [-90.0, 0.0, 0.0], scale: [1.0; 3] }, kind: NodeKind::Light(l), ..Node::default() }).unwrap();
    s
}

/// Brightness of the floor at a world point.
fn floor(s: &Scene, x: f32, z: f32) -> i32 {
    let img = render(s, &RenderOptions { ssaa: 1, ..RenderOptions::new(W, W) }).unwrap();
    let (px, py) = project_point(s, W, W, Vec3::new(x, 0.0, z)).unwrap();
    i32::from(img.rgba[((py as u32 * W + px as u32) * 4) as usize])
}

const KINDS: [LightKind; 3] = [LightKind::Spot { angle: 90.0 }, LightKind::Point, LightKind::Directional];

#[test]
fn a_pixel_light_throws_the_cube_s_shadow_on_the_floor() {
    for kind in KINDS {
        let on = scene(kind, |_| {});
        let off = scene(kind, |l| l.cast_shadows = false);
        let (lit_off, under_off) = (floor(&off, 2.5, 0.0), floor(&off, 0.0, 0.0));
        let (lit_on, under_on) = (floor(&on, 2.5, 0.0), floor(&on, 0.0, 0.0));
        assert!(under_off > 40, "{kind:?}: the floor under the cube is lit when nothing casts: {under_off}");
        assert!(under_on * 4 < under_off, "{kind:?}: shadowed {under_on} vs lit {under_off}");
        // Away from the cube nothing shadows the floor (no self-shadowing acne).
        assert!((lit_on - lit_off).abs() <= 3, "{kind:?}: floor outside the shadow changed: {lit_on} vs {lit_off}");
    }
}

#[test]
fn shadow_strength_sets_how_dark_the_shadow_is() {
    for kind in KINDS {
        let full = floor(&scene(kind, |l| l.shadow_strength = 1.0), 0.0, 0.0);
        let half = floor(&scene(kind, |l| l.shadow_strength = 0.5), 0.0, 0.0);
        let none = floor(&scene(kind, |l| l.shadow_strength = 0.0), 0.0, 0.0);
        assert!(full < half && half < none, "{kind:?}: {full} < {half} < {none}");
        let off = floor(&scene(kind, |l| l.cast_shadows = false), 0.0, 0.0);
        assert!((none - off).abs() <= 1, "{kind:?}: strength 0 is no shadow: {none} vs {off}");
    }
}

#[test]
fn vertex_lights_cast_no_shadows() {
    let shadowed = floor(&scene(LightKind::Spot { angle: 90.0 }, |l| l.render_mode = LightRender::ForceVertex), 0.0, 0.0);
    let off = floor(&scene(LightKind::Spot { angle: 90.0 }, |l| {
        l.render_mode = LightRender::ForceVertex;
        l.cast_shadows = false;
    }), 0.0, 0.0);
    assert_eq!(shadowed, off);
}

#[test]
fn a_vertex_light_is_lit_at_the_corners_a_pixel_light_in_between() {
    // One big quad: a spot over its middle lights the middle per pixel, but its corners are outside the cone.
    let build = |mode| {
        let mut s = Scene::default();
        s.ambient = [0.0; 3];
        s.camera.position = [0.0, 6.0, 0.001];
        s.camera.target = [0.0, 0.0, 0.0];
        s.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
        let light = Light { kind: LightKind::Spot { angle: 30.0 }, intensity: 3.0, range: 30.0, render_mode: mode, ..Light::default() };
        s.add(Node { transform: Transform { translation: [0.0, 4.0, 0.0], rotation: [-90.0, 0.0, 0.0], scale: [1.0; 3] }, kind: NodeKind::Light(light), ..Node::default() }).unwrap();
        render(&s, &RenderOptions { ssaa: 1, ..RenderOptions::new(64, 64) }).unwrap()
    };
    let at_centre = |img: &Image| img.rgba[((32 * 64 + 32) * 4) as usize];
    assert!(at_centre(&build(LightRender::ForcePixel)) > 40);
    assert!(at_centre(&build(LightRender::ForceVertex)) < 5);
}

#[test]
fn point_bias_reaches_only_point_lights_and_a_small_bias_keeps_the_shadow() {
    // The bias reaches only point lights (Virt-a-Mate's Point Bias): a spot ignores it.
    let spot = |bias: f32| floor(&scene(LightKind::Spot { angle: 90.0 }, |l| l.point_bias = bias), 0.0, 0.0);
    assert_eq!(spot(0.0), spot(0.03));
    let point = |bias: f32| floor(&scene(LightKind::Point, |l| l.point_bias = bias), 0.0, 0.0);
    assert!(point(0.0) < 15 && point(0.03) < 15, "a small bias keeps the shadow: {} {}", point(0.0), point(0.03));
}

#[test]
fn hostile_light_settings_render() {
    for kind in KINDS {
        for v in [f32::NAN, f32::INFINITY, -3.0, 1e30, 0.0] {
            let s = scene(kind, |l| {
                l.shadow_strength = v;
                l.point_bias = v;
                l.range = v;
                l.intensity = v;
            });
            render(&s, &RenderOptions { ssaa: 1, ..RenderOptions::new(16, 16) }).unwrap();
        }
    }
    // Shadow casters that are not there, and a light inside the geometry.
    let mut s = Scene::default();
    s.add(Node { kind: NodeKind::Light(Light { render_mode: LightRender::ForcePixel, ..Light::default() }), ..Node::default() }).unwrap();
    render(&s, &RenderOptions { ssaa: 1, ..RenderOptions::new(16, 16) }).unwrap();
}

#[test]
#[ignore = "timing, run with --nocapture"]
fn timing_of_shadow_maps() {
    for kind in KINDS {
        for (name, cast) in [("shadows", true), ("no shadows", false)] {
            let s = scene(kind, |l| l.cast_shadows = cast);
            let o = RenderOptions { ssaa: 1, ..RenderOptions::new(640, 640) };
            let t = std::time::Instant::now();
            for _ in 0..5 {
                render(&s, &o).unwrap();
            }
            println!("{kind:?} {name}: {:?} per render", t.elapsed() / 5);
        }
    }
}
