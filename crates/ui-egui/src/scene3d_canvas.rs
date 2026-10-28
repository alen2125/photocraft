//! The 3D Pose Editor as an on-canvas mode (like Puppet Warp): the scene is rendered over the
//! whole document, the pointer works in document coordinates, and baking turns it into a pixel
//! layer at the same place (`scene3d.renderToLayer`, one undo step). The shell calls these
//! hooks through `distort_ui` (pointer, keys, content, overlay, options bar); the node list,
//! inspector and poses live in the floating panel of `scene3d_ui`.
//!
//! Handles: the transform gizmo (move W, rotate E, scale R) on the selected object or bone, the
//! bone dots of a skeleton (drag to pose), and the camera (Alt-drag orbits, Shift pans, Ctrl
//! dollies; so does a plain drag on empty space).

use egui::{Color32, Stroke, pos2};
use photocraft_scene3d::glam::Vec3;
use photocraft_scene3d::{Delta, Gizmo, GizmoMode, Handle, Node, NodeId, NodeKind, Ray};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::scene3d_ui::{BoneDrag, Scene3dEditor, bone_screen, camera_axes, drag_bone, nearest_bone};

/// Longest edge of the live preview, in pixels.
const PREVIEW_EDGE: f32 = 960.0;
/// Seconds without a camera or pose change before the preview is drawn at full quality.
const SETTLE: f64 = 0.15;
/// A press that moves less than this many preview pixels is a click, not a camera drag.
const CLICK_SLOP: f32 = 3.0;
/// Scene edits kept for undo.
const HISTORY: usize = 64;
/// The gizmo's size as a fraction of its distance from the camera (so it keeps its size on screen).
const GIZMO_SCALE: f32 = 0.16;
/// Screen pixels within which a press grabs a bone dot.
const BONE_GRAB: f32 = 16.0;

/// A gizmo drag in progress. Each step restores the nodes as they were at the press and applies
/// the whole drag to them, so the result depends on where the pointer is, not on the path.
#[derive(Clone, Debug)]
pub(crate) struct GizmoDrag {
    drag: photocraft_scene3d::Drag,
    id: NodeId,
    origin: Vec3,
    base: Vec<Node>,
    base_scale: [f32; 3],
}

/// What the pressed pointer is doing.
#[derive(Clone, Debug)]
pub(crate) enum PointerKind {
    Orbit,
    Pan,
    Dolly,
    Bone(BoneDrag),
    Gizmo(Box<GizmoDrag>),
}

#[derive(Clone, Debug)]
pub(crate) struct PointerState {
    kind: PointerKind,
    start: (f32, f32),
    last: (f32, f32),
    moved: bool,
    /// Pressed on empty space: a release without movement is a click that picks.
    pending_click: bool,
}

/// The preview's logical size in pixels for a document: its shape, at most [`PREVIEW_EDGE`] long.
pub(crate) fn view_size(doc: [u32; 2]) -> (u32, u32) {
    let (w, h) = (doc[0].max(1) as f32, doc[1].max(1) as f32);
    let k = (PREVIEW_EDGE / w.max(h)).min(1.0);
    (((w * k) as u32).max(8), ((h * k) as u32).max(8))
}

/// Ends the mode. The scene stays in the editor, so opening it again finds it as it was.
pub(crate) fn close(ed: &mut Scene3dEditor) {
    ed.physics = None;
    ed.play_base = None;
    ed.open = false;
    ed.owner = None;
    ed.pointer = None;
    ed.preview = None;
}

/// Once per frame while the mode is open: follow the active document, remember edits for undo,
/// settle the preview quality, and keep the background renderer fed. Closes the mode when its
/// document goes away.
pub(crate) fn frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.scene3d.open {
        return;
    }
    let current = app.session.active().map(|s| (s.doc.id, [s.doc.size.width, s.doc.size.height]));
    let ed = &mut app.scene3d;
    let Some((id, size)) = current else {
        close(ed);
        return;
    };
    match ed.owner {
        None => ed.owner = Some(id),
        Some(o) if o != id => {
            close(ed);
            return;
        }
        Some(_) => {}
    }
    if ed.doc_size != size {
        ed.doc_size = size;
        ed.touch();
    }
    if ed.physics.is_some() {
        tick(ed, ctx.input(|i| i.stable_dt));
        ctx.request_repaint();
    } else if ed.pointer.is_none() {
        remember_edit(ed);
    }
    let now = ctx.input(|i| i.time);
    if std::mem::take(&mut ed.pending_change) {
        ed.last_change = now;
    }
    let settled = now - ed.last_change >= SETTLE;
    if !settled {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SETTLE));
    }
    let view = view_size(ed.doc_size);
    // A quarter of the pixels and no smoothing while things move; full quality once they rest.
    let (render, ssaa) = if settled { (view, ed.out_ssaa.clamp(1, 2)) } else { (((view.0 / 2).max(8), (view.1 / 2).max(8)), 1) };
    crate::scene3d_ui::refresh_preview(ctx, ed, render, ssaa);
}

/// Starts the physics: unpinned solids fall and collide with the rest. Remembers the scene as it
/// is so Stop can keep the result as one undo step, or Revert can bring it back.
pub(crate) fn play(ed: &mut Scene3dEditor) -> Result<(), String> {
    if ed.physics.is_some() {
        return Ok(());
    }
    let world = photocraft_physics3d::World::new(&ed.scene);
    if world.is_inert() {
        return Err(tl!("Nothing to move: untick Pinned on an object or a bone, or use a model with hair.").to_string());
    }
    remember_edit(ed);
    ed.play_base = Some(ed.scene.clone());
    ed.play_acc = 0.0;
    ed.physics = Some(world);
    ed.pointer = None;
    Ok(())
}

/// Ends the physics. `keep` leaves the objects where they fell (one undo step); otherwise the
/// scene goes back to how it was when Play was pressed.
pub(crate) fn stop(ed: &mut Scene3dEditor, keep: bool) {
    ed.physics = None;
    let Some(base) = ed.play_base.take() else { return };
    if keep {
        ed.history.push(base);
        if ed.history.len() > HISTORY {
            ed.history.remove(0);
        }
        ed.future.clear();
    } else {
        let camera = ed.scene.camera.clone();
        ed.scene = base;
        ed.scene.camera = camera;
    }
    ed.committed = Some(ed.scene.clone());
    ed.touch();
    ed.history_rev = ed.rev;
    ed.pending_change = true;
}

/// Runs `dt` seconds of physics in fixed steps (at most a few per frame, so a slow frame cannot
/// snowball) and writes the bodies' places into the scene.
pub(crate) fn tick(ed: &mut Scene3dEditor, dt: f32) {
    let Some(mut world) = ed.physics.take() else { return };
    ed.play_acc += if dt.is_finite() { dt.clamp(0.0, 0.1) } else { 0.0 };
    let mut steps = 0;
    while ed.play_acc >= photocraft_physics3d::STEP && steps < 6 {
        // Bodies, limbs and hair, each step written into the scene.
        world.step_scene(&mut ed.scene);
        ed.play_acc -= photocraft_physics3d::STEP;
        steps += 1;
    }
    if steps > 0 {
        ed.touch();
        ed.pending_change = true;
    }
    ed.physics = Some(world);
}

/// Pushes the scene as it was onto the undo stack when something other than the camera changed
/// since the last look. Runs while no button is down, so a drag is one step.
fn remember_edit(ed: &mut Scene3dEditor) {
    if ed.history_rev == ed.rev {
        return;
    }
    ed.history_rev = ed.rev;
    let Some(prev) = ed.committed.as_mut() else {
        ed.committed = Some(ed.scene.clone());
        return;
    };
    prev.camera = ed.scene.camera.clone();
    if *prev != ed.scene {
        let before = std::mem::replace(prev, ed.scene.clone());
        ed.history.push(before);
        if ed.history.len() > HISTORY {
            ed.history.remove(0);
        }
        ed.future.clear();
    }
}

/// Steps back (or forward) through scene edits. The camera stays where it is.
pub(crate) fn step_history(ed: &mut Scene3dEditor, undo: bool) -> bool {
    remember_edit(ed);
    let Some(target) = (if undo { ed.history.pop() } else { ed.future.pop() }) else { return false };
    let current = std::mem::replace(&mut ed.scene, target);
    ed.scene.camera = current.camera.clone();
    if undo { ed.future.push(current) } else { ed.history.push(current) }
    ed.committed = Some(ed.scene.clone());
    if ed.selected.is_some_and(|id| ed.scene.node(id).is_none()) {
        ed.selected = None;
    }
    ed.touch();
    ed.history_rev = ed.rev;
    ed.pending_change = true;
    true
}

/// The gizmo for the selected node, sized to stay the same on screen. Bones have no scale gizmo.
pub(crate) fn gizmo_for(ed: &Scene3dEditor) -> Option<Gizmo> {
    let id = ed.selected?;
    let node = ed.scene.node(id)?;
    if matches!(node.kind, NodeKind::Bone) && ed.gizmo_mode == GizmoMode::Scale {
        return None;
    }
    let world = ed.scene.world_matrix(id)?;
    let origin = world.transform_point3(Vec3::ZERO);
    let axes = if ed.gizmo_local {
        [Vec3::X, Vec3::Y, Vec3::Z].map(|a| world.transform_vector3(a).normalize_or_zero())
    } else {
        [Vec3::X, Vec3::Y, Vec3::Z]
    };
    let size = origin.distance(Vec3::from(ed.scene.camera.position)) * GIZMO_SCALE;
    let g = Gizmo { origin, axes, size };
    g.is_valid().then_some(g)
}

fn ray_at(ed: &Scene3dEditor, view: (u32, u32), p: (f32, f32)) -> Option<Ray> {
    photocraft_render3d::pixel_ray(&ed.scene, view.0, view.1, p.0, p.1).map(|(origin, dir)| Ray { origin, dir })
}

/// Pointer events in document coordinates. A press on the gizmo or a bone dot manipulates it;
/// elsewhere it turns the camera (Alt anywhere); a click selects.
pub(crate) fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    let grab = grab_radius(app);
    let ed = &mut app.scene3d;
    let view = view_size(ed.doc_size);
    let doc = (ed.doc_size[0].max(1) as f32, ed.doc_size[1].max(1) as f32);
    let to_view = |x: f64, y: f64| (x as f32 / doc.0 * view.0 as f32, y as f32 / doc.1 * view.1 as f32);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = to_view(x, y);
            let camera = if mods.shift { PointerKind::Pan } else if mods.command { PointerKind::Dolly } else { PointerKind::Orbit };
            let mut state = PointerState { kind: camera, start: p, last: p, moved: false, pending_click: true };
            // While the physics runs the pointer only moves the camera and selects.
            if !mods.alt && ed.physics.is_none() {
                if let Some(g) = begin_gizmo(ed, view, p) {
                    state.kind = PointerKind::Gizmo(Box::new(g));
                    state.pending_click = false;
                } else if ed.show_bones {
                    let bones = bone_screen(&ed.scene, view);
                    if let Some(id) = nearest_bone(&bones, p.0, p.1, grab) {
                        ed.selected = Some(id);
                        if let Some(b) = ed.scene.bone_positions().iter().find(|b| b.0 == id) {
                            state.kind = PointerKind::Bone(BoneDrag { id, plane_point: b.2 });
                            state.pending_click = false;
                        }
                    }
                }
            }
            ed.pointer = Some(state);
        }
        ToolEvent::Move { x, y, .. } => {
            let p = to_view(x, y);
            let Some(mut s) = ed.pointer.take() else { return };
            let d = (p.0 - s.last.0, p.1 - s.last.1);
            if (p.0 - s.start.0).hypot(p.1 - s.start.1) > CLICK_SLOP {
                s.moved = true;
            }
            let moving = s.moved || !s.pending_click;
            s.last = p;
            if moving {
                match &s.kind {
                    PointerKind::Orbit => {
                        ed.orbit.yaw -= d.0 * 0.4;
                        ed.orbit.pitch = (ed.orbit.pitch + d.1 * 0.4).clamp(-89.0, 89.0);
                        apply_camera(ed);
                    }
                    PointerKind::Pan => {
                        let (right, up, _) = camera_axes(&ed.scene.camera);
                        let k = ed.orbit.distance * 0.0016 * (PREVIEW_EDGE / view.0.max(view.1) as f32).max(1.0);
                        let t = Vec3::from(ed.orbit.target) + (-right * d.0 + up * d.1) * k;
                        ed.orbit.target = t.to_array();
                        apply_camera(ed);
                    }
                    PointerKind::Dolly => {
                        ed.orbit.distance = (ed.orbit.distance * (1.0 + d.1 * 0.005)).clamp(0.2, 500.0);
                        apply_camera(ed);
                    }
                    PointerKind::Bone(drag) => {
                        drag_bone(ed, *drag, view, p, d);
                        ed.touch();
                        ed.pending_change = true;
                    }
                    PointerKind::Gizmo(g) => {
                        if let Some(ray) = ray_at(ed, view, p) {
                            apply_gizmo(ed, g, ray);
                        }
                    }
                }
            }
            ed.pointer = Some(s);
        }
        ToolEvent::Up { .. } => {
            let Some(s) = ed.pointer.take() else { return };
            if s.pending_click && !s.moved {
                let bones = if ed.show_bones { bone_screen(&ed.scene, view) } else { Vec::new() };
                ed.selected = nearest_bone(&bones, s.start.0, s.start.1, grab).or_else(|| photocraft_render3d::pick(&ed.scene, view.0, view.1, s.start.0, s.start.1));
            }
            ed.pending_change = true;
        }
    }
}

/// Starts a gizmo drag if the press at preview pixel `p` lands on a handle of the selection's gizmo.
fn begin_gizmo(ed: &Scene3dEditor, view: (u32, u32), p: (f32, f32)) -> Option<GizmoDrag> {
    let gizmo = gizmo_for(ed)?;
    let ray = ray_at(ed, view, p)?;
    let handle = gizmo.pick(ed.gizmo_mode, ray)?;
    let (_, up, _) = camera_axes(&ed.scene.camera);
    let drag = gizmo.begin(ed.gizmo_mode, handle, ray, up)?;
    let id = ed.selected?;
    let base_scale = ed.scene.node(id)?.transform.scale;
    Some(GizmoDrag { drag, id, origin: gizmo.origin, base: ed.scene.nodes.clone(), base_scale })
}

/// One step of a gizmo drag: the nodes go back to the press, then take the drag's whole effect.
fn apply_gizmo(ed: &mut Scene3dEditor, g: &GizmoDrag, ray: Ray) {
    let Some(delta) = g.drag.update(ray) else { return };
    ed.scene.nodes = g.base.clone();
    let linked = ed.scene.node(g.id).is_some_and(|n| n.link.is_some() && !matches!(n.kind, NodeKind::Bone));
    let result = match delta {
        // A linked node keeps its link: moving it re-aims the link, turning swings it round the
        // target, scaling pulls it closer or pushes it away.
        Delta::Translate(v) if linked => ed.scene.set_link_from_world(g.id, g.origin + v),
        Delta::Rotate(q) if linked => ed.scene.orbit_link(g.id, q),
        Delta::Scale(f) if linked => ed.scene.scale_link_distance(g.id, farthest_from_one(f)),
        Delta::Translate(v) => {
            let target = g.origin + v;
            let bone_below_bone = ed.scene.node(g.id).is_some_and(|n| matches!(n.kind, NodeKind::Bone) && n.parent.and_then(|p| ed.scene.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Bone)));
            if bone_below_bone {
                // A limb bone follows the pointer by turning the bones above it.
                photocraft_scene3d::solve_ik(&mut ed.scene, g.id, target, ed.ik_chain, 12).map(|_| ())
            } else {
                photocraft_scene3d::translate_to(&mut ed.scene, g.id, target)
            }
        }
        Delta::Rotate(q) => photocraft_scene3d::rotate_world(&mut ed.scene, g.id, q),
        Delta::Scale(f) => {
            if let Some(n) = ed.scene.node_mut(g.id) {
                n.transform.scale = [g.base_scale[0] * f.x, g.base_scale[1] * f.y, g.base_scale[2] * f.z];
            }
            Ok(())
        }
    };
    if let Err(e) = result {
        ed.status = e.to_string();
    }
    ed.touch();
    ed.pending_change = true;
}

/// The component of a scale factor that differs most from 1 (an axis handle changes one, the
/// centre handle all three equally).
fn farthest_from_one(f: Vec3) -> f32 {
    [f.x, f.y, f.z].into_iter().max_by(|a, b| a.ln().abs().total_cmp(&b.ln().abs())).unwrap_or(1.0)
}

fn apply_camera(ed: &mut Scene3dEditor) {
    ed.orbit.apply(&mut ed.scene.camera);
    ed.touch();
    ed.pending_change = true;
}

/// How close, in preview pixels, a press must be to a bone dot to grab it: about 16 screen
/// pixels at any canvas zoom.
fn grab_radius(app: &PhotocraftApp) -> f32 {
    let view = view_size(app.scene3d.doc_size);
    let doc_w = app.scene3d.doc_size[0].max(1) as f32;
    (BONE_GRAB / app.point_zoom().max(0.01) * view.0 as f32 / doc_w).clamp(4.0, 60.0)
}

/// Keys while the mode is open: Enter bakes, Esc closes, W/E/R pick move/rotate/scale, Ctrl+Z and
/// Ctrl+Shift+Z (or Ctrl+Y) undo and redo scene edits. True when a key was used.
pub(crate) fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use egui::{Key, Modifiers};
    if !app.scene3d.open {
        return false;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        bake(app, ctx);
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        close(&mut app.scene3d);
        return true;
    }
    let redo = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y));
    if redo {
        step_history(&mut app.scene3d, false);
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
        step_history(&mut app.scene3d, true);
        return true;
    }
    for (key, mode) in [(Key::W, GizmoMode::Translate), (Key::E, GizmoMode::Rotate), (Key::R, GizmoMode::Scale)] {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            app.scene3d.gizmo_mode = mode;
            return true;
        }
    }
    false
}

/// The smoothing (samples per pixel edge) that keeps a `w`×`h` render inside the renderer's
/// sample budget: a big document gets less, down to none.
pub(crate) fn limited_ssaa(w: u32, h: u32, wanted: u32) -> u32 {
    let mut ssaa = wanted.clamp(1, 4);
    while ssaa > 1 && u64::from(w) * u64::from(h) * u64::from(ssaa * ssaa) > photocraft_render3d::MAX_SAMPLES {
        ssaa -= 1;
    }
    ssaa
}

/// Renders the scene at the document's size into a new layer above the active one, at the
/// document's origin, and closes the mode. The picture is what the canvas showed.
pub(crate) fn bake(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let ed = &app.scene3d;
    let [w, h] = ed.doc_size;
    let ssaa = limited_ssaa(w, h, ed.out_ssaa);
    let params = json!({
        "scene": serde_json::to_value(&ed.scene).unwrap_or_default(),
        "width": w, "height": h, "ssaa": ssaa, "transparent": ed.out_transparent,
    });
    match crate::menus::invoke(app, ctx, "scene3d.renderToLayer", params) {
        Ok(_) => close(&mut app.scene3d),
        Err(e) => app.scene3d.status = e,
    }
}

/// The scene over the canvas: one textured quad on the document's rectangle. Image content, so
/// the canvas draws it under the grid and guides.
pub(crate) fn draw_content(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let ed = &app.scene3d;
    let Some(p) = &ed.preview else { return };
    let (w, h) = (ed.doc_size[0] as f32, ed.doc_size[1] as f32);
    let mut mesh = egui::Mesh::with_texture(p.texture.id());
    for (x, y, u, v) in [(0.0, 0.0, 0.0, 0.0), (w, 0.0, 1.0, 0.0), (w, h, 1.0, 1.0), (0.0, h, 0.0, 1.0)] {
        mesh.vertices.push(egui::epaint::Vertex { pos: xf.to_screen(x, y), uv: pos2(u, v), color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

const AXIS_COLORS: [Color32; 3] = [Color32::from_rgb(230, 70, 70), Color32::from_rgb(90, 200, 90), Color32::from_rgb(80, 130, 240)];
const HOT: Color32 = Color32::from_rgb(255, 220, 60);

/// The skeleton, the gizmo and the selection marker over the picture.
pub(crate) fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let ed = &app.scene3d;
    let view = view_size(ed.doc_size);
    let (dw, dh) = (ed.doc_size[0] as f32, ed.doc_size[1] as f32);
    let to_screen = |x: f32, y: f32| xf.to_screen(x / view.0 as f32 * dw, y / view.1 as f32 * dh);
    let bones = if ed.show_bones { bone_screen(&ed.scene, view) } else { Vec::new() };
    let screen_of = |id| bones.iter().find(|b| b.0 == id).map(|b| to_screen(b.2.0, b.2.1));
    for (id, parent, (x, y)) in &bones {
        let c = to_screen(*x, *y);
        if let Some(pp) = parent.and_then(screen_of) {
            painter.line_segment([pp, c], Stroke::new(2.0, Color32::from_rgba_unmultiplied(120, 200, 255, 200)));
        }
        let (r, col) = if ed.selected == Some(*id) { (8.0, Color32::from_rgb(255, 170, 40)) } else { (5.5, Color32::from_rgb(120, 200, 255)) };
        painter.circle_filled(c, r, col);
        painter.circle_stroke(c, r, Stroke::new(1.0, Color32::from_black_alpha(160)));
    }
    draw_links(ed, view, painter, &to_screen);
    if let Some(gizmo) = gizmo_for(ed) {
        draw_gizmo(ed, &gizmo, view, painter, xf, &to_screen);
    } else if let Some(id) = ed.selected
        && !bones.iter().any(|b| b.0 == id)
        && let Some((x, y)) = photocraft_render3d::project(&ed.scene, view.0, view.1, id)
    {
        let c = to_screen(x, y);
        painter.circle_stroke(c, 9.0, Stroke::new(2.5, Color32::from_rgb(255, 170, 40)));
        painter.circle_filled(c, 3.0, Color32::from_rgb(255, 170, 40));
    }
}

/// A dashed line from each linked node to its target, with a diamond on the target.
fn draw_links(ed: &Scene3dEditor, view: (u32, u32), painter: &egui::Painter, to_screen: &dyn Fn(f32, f32) -> egui::Pos2) {
    let world = ed.scene.world_matrices();
    let at = |id: NodeId| world.get(&id).map(|m| m.transform_point3(Vec3::ZERO));
    let pairs: Vec<(NodeId, Vec3, Vec3)> = ed.scene.nodes.iter().filter_map(|n| Some((n.id, at(n.id)?, at(n.link.as_ref()?.target)?))).collect();
    if pairs.is_empty() {
        return;
    }
    let pts: Vec<Vec3> = pairs.iter().flat_map(|p| [p.1, p.2]).collect();
    let screen = photocraft_render3d::project_points(&ed.scene, view.0, view.1, &pts);
    for (i, (id, _, _)) in pairs.iter().enumerate() {
        let (Some(Some((ax, ay))), Some(Some((bx, by)))) = (screen.get(i * 2), screen.get(i * 2 + 1)) else { continue };
        let (a, b) = (to_screen(*ax, *ay), to_screen(*bx, *by));
        let col = if ed.selected == Some(*id) { Color32::from_rgb(255, 210, 90) } else { Color32::from_rgba_unmultiplied(255, 210, 90, 140) };
        painter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.5, col), 6.0, 4.0));
        let r = 6.0;
        painter.add(egui::Shape::convex_polygon(vec![b + egui::vec2(0.0, -r), b + egui::vec2(r, 0.0), b + egui::vec2(0.0, r), b + egui::vec2(-r, 0.0)], col, Stroke::new(1.0, Color32::BLACK)));
    }
}

fn draw_gizmo(ed: &Scene3dEditor, g: &Gizmo, view: (u32, u32), painter: &egui::Painter, xf: &ViewXform, to_screen: &dyn Fn(f32, f32) -> egui::Pos2) {
    // The handle under the pointer (or being dragged) is lit.
    let active = match ed.pointer.as_ref().map(|p| &p.kind) {
        Some(PointerKind::Gizmo(d)) => Some(d.drag.handle),
        _ => painter.ctx().pointer_hover_pos().filter(|_| ed.pointer.is_none()).and_then(|p| {
            let d = xf.to_doc(p);
            let v = ((d[0] as f32 / ed.doc_size[0].max(1) as f32) * view.0 as f32, (d[1] as f32 / ed.doc_size[1].max(1) as f32) * view.1 as f32);
            ray_at(ed, view, v).and_then(|r| g.pick(ed.gizmo_mode, r))
        }),
    };
    let colour = |axis: usize| if active == Some(Handle::Axis(axis)) { HOT } else { AXIS_COLORS[axis] };
    let project = |pts: &[Vec3]| photocraft_render3d::project_points(&ed.scene, view.0, view.1, pts);
    let screen = |p: Option<(f32, f32)>| p.map(|(x, y)| to_screen(x, y));
    match ed.gizmo_mode {
        GizmoMode::Rotate => {
            for axis in 0..3 {
                let u = g.axes[axis].any_orthonormal_vector();
                let v = g.axes[axis].cross(u);
                let ring: Vec<Vec3> = (0..=64)
                    .map(|i| {
                        let a = i as f32 / 64.0 * std::f32::consts::TAU;
                        g.origin + (u * a.cos() + v * a.sin()) * g.size
                    })
                    .collect();
                let pts: Vec<Option<egui::Pos2>> = project(&ring).into_iter().map(screen).collect();
                for w in pts.windows(2) {
                    if let [Some(a), Some(b)] = w {
                        painter.line_segment([*a, *b], Stroke::new(4.0, colour(axis)));
                    }
                }
            }
        }
        GizmoMode::Translate | GizmoMode::Scale => {
            let mut pts = vec![g.origin];
            pts.extend((0..3).map(|a| g.origin + g.axes[a] * g.size));
            let s: Vec<Option<egui::Pos2>> = project(&pts).into_iter().map(screen).collect();
            for axis in 0..3 {
                if let (Some(Some(o)), Some(Some(t))) = (s.first(), s.get(axis + 1)) {
                    painter.line_segment([*o, *t], Stroke::new(4.5, colour(axis)));
                    if ed.gizmo_mode == GizmoMode::Scale {
                        painter.rect_filled(egui::Rect::from_center_size(*t, egui::vec2(14.0, 14.0)), 0.0, colour(axis));
                    } else {
                        painter.circle_filled(*t, 8.0, colour(axis));
                    }
                }
            }
            if ed.gizmo_mode == GizmoMode::Scale
                && let Some(Some(o)) = s.first()
            {
                let col = if active == Some(Handle::Uniform) { HOT } else { Color32::from_gray(230) };
                painter.rect_filled(egui::Rect::from_center_size(*o, egui::vec2(15.0, 15.0)), 0.0, col);
            }
        }
    }
}

/// The options bar while the mode is open: gizmo tool and space, bake, cancel, quality.
pub(crate) fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    ui.label(tl!("3D Pose Editor"));
    for (mode, label) in [(GizmoMode::Translate, tl!("Move")), (GizmoMode::Rotate, tl!("Rotate")), (GizmoMode::Scale, tl!("Scale"))] {
        ui.selectable_value(&mut app.scene3d.gizmo_mode, mode, label);
    }
    ui.checkbox(&mut app.scene3d.gizmo_local, tl!("Local"));
    let mut changed = false;
    changed |= ui.checkbox(&mut app.scene3d.out_transparent, tl!("Transparent sky")).changed();
    ui.add(egui::Slider::new(&mut app.scene3d.out_ssaa, 1..=4).text(tl!("Smoothing")));
    let mut fov = app.scene3d.scene.camera.fov;
    if ui.add(egui::DragValue::new(&mut fov).range(10.0..=120.0).speed(0.5).prefix(format!("{} ", tl!("Field of view")))).changed() {
        app.scene3d.scene.camera.fov = fov;
        changed = true;
    }
    if changed {
        app.scene3d.touch();
    }
    if crate::widgets::primary_button(ui, tl!("Bake to Layer"), 130.0).clicked() {
        bake(app, &ctx);
    }
    if ui.button(tl!("Cancel")).clicked() {
        close(&mut app.scene3d);
    }
    if !app.scene3d.status.is_empty() {
        ui.colored_label(Color32::from_rgb(230, 110, 90), &app.scene3d.status);
    }
}

#[cfg(test)]
#[path = "scene3d_canvas_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "scene3d_canvas_gizmo_tests.rs"]
mod gizmo_tests;

#[cfg(test)]
#[path = "scene3d_canvas_link_tests.rs"]
mod link_tests;

#[cfg(test)]
#[path = "scene3d_canvas_physics_tests.rs"]
mod physics_tests;
