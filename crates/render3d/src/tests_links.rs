use photocraft_scene3d::{Link, LinkSpace, Node, Primitive, Transform};

use super::*;

fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
}

/// A big floor seen from above, a marker node at the floor's middle, and a spot light up in
/// the air that has no turn of its own (so on its own it shines along −Z, away from the floor's
/// middle). Returns the scene and the light's id.
fn floor_and_spot() -> (Scene, photocraft_scene3d::NodeId, photocraft_scene3d::NodeId) {
    let mut s = Scene::default();
    s.ambient = [0.0; 3];
    s.camera.position = [0.0, 8.0, 0.001];
    s.camera.target = [0.0; 3];
    s.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    let marker = s.add(Node { name: "face".into(), ..Node::default() }).unwrap();
    let spot = s
        .add(Node {
            transform: Transform::at(0.0, 4.0, 0.0),
            kind: NodeKind::Light(Light { kind: LightKind::Spot { angle: 30.0 }, intensity: 3.0, range: 40.0, ..Light::default() }),
            ..Node::default()
        })
        .unwrap();
    (s, marker, spot)
}

fn opts() -> RenderOptions {
    RenderOptions { ssaa: 1, ..RenderOptions::new(64, 64) }
}

#[test]
fn a_spot_light_linked_to_a_node_lights_that_node_without_being_turned_by_hand() {
    let (mut s, marker, spot) = floor_and_spot();
    let unlinked = render(&s, &opts()).unwrap();
    assert!(px(&unlinked, 32, 32)[0] < 5, "pointing the wrong way, it lights nothing");
    s.set_link(spot, Some(Link { target: marker, distance: 4.0, yaw: 0.0, pitch: 89.0, look_at: true, space: LinkSpace::World })).unwrap();
    let linked = render(&s, &opts()).unwrap();
    assert!(px(&linked, 32, 32)[0] > 40, "aimed at the marker it lights the floor under it: {:?}", px(&linked, 32, 32));
    assert!(px(&linked, 2, 2)[0] < 5, "and only there");
}

#[test]
fn moving_the_target_moves_the_pool_of_light() {
    let (mut s, marker, spot) = floor_and_spot();
    s.set_link(spot, Some(Link { target: marker, distance: 4.0, yaw: 0.0, pitch: 89.0, look_at: true, space: LinkSpace::World })).unwrap();
    let before = render(&s, &opts()).unwrap();
    s.node_mut(marker).unwrap().transform.translation = [3.0, 0.0, 0.0];
    let after = render(&s, &opts()).unwrap();
    // Looking straight down with Z as "up", screen-right is world -X, so +X is on the left.
    assert!(px(&before, 32, 32)[0] > px(&before, 6, 32)[0]);
    assert!(px(&after, 6, 32)[0] > px(&after, 32, 32)[0], "{:?} {:?}", px(&after, 6, 32), px(&after, 32, 32));
}
