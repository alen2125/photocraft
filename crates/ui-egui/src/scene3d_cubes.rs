//! Hand controls drawn as small shaded cubes that turn with their bone, so which way a bone faces
//! can be read at a glance, and coloured by what is On: green when the position is On, orange when
//! only the orientation is, red when neither (the same code as VaM's controllers). Also the cone
//! tips of the gizmo's arrows.

use egui::{Color32, Pos2, Stroke};
use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{Node, Scene};

use crate::scene3d_ui::camera_axes;

pub(crate) const GREEN: Color32 = Color32::from_rgb(40, 225, 60);
pub(crate) const ORANGE: Color32 = Color32::from_rgb(255, 150, 20);
pub(crate) const RED: Color32 = Color32::from_rgb(238, 40, 36);
/// Half the side of a bone's cube, in preview pixels.
pub(crate) const HALF_PX: f32 = 7.0;

/// The colour of a bone's cube: position On is green whatever the orientation; position Off with
/// orientation On is orange; both Off is red.
pub(crate) fn bone_color(node: &Node) -> Color32 {
    if node.hold.is_some() {
        GREEN
    } else if node.hold_rot.is_some() {
        ORANGE
    } else {
        RED
    }
}

/// The six faces of a cube: the outward normal, and the corner indices (into the eight corners
/// numbered by bits x = 1, y = 2, z = 4, 1 meaning the positive side) going round the face.
const FACES: [([f32; 3], [usize; 4]); 6] = [
    ([1.0, 0.0, 0.0], [1, 3, 7, 5]),
    ([-1.0, 0.0, 0.0], [0, 4, 6, 2]),
    ([0.0, 1.0, 0.0], [2, 6, 7, 3]),
    ([0.0, -1.0, 0.0], [0, 1, 5, 4]),
    ([0.0, 0.0, 1.0], [4, 5, 7, 6]),
    ([0.0, 0.0, -1.0], [0, 2, 3, 1]),
];

/// World size of `px` preview pixels at `depth` units in front of the camera.
pub(crate) fn px_to_world(scene: &Scene, view_h: u32, depth: f32, px: f32) -> f32 {
    let half_fov = (scene.camera.fov.clamp(1.0, 170.0)).to_radians() * 0.5;
    px * 2.0 * depth.max(1e-4) * half_fov.tan() / view_h.max(1) as f32
}

/// The faces of a cube at `centre` (turned by `rot`, half-side `half`) that face the camera,
/// farthest first, each as (outward normal in the world, the four corners in the world).
pub(crate) fn visible_faces(eye: Vec3, centre: Vec3, rot: Quat, half: f32) -> Vec<(Vec3, [Vec3; 4])> {
    let corner = |i: usize| centre + rot * Vec3::new(if i & 1 != 0 { half } else { -half }, if i & 2 != 0 { half } else { -half }, if i & 4 != 0 { half } else { -half });
    let mut out: Vec<(f32, Vec3, [Vec3; 4])> = FACES
        .iter()
        .filter_map(|(n, idx)| {
            let normal = rot * Vec3::from(*n);
            let face_centre = centre + normal * half;
            // A face seen almost edge-on is a sliver that only draws a stripe: leave it out.
            let look = (face_centre - eye).normalize_or_zero();
            (normal.dot(look) < -0.08).then(|| (face_centre.distance(eye), normal, idx.map(corner)))
        })
        .collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.into_iter().map(|(_, n, c)| (n, c)).collect()
}

/// Draws a shaded cube. `to_screen` maps preview pixels to the screen; `half_px` is the cube's
/// half-side in preview pixels at its distance from the camera. `alpha` below 255 makes it see-through.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_cube(painter: &egui::Painter, to_screen: &dyn Fn(f32, f32) -> Pos2, scene: &Scene, view: (u32, u32), centre: Vec3, rot: Quat, half_px: f32, colour: Color32, alpha: u8, outline: Color32) {
    let (right, up, fwd) = camera_axes(&scene.camera);
    let eye = Vec3::from(scene.camera.position);
    let depth = (centre - eye).dot(fwd);
    if !(depth.is_finite() && depth > 1e-3) || !rot.is_finite() {
        return;
    }
    let half = px_to_world(scene, view.1, depth, half_px);
    // A light a little above and to the left of the viewer.
    let light = (up * 0.7 - right * 0.35 - fwd * 0.6).normalize_or_zero();
    for (normal, corners) in visible_faces(eye, centre, rot, half) {
        let projected = photocraft_render3d::project_points(scene, view.0, view.1, &corners);
        let pts: Vec<Pos2> = projected.iter().filter_map(|p| p.map(|(x, y)| to_screen(x, y))).collect();
        if pts.len() != 4 {
            continue;
        }
        let shade = 0.55 + 0.45 * normal.dot(light).max(0.0);
        let fill = Color32::from_rgba_unmultiplied((colour.r() as f32 * shade) as u8, (colour.g() as f32 * shade) as u8, (colour.b() as f32 * shade) as u8, alpha);
        painter.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.0, outline)));
    }
}

/// The convex hull of some points (Andrew's monotone chain): what a cone looks like flat.
fn hull(mut pts: Vec<Pos2>) -> Vec<Pos2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Pos2, a: Pos2, b: Pos2| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut lower: Vec<Pos2> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Pos2> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// A cone from `base` to `tip` (world), `radius` wide at the base, flattened onto the screen.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_cone(painter: &egui::Painter, to_screen: &dyn Fn(f32, f32) -> Pos2, scene: &Scene, view: (u32, u32), base: Vec3, tip: Vec3, radius: f32, colour: Color32) {
    let axis = (tip - base).normalize_or_zero();
    if axis == Vec3::ZERO || !radius.is_finite() {
        return;
    }
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let mut pts = vec![tip];
    pts.extend((0..14).map(|i| {
        let a = i as f32 / 14.0 * std::f32::consts::TAU;
        base + (u * a.cos() + v * a.sin()) * radius
    }));
    let screen: Vec<Pos2> = photocraft_render3d::project_points(scene, view.0, view.1, &pts).into_iter().flatten().map(|(x, y)| to_screen(x, y)).collect();
    let poly = hull(screen);
    if poly.len() >= 3 {
        painter.add(egui::Shape::convex_polygon(poly, colour, Stroke::new(1.0, Color32::from_black_alpha(170))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_says_what_is_on() {
        let mut n = Node::default();
        assert_eq!(bone_color(&n), RED);
        n.hold_rot = Some([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(bone_color(&n), ORANGE, "orientation only is orange");
        n.hold = Some([0.0; 3]);
        assert_eq!(bone_color(&n), GREEN, "position On is green, whatever the orientation");
        n.hold_rot = None;
        assert_eq!(bone_color(&n), GREEN);
    }

    #[test]
    fn a_cube_shows_one_face_head_on_three_from_a_corner_and_the_far_ones_first() {
        let c = Vec3::ZERO;
        assert_eq!(visible_faces(Vec3::new(0.0, 0.0, 10.0), c, Quat::IDENTITY, 1.0).len(), 1);
        let faces = visible_faces(Vec3::new(5.0, 6.0, 7.0), c, Quat::IDENTITY, 1.0);
        assert_eq!(faces.len(), 3);
        let eye = Vec3::new(5.0, 6.0, 7.0);
        let mid = |f: &(Vec3, [Vec3; 4])| (f.1.iter().sum::<Vec3>() / 4.0).distance(eye);
        assert!(faces.windows(2).all(|w| mid(&w[0]) >= mid(&w[1])), "back to front");
        // Turned, it shows a different face: the cube carries its bone's orientation.
        let turned = visible_faces(Vec3::new(0.0, 0.0, 10.0), c, Quat::from_rotation_y(0.7), 1.0);
        assert_eq!(turned.len(), 2, "a corner edge toward the viewer");
        // Garbage in, nothing out and no panic.
        assert!(visible_faces(Vec3::NAN, c, Quat::IDENTITY, 1.0).is_empty());
    }

    #[test]
    fn a_size_in_pixels_grows_with_the_distance() {
        let s = Scene::default();
        let (near, far) = (px_to_world(&s, 600, 2.0, 7.0), px_to_world(&s, 600, 8.0, 7.0));
        assert!((far / near - 4.0).abs() < 1e-3);
        assert!(px_to_world(&s, 0, 0.0, 7.0).is_finite());
    }

    #[test]
    fn the_hull_of_a_cone_is_a_convex_outline() {
        let pts = vec![Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0), Pos2::new(10.0, 10.0), Pos2::new(0.0, 10.0), Pos2::new(5.0, 5.0), Pos2::new(5.0, 5.0)];
        assert_eq!(hull(pts).len(), 4, "the middle point is inside");
        assert!(hull(vec![Pos2::new(1.0, 1.0)]).len() <= 1);
    }
}
