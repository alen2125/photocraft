use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::{Node, NodeKind, Primitive, Scene, Transform};

use super::*;

fn floor() -> Scene {
    let mut s = Scene::default();
    s.add_primitive(Primitive::Plane { size: [20.0, 20.0] }, Transform::default()).unwrap();
    s
}

fn drop_at(s: &mut Scene, p: Primitive, at: [f32; 3]) -> NodeId {
    let id = s.add_primitive(p, Transform::at(at[0], at[1], at[2])).unwrap();
    s.node_mut(id).unwrap().pinned = false;
    id
}

fn pos(s: &Scene, id: NodeId) -> Vec3 {
    s.world_matrix(id).unwrap().transform_point3(Vec3::ZERO)
}

fn run(s: &mut Scene, seconds: f32) -> World {
    let mut w = World::new(s);
    for _ in 0..(seconds / STEP) as usize {
        w.step();
    }
    w.apply(s);
    w
}

#[test]
fn an_unpinned_cube_falls_and_rests_on_the_floor() {
    let mut s = floor();
    let cube = drop_at(&mut s, Primitive::cube(), [0.0, 5.0, 0.0]);
    // Bodies fall asleep after about two seconds of rest.
    let w = run(&mut s, 6.0);
    assert_eq!(w.dynamic_count(), 1);
    let p = pos(&s, cube);
    assert!((p.y - 0.5).abs() < 0.03, "it came to rest on its face, half a unit up: {p:?}");
    assert!(p.x.abs() < 0.05 && p.z.abs() < 0.05, "straight down");
    assert!(w.settled());
}

#[test]
fn a_pinned_solid_stays_exactly_where_it_is() {
    let mut s = floor();
    let held = s.add_primitive(Primitive::cube(), Transform::at(0.0, 5.0, 0.0)).unwrap();
    let before = s.node(held).unwrap().transform.clone();
    let w = run(&mut s, 2.0);
    assert_eq!(w.dynamic_count(), 0);
    assert_eq!(s.node(held).unwrap().transform, before);
}

#[test]
fn a_falling_cube_lands_on_a_pinned_one_and_the_pinned_one_does_not_move() {
    let mut s = floor();
    let shelf = s.add_primitive(Primitive::Cube { size: [4.0, 1.0, 4.0] }, Transform::at(0.0, 2.0, 0.0)).unwrap();
    let ball = drop_at(&mut s, Primitive::sphere(), [0.2, 6.0, 0.1]);
    run(&mut s, 4.0);
    assert_eq!(pos(&s, shelf), Vec3::new(0.0, 2.0, 0.0));
    assert!((pos(&s, ball).y - 3.0).abs() < 0.1, "it sits on the shelf's top (2.5) by its radius (0.5): {:?}", pos(&s, ball));
}

#[test]
fn falling_solids_stack_and_do_not_pass_through_each_other() {
    let mut s = floor();
    let lower = drop_at(&mut s, Primitive::cube(), [0.0, 1.0, 0.0]);
    let upper = drop_at(&mut s, Primitive::cube(), [0.0, 3.0, 0.0]);
    run(&mut s, 4.0);
    let (a, b) = (pos(&s, lower), pos(&s, upper));
    assert!((a.y - 0.5).abs() < 0.05 && (b.y - 1.5).abs() < 0.08, "stacked: {a:?} {b:?}");
}

#[test]
fn every_primitive_shape_comes_to_rest_above_the_floor() {
    let mut s = floor();
    let ids: Vec<NodeId> = [Primitive::cube(), Primitive::sphere(), Primitive::cylinder(), Primitive::cone()]
        .into_iter()
        .enumerate()
        .map(|(i, p)| drop_at(&mut s, p, [i as f32 * 3.0 - 4.5, 4.0, 0.0]))
        .collect();
    let w = run(&mut s, 6.0);
    for id in ids {
        let y = pos(&s, id).y;
        assert!((0.1..1.0).contains(&y), "resting near the floor, not through it or in the air: {y}");
    }
    assert!(w.dynamic_count() == 4);
}

#[test]
fn the_same_scene_always_settles_the_same_way() {
    let build = || {
        let mut s = floor();
        drop_at(&mut s, Primitive::cube(), [0.3, 4.0, 0.1]);
        drop_at(&mut s, Primitive::sphere(), [0.0, 6.0, 0.0]);
        s
    };
    let (mut a, mut b) = (build(), build());
    run(&mut a, 3.0);
    run(&mut b, 3.0);
    assert_eq!(a.nodes, b.nodes);
}

#[test]
fn scale_rotation_and_parents_are_respected() {
    let mut s = floor();
    // A cube twice as big falls onto the floor and rests a unit up (half of its scaled 2).
    let big = drop_at(&mut s, Primitive::cube(), [0.0, 5.0, 0.0]);
    s.node_mut(big).unwrap().transform.scale = [2.0, 2.0, 2.0];
    // A cube under a moved parent: its world place is what falls, and it ends up on the floor.
    let holder = s.add(Node { transform: Transform::at(6.0, 0.0, 0.0), ..Node::default() }).unwrap();
    let child = s.add_primitive(Primitive::cube(), Transform::at(0.0, 5.0, 0.0)).unwrap();
    s.set_parent(child, Some(holder)).unwrap();
    s.node_mut(child).unwrap().pinned = false;
    run(&mut s, 4.0);
    assert!((pos(&s, big).y - 1.0).abs() < 0.06, "{:?}", pos(&s, big));
    let c = pos(&s, child);
    assert!((c.y - 0.5).abs() < 0.05 && (c.x - 6.0).abs() < 0.1, "{c:?}");
}

#[test]
fn hidden_and_non_solid_nodes_are_left_out_and_bad_numbers_do_not_matter() {
    let mut s = floor();
    let hidden = drop_at(&mut s, Primitive::cube(), [0.0, 5.0, 0.0]);
    s.node_mut(hidden).unwrap().visible = false;
    s.add(Node { kind: NodeKind::Bone, ..Node::default() }).unwrap();
    let nasty = drop_at(&mut s, Primitive::Sphere { radius: f32::NAN, segments: 0 }, [f32::NAN, 4.0, 0.0]);
    let flat = drop_at(&mut s, Primitive::Cube { size: [0.0, f32::INFINITY, -3.0] }, [0.0, 4.0, 0.0]);
    s.node_mut(flat).unwrap().transform.scale = [0.0, 0.0, 0.0];
    let before = s.node(hidden).unwrap().transform.clone();
    let w = run(&mut s, 2.0);
    assert_eq!(s.node(hidden).unwrap().transform, before, "hidden: not simulated");
    assert!(w.dynamic_count() <= 1, "the NaN one is skipped, the degenerate one is clamped, never a crash");
    let _ = nasty;
    // Nothing falls: stepping does nothing and nothing is written.
    let mut empty = Scene::default();
    let mut w = World::new(&empty);
    w.step();
    assert!(w.poses().is_empty() && w.settled());
    w.apply(&mut empty);
}

#[test]
fn a_crowd_beyond_the_limit_is_trimmed_not_refused() {
    let mut s = floor();
    for i in 0..(MAX_DYNAMIC + 20) {
        drop_at(&mut s, Primitive::sphere(), [(i % 16) as f32 * 1.2 - 9.0, 2.0 + (i / 16) as f32 * 1.1, 0.0]);
    }
    let mut w = World::new(&s);
    assert_eq!(w.dynamic_count(), MAX_DYNAMIC);
    for _ in 0..5 {
        w.step();
    }
    assert_eq!(w.poses().len(), MAX_DYNAMIC);
}
