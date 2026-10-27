//! Window › 3D Pose Editor: build a scene of primitives, lights and imported models (glTF, GLB,
//! VRM), pose a model's skeleton by turning bones or dragging them (inverse kinematics), frame
//! it with an orbit camera, and render it into a pixel layer (`scene3d.renderToLayer`). The scene
//! saves to and opens from a `.pcscene` file so a pose can be fixed later. The viewport preview
//! uses the software renderer (`photocraft-render3d`); nothing here touches the document until
//! Render to Layer.

use std::sync::Arc;

use egui::{Color32, Sense, Stroke, Vec2, pos2, vec2};
use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{Camera, Light, LightKind, Material, Node, NodeId, NodeKind, Primitive, RotationLimits, Scene, Transform};
use serde_json::json;

use crate::PhotocraftApp;

const OPEN_ID: &str = "ui.scene3d.open";
/// Longest edge of the live preview, in pixels.
const PREVIEW_EDGE: f32 = 960.0;

/// Whether `id` is a command this module carries out.
pub fn handles(id: &str) -> bool {
    id == OPEN_ID
}

pub fn invoke(app: &mut PhotocraftApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    if !handles(id) {
        return None;
    }
    app.scene3d.open = true;
    Some(Ok(json!({"open": true})))
}

/// Orbit camera state: where it looks and from which angles.
#[derive(Clone, Debug, PartialEq)]
pub struct Orbit {
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl Orbit {
    pub fn from_camera(c: &Camera) -> Self {
        let d = [c.position[0] - c.target[0], c.position[1] - c.target[1], c.position[2] - c.target[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if !dist.is_finite() || dist < 1e-4 {
            return Self { target: [0.0; 3], yaw: 20.0, pitch: 20.0, distance: 5.0 };
        }
        Self { target: c.target, yaw: d[0].atan2(d[2]).to_degrees(), pitch: (d[1] / dist).clamp(-1.0, 1.0).asin().to_degrees(), distance: dist }
    }

    pub fn apply(&self, c: &mut Camera) {
        c.target = self.target;
        c.position = Camera::orbit(self.target, self.yaw, self.pitch, self.distance);
    }
}

pub struct Scene3dEditor {
    pub open: bool,
    pub scene: Scene,
    selected: Option<NodeId>,
    orbit: Orbit,
    preview: Option<Preview>,
    /// Bumped on every change to the scene or camera; the preview re-renders when it moves.
    rev: u64,
    status: String,
    file_name: Option<String>,
    out_width: u32,
    out_height: u32,
    out_ssaa: u32,
    out_transparent: bool,
    show_bones: bool,
    pose_mode: PoseMode,
    /// How many bones above the dragged one turn in IK mode.
    ik_chain: usize,
    drag: Option<BoneDrag>,
    pose_name: String,
    /// Set by the side panel; carried out after the window is drawn (it opens a file dialog).
    want_import: bool,
}

/// What dragging a bone in the viewport does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoseMode {
    /// Turn the bone about the camera's axes (forward kinematics).
    Rotate,
    /// Pull the bone to the pointer by turning the bones above it.
    Ik,
}

/// A bone drag in progress: the plane facing the camera through the bone where it was grabbed.
#[derive(Clone, Copy, Debug)]
struct BoneDrag {
    id: NodeId,
    plane_point: Vec3,
}

struct Preview {
    texture: egui::TextureHandle,
    key: (u64, u32, u32, u32),
}

impl Default for Scene3dEditor {
    fn default() -> Self {
        let mut scene = Scene::new();
        scene.add_primitive(Primitive::Plane { size: [10.0, 10.0] }, Transform::default()).ok();
        let orbit = Orbit::from_camera(&scene.camera);
        Self {
            open: false,
            scene,
            selected: None,
            orbit,
            preview: None,
            rev: 1,
            status: String::new(),
            file_name: None,
            out_width: 1024,
            out_height: 768,
            out_ssaa: 2,
            out_transparent: true,
            show_bones: true,
            pose_mode: PoseMode::Ik,
            ik_chain: 2,
            drag: None,
            pose_name: "Pose 1".into(),
            want_import: false,
        }
    }
}

impl Scene3dEditor {
    /// Replaces the scene (a loaded file), keeping the camera it was saved with.
    pub fn load(&mut self, scene: Scene, file_name: Option<String>) {
        self.orbit = Orbit::from_camera(&scene.camera);
        self.scene = scene;
        self.selected = None;
        self.file_name = file_name;
        self.rev += 1;
    }

    fn touch(&mut self) {
        self.rev += 1;
    }

    /// Adds a model from file bytes (`name` is the path on the desktop) and frames its skeleton.
    fn import_model(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let model = photocraft_scene3d::model3d::load(name, bytes).map_err(|e| format!("{name}: {e}"))?;
        // A path lets the scene file find the model again; the browser gives a bare name.
        let path = std::path::Path::new(name).is_absolute().then(|| name.to_string());
        let root = self.scene.add_model(Arc::new(model), path).map_err(|e| e.to_string())?;
        self.selected = Some(root);
        self.frame_bones();
        self.touch();
        Ok(())
    }

    /// Points the orbit camera at the skeleton and backs off until it fits.
    fn frame_bones(&mut self) {
        let pts: Vec<Vec3> = self.scene.bone_positions().iter().map(|b| b.2).collect();
        let Some(first) = pts.first().copied() else { return };
        let (lo, hi) = pts.iter().fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let size = (hi - lo).max_element().max(0.5);
        self.orbit.target = ((lo + hi) * 0.5).to_array();
        self.orbit.distance = (size * 1.9).clamp(1.0, 200.0);
        self.orbit.apply(&mut self.scene.camera);
    }

    fn add(&mut self, node: Node) {
        match self.scene.add(node) {
            Ok(id) => {
                self.selected = Some(id);
                self.touch();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn add_primitive(&mut self, p: Primitive) {
        let lift = if matches!(p, Primitive::Plane { .. }) { 0.0 } else { 0.5 };
        self.add(Node { transform: Transform::at(0.0, lift, 0.0), kind: NodeKind::Primitive { primitive: p, material: Material::default() }, ..Node::default() });
    }

    fn add_light(&mut self, kind: LightKind) {
        let rotation = if matches!(kind, LightKind::Point) { [0.0; 3] } else { [-60.0, 20.0, 0.0] };
        let name = match kind {
            LightKind::Directional => "Sun",
            LightKind::Point => "Point Light",
            LightKind::Spot { .. } => "Spot Light",
        };
        self.add(Node {
            name: name.into(),
            transform: Transform { translation: [0.0, 3.0, 0.0], rotation, scale: [1.0; 3] },
            kind: NodeKind::Light(Light { kind, ..Light::default() }),
            ..Node::default()
        });
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            if let Err(e) = self.scene.remove(id) {
                self.status = e.to_string();
            }
            self.touch();
        }
    }

    fn duplicate_selected(&mut self) {
        if let Some(n) = self.selected.and_then(|id| self.scene.node(id)).cloned() {
            let mut copy = n;
            copy.name = format!("{} copy", copy.name);
            copy.transform.translation[0] += 0.5;
            self.add(copy);
        }
    }
}

/// Loads the models a freshly read scene names, from their saved paths. Returns what failed.
#[cfg(not(target_arch = "wasm32"))]
fn load_missing_assets(scene: &mut Scene) -> Vec<String> {
    let missing: Vec<(u32, String, Option<String>)> = scene.missing_assets().iter().map(|a| (a.id, a.name.clone(), a.path.clone())).collect();
    let mut errors = Vec::new();
    for (id, name, path) in missing {
        let Some(path) = path else {
            errors.push(format!("{name}: the scene has no file path for this model"));
            continue;
        };
        let loaded = photocraft_format::read_file(std::path::Path::new(&path)).map_err(|e| e.to_string()).and_then(|b| photocraft_scene3d::model3d::load(&name, &b).map_err(|e| e.to_string()));
        match loaded {
            Ok(m) => {
                let _ = scene.attach_model(id, Arc::new(m));
            }
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }
    errors
}

#[cfg(target_arch = "wasm32")]
fn load_missing_assets(scene: &mut Scene) -> Vec<String> {
    scene.missing_assets().iter().map(|a| format!("{}: models are not loaded from a path in the browser; import it again", a.name)).collect()
}

/// The picture shown in the viewport for `size` pixels; re-rendered only when something changed.
fn refresh_preview(ctx: &egui::Context, ed: &mut Scene3dEditor, size: (u32, u32), ssaa: u32) {
    let key = (ed.rev, size.0, size.1, ssaa);
    if ed.preview.as_ref().is_some_and(|p| p.key == key) {
        return;
    }
    let opts = photocraft_render3d::RenderOptions { width: size.0, height: size.1, ssaa, transparent: false };
    match photocraft_render3d::render(&ed.scene, &opts) {
        Ok(img) => {
            let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.rgba);
            match ed.preview.as_mut() {
                Some(p) => {
                    p.texture.set(color, egui::TextureOptions::LINEAR);
                    p.key = key;
                }
                None => ed.preview = Some(Preview { texture: ctx.load_texture("scene3d-preview", color, egui::TextureOptions::LINEAR), key }),
            }
        }
        Err(e) => ed.status = e.to_string(),
    }
}

fn depth_of(scene: &Scene, id: NodeId) -> usize {
    let mut d = 0;
    let mut cur = scene.node(id).and_then(|n| n.parent);
    while let Some(p) = cur {
        d += 1;
        if d >= photocraft_scene3d::MAX_DEPTH {
            break;
        }
        cur = scene.node(p).and_then(|n| n.parent);
    }
    d
}

/// Nodes in tree order (children right after their parent), bounded by the node count.
fn tree_order(scene: &Scene) -> Vec<NodeId> {
    let mut out = Vec::with_capacity(scene.nodes.len());
    fn walk(scene: &Scene, parent: Option<NodeId>, out: &mut Vec<NodeId>, depth: usize) {
        if depth > photocraft_scene3d::MAX_DEPTH {
            return;
        }
        for n in scene.nodes.iter().filter(|n| n.parent == parent) {
            if out.len() >= scene.nodes.len() {
                return;
            }
            out.push(n.id);
            walk(scene, Some(n.id), out, depth + 1);
        }
    }
    walk(scene, None, &mut out, 0);
    out
}

fn vec3_row(ui: &mut egui::Ui, label: &str, v: &mut [f32; 3], speed: f64) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized([62.0, 18.0], egui::Label::new(label));
        for c in v.iter_mut() {
            changed |= ui.add(egui::DragValue::new(c).speed(speed).max_decimals(3)).changed();
        }
    });
    changed
}

fn node_inspector(ui: &mut egui::Ui, ed: &mut Scene3dEditor, id: NodeId) {
    let mut changed = false;
    let others: Vec<(NodeId, String)> = ed.scene.nodes.iter().filter(|n| n.id != id).map(|n| (n.id, n.name.clone())).collect();
    let mut reparent: Option<Option<NodeId>> = None;
    let Some(node) = ed.scene.node_mut(id) else { return };
    changed |= ui.text_edit_singleline(&mut node.name).changed();
    changed |= ui.checkbox(&mut node.visible, tl!("Visible")).changed();
    let parent_name = node.parent.and_then(|p| others.iter().find(|o| o.0 == p)).map_or_else(|| tl!("(none)").to_string(), |o| o.1.clone());
    egui::ComboBox::from_label(tl!("Parent")).selected_text(parent_name).show_ui(ui, |ui| {
        if ui.selectable_label(node.parent.is_none(), tl!("(none)")).clicked() {
            reparent = Some(None);
        }
        for (oid, name) in &others {
            if ui.selectable_label(node.parent == Some(*oid), name).clicked() {
                reparent = Some(Some(*oid));
            }
        }
    });
    ui.separator();
    changed |= vec3_row(ui, tl!("Position"), &mut node.transform.translation, 0.02);
    changed |= vec3_row(ui, tl!("Rotation"), &mut node.transform.rotation, 0.5);
    changed |= vec3_row(ui, tl!("Scale"), &mut node.transform.scale, 0.01);
    ui.separator();
    match &mut node.kind {
        NodeKind::Empty => {}
        NodeKind::Primitive { primitive, material } => {
            changed |= primitive_params(ui, primitive);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(tl!("Color"));
                changed |= ui.color_edit_button_rgb(&mut material.color).changed();
                ui.label(tl!("Glow"));
                changed |= ui.color_edit_button_rgb(&mut material.emissive).changed();
            });
            changed |= ui.add(egui::Slider::new(&mut material.specular, 0.0..=1.0).text(tl!("Specular"))).changed();
            changed |= ui.add(egui::Slider::new(&mut material.shininess, 1.0..=256.0).logarithmic(true).text(tl!("Shininess"))).changed();
            changed |= ui.checkbox(&mut material.unlit, tl!("Unlit")).changed();
        }
        NodeKind::Bone => {
            let mut limited = node.limits.is_some();
            if ui.checkbox(&mut limited, tl!("Limit rotation")).changed() {
                node.limits = limited.then(RotationLimits::default);
                changed = true;
            }
            if let Some(l) = &mut node.limits {
                changed |= vec3_row(ui, tl!("Min"), &mut l.min, 1.0);
                changed |= vec3_row(ui, tl!("Max"), &mut l.max, 1.0);
            }
            if let Some(rest) = node.rest.clone()
                && ui.button(tl!("Reset bone")).clicked()
            {
                node.transform = rest;
                changed = true;
            }
        }
        NodeKind::Mesh(m) => {
            ui.weak(format!("{} · {}", tl!("Model mesh"), m.joints.len()));
        }
        NodeKind::Light(l) => {
            ui.horizontal(|ui| {
                ui.label(tl!("Color"));
                changed |= ui.color_edit_button_rgb(&mut l.color).changed();
            });
            changed |= ui.add(egui::Slider::new(&mut l.intensity, 0.0..=10.0).text(tl!("Intensity"))).changed();
            if !matches!(l.kind, LightKind::Directional) {
                changed |= ui.add(egui::Slider::new(&mut l.range, 0.5..=100.0).logarithmic(true).text(tl!("Range"))).changed();
            }
            if let LightKind::Spot { angle } = &mut l.kind {
                changed |= ui.add(egui::Slider::new(angle, 5.0..=170.0).text(tl!("Cone angle"))).changed();
            }
        }
    }
    if let Some(p) = reparent {
        match ed.scene.set_parent(id, p) {
            Ok(()) => changed = true,
            Err(e) => ed.status = e.to_string(),
        }
    }
    if changed {
        ed.touch();
    }
}

fn primitive_params(ui: &mut egui::Ui, p: &mut Primitive) -> bool {
    let mut changed = false;
    let drag = |ui: &mut egui::Ui, label: &str, v: &mut f32| {
        ui.horizontal(|ui| {
            ui.add_sized([62.0, 18.0], egui::Label::new(label));
            ui.add(egui::DragValue::new(v).speed(0.01).range(0.001..=1000.0).max_decimals(3)).changed()
        })
        .inner
    };
    match p {
        Primitive::Cube { size } => {
            for (c, name) in size.iter_mut().zip([tl!("Width"), tl!("Height"), tl!("Depth")]) {
                changed |= drag(ui, name, c);
            }
        }
        Primitive::Sphere { radius, segments } => {
            changed |= drag(ui, tl!("Radius"), radius);
            changed |= ui.add(egui::Slider::new(segments, 4..=128).text(tl!("Segments"))).changed();
        }
        Primitive::Cylinder { radius, height, segments } | Primitive::Cone { radius, height, segments } => {
            changed |= drag(ui, tl!("Radius"), radius);
            changed |= drag(ui, tl!("Height"), height);
            changed |= ui.add(egui::Slider::new(segments, 3..=128).text(tl!("Segments"))).changed();
        }
        Primitive::Plane { size } => {
            for (c, name) in size.iter_mut().zip([tl!("Width"), tl!("Depth")]) {
                changed |= drag(ui, name, c);
            }
        }
    }
    changed
}

fn node_list(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    let mut clicked = None;
    for id in tree_order(&ed.scene) {
        let Some(n) = ed.scene.node(id) else { continue };
        let icon = match n.kind {
            NodeKind::Empty => "◇",
            NodeKind::Primitive { .. } => "■",
            NodeKind::Light(_) => "☀",
            NodeKind::Bone => "◦",
            NodeKind::Mesh(_) => "▲",
        };
        let hidden = if n.visible { "" } else { " ·" };
        let label = format!("{}{icon}  {}{hidden}", "    ".repeat(depth_of(&ed.scene, id)), n.name);
        if ui.selectable_label(ed.selected == Some(id), label).clicked() {
            clicked = Some(id);
        }
    }
    if clicked.is_some() {
        ed.selected = clicked;
    }
}

/// Every bone's parent and its position in preview pixels (bones out of view are left out).
fn bone_screen(scene: &Scene, size: (u32, u32)) -> Vec<(NodeId, Option<NodeId>, (f32, f32))> {
    let list = scene.bone_positions();
    let pts: Vec<Vec3> = list.iter().map(|b| b.2).collect();
    let screen = photocraft_render3d::project_points(scene, size.0, size.1, &pts);
    list.iter().zip(screen).filter_map(|(b, s)| s.map(|s| (b.0, b.1, s))).collect()
}

/// The bone dot closest to `(x, y)` within `radius` preview pixels.
fn nearest_bone(bones: &[(NodeId, Option<NodeId>, (f32, f32))], x: f32, y: f32, radius: f32) -> Option<NodeId> {
    bones
        .iter()
        .map(|b| (b.0, (b.2.0 - x).hypot(b.2.1 - y)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The camera's right and up axes and its view direction in the world.
fn camera_axes(c: &Camera) -> (Vec3, Vec3, Vec3) {
    let f = (Vec3::from(c.target) - Vec3::from(c.position)).normalize_or_zero();
    let f = if f == Vec3::ZERO { Vec3::NEG_Z } else { f };
    let right = f.cross(Vec3::Y).normalize_or_zero();
    let right = if right == Vec3::ZERO { Vec3::X } else { right };
    (right, right.cross(f), f)
}

/// Applies one pointer step of a bone drag: pull it to the pointer (IK, or sliding the top
/// bone) or turn it about the camera's axes.
fn drag_bone(ed: &mut Scene3dEditor, drag: BoneDrag, size: (u32, u32), pointer: (f32, f32), delta: (f32, f32)) {
    let (right, up, forward) = camera_axes(&ed.scene.camera);
    let result = match ed.pose_mode {
        PoseMode::Rotate => {
            let q = Quat::from_axis_angle(up, delta.0 * 0.01) * Quat::from_axis_angle(right, delta.1 * 0.01);
            photocraft_scene3d::rotate_world(&mut ed.scene, drag.id, q)
        }
        PoseMode::Ik => {
            let Some((origin, dir)) = photocraft_render3d::pixel_ray(&ed.scene, size.0, size.1, pointer.0, pointer.1) else { return };
            let denom = dir.dot(forward);
            if denom.abs() < 1e-4 {
                return;
            }
            let t = (drag.plane_point - origin).dot(forward) / denom;
            if t <= 0.0 {
                return;
            }
            let target = origin + dir * t;
            let has_bone_parent = ed.scene.node(drag.id).and_then(|n| n.parent).and_then(|p| ed.scene.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Bone));
            if has_bone_parent {
                photocraft_scene3d::solve_ik(&mut ed.scene, drag.id, target, ed.ik_chain, 12).map(|_| ())
            } else {
                // The top bone has nothing above it to turn: it slides.
                photocraft_scene3d::translate_to(&mut ed.scene, drag.id, target)
            }
        }
    };
    if let Err(e) = result {
        ed.status = e.to_string();
    }
}

fn viewport(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    let avail = ui.available_size_before_wrap().max(vec2(160.0, 120.0));
    let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let scale = (PREVIEW_EDGE / (rect.width() * ppp).max(rect.height() * ppp)).min(1.0);
    let size = (((rect.width() * ppp * scale) as u32).max(8), ((rect.height() * ppp * scale) as u32).max(8));

    let to_px = |p: egui::Pos2| ((p.x - rect.min.x) / rect.width() * size.0 as f32, (p.y - rect.min.y) / rect.height() * size.1 as f32);
    // Preview pixels per point, so a 12-point grab radius is the same on any scale.
    let grab = 12.0 * size.0 as f32 / rect.width().max(1.0);
    let bones = if ed.show_bones { bone_screen(&ed.scene, size) } else { Vec::new() };

    let mut moved = false;
    let (dx, dy) = (resp.drag_delta().x, resp.drag_delta().y);
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(p) = resp.interact_pointer_pos()
    {
        // Grabbing a bone dot poses it; anywhere else the drag turns the camera.
        let (px, py) = to_px(p);
        ed.drag = nearest_bone(&bones, px, py, grab).and_then(|id| {
            ed.selected = Some(id);
            ed.scene.bone_positions().iter().find(|b| b.0 == id).map(|b| BoneDrag { id, plane_point: b.2 })
        });
    }
    if resp.drag_stopped() {
        ed.drag = None;
    }
    if resp.dragged_by(egui::PointerButton::Primary)
        && let Some(drag) = ed.drag
    {
        if let Some(p) = resp.interact_pointer_pos() {
            let (px, py) = to_px(p);
            drag_bone(ed, drag, size, (px, py), (dx, dy));
            ed.touch();
        }
    } else if resp.dragged_by(egui::PointerButton::Primary) && (dx != 0.0 || dy != 0.0) {
        ed.orbit.yaw -= dx * 0.4;
        ed.orbit.pitch = (ed.orbit.pitch + dy * 0.4).clamp(-89.0, 89.0);
        moved = true;
    }
    if (resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle)) && (dx != 0.0 || dy != 0.0) {
        // Pan in the camera's plane, scaled so the scene follows the pointer.
        let k = ed.orbit.distance * 0.0016;
        let yaw = ed.orbit.yaw.to_radians();
        let right = [yaw.cos(), 0.0, -yaw.sin()];
        for (t, r) in ed.orbit.target.iter_mut().zip(right) {
            *t -= r * dx * k;
        }
        ed.orbit.target[1] += dy * k;
        moved = true;
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            ed.orbit.distance = (ed.orbit.distance * (1.0 - scroll * 0.0015)).clamp(0.2, 500.0);
            moved = true;
        }
    }
    if moved {
        ed.orbit.apply(&mut ed.scene.camera);
        ed.touch();
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let (px, py) = to_px(p);
        ed.selected = nearest_bone(&bones, px, py, grab).or_else(|| photocraft_render3d::pick(&ed.scene, size.0, size.1, px, py));
    }

    // Fast while the camera moves; anti-aliased once it rests.
    let ssaa = if resp.dragged() { 1 } else { 2 };
    refresh_preview(ui.ctx(), ed, size, ssaa);
    if let Some(p) = &ed.preview {
        ui.painter().image(p.texture.id(), rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        ui.painter().rect_filled(rect, 0.0, Color32::from_gray(40));
    }
    let to_screen = |x: f32, y: f32| pos2(rect.min.x + x / size.0 as f32 * rect.width(), rect.min.y + y / size.1 as f32 * rect.height());
    let screen_of = |id: NodeId| bones.iter().find(|b| b.0 == id).map(|b| to_screen(b.2.0, b.2.1));
    for (id, parent, (x, y)) in &bones {
        let c = to_screen(*x, *y);
        if let Some(pp) = parent.and_then(screen_of) {
            ui.painter().line_segment([pp, c], Stroke::new(1.5, Color32::from_rgba_unmultiplied(120, 200, 255, 200)));
        }
        let selected = ed.selected == Some(*id);
        let (r, col) = if selected { (5.0, Color32::from_rgb(255, 170, 40)) } else { (3.0, Color32::from_rgb(120, 200, 255)) };
        ui.painter().circle_filled(c, r, col);
    }
    if let Some(id) = ed.selected
        && !bones.iter().any(|b| b.0 == id)
        && let Some((x, y)) = photocraft_render3d::project(&ed.scene, size.0, size.1, id)
    {
        let c = pos2(rect.min.x + x / size.0 as f32 * rect.width(), rect.min.y + y / size.1 as f32 * rect.height());
        if rect.contains(c) {
            ui.painter().circle_stroke(c, 7.0, Stroke::new(2.0, Color32::from_rgb(255, 170, 40)));
            ui.painter().circle_filled(c, 2.0, Color32::from_rgb(255, 170, 40));
        }
    }
}

fn side_panels(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    ui.label(egui::RichText::new(tl!("Add")).strong());
    ui.horizontal_wrapped(|ui| {
        for p in [Primitive::cube(), Primitive::sphere(), Primitive::cylinder(), Primitive::cone(), Primitive::plane()] {
            if ui.button(tl!(p.label())).clicked() {
                ed.add_primitive(p);
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button(tl!("Sun")).clicked() {
            ed.add_light(LightKind::Directional);
        }
        if ui.button(tl!("Point light")).clicked() {
            ed.add_light(LightKind::Point);
        }
        if ui.button(tl!("Spot light")).clicked() {
            ed.add_light(LightKind::Spot { angle: 45.0 });
        }
        if ui.button(tl!("Empty")).clicked() {
            ed.add(Node::default());
        }
        if ui.button(tl!("Import Model…")).clicked() {
            ed.want_import = true;
        }
    });
    ui.horizontal(|ui| {
        let has = ed.selected.is_some();
        if ui.add_enabled(has, egui::Button::new(tl!("Duplicate"))).clicked() {
            ed.duplicate_selected();
        }
        if ui.add_enabled(has, egui::Button::new(tl!("Delete"))).clicked() {
            ed.delete_selected();
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().id_salt("scene3d-nodes").max_height(200.0).show(ui, |ui| node_list(ui, ed));
    ui.separator();
    egui::ScrollArea::vertical().id_salt("scene3d-inspector").show(ui, |ui| {
        if ed.scene.nodes.iter().any(|n| matches!(n.kind, NodeKind::Bone)) {
            egui::CollapsingHeader::new(tl!("Pose")).default_open(true).show(ui, |ui| pose_panel(ui, ed));
            ui.separator();
        }
        match ed.selected {
        Some(id) if ed.scene.node(id).is_some() => node_inspector(ui, ed, id),
        _ => {
            ui.weak(tl!("Click an object or pick it in the list."));
            ui.separator();
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(tl!("Sky"));
                changed |= ui.color_edit_button_rgb(&mut ed.scene.background).changed();
                ui.label(tl!("Ambient"));
                changed |= ui.color_edit_button_rgb(&mut ed.scene.ambient).changed();
            });
            changed |= ui.add(egui::Slider::new(&mut ed.scene.camera.fov, 10.0..=120.0).text(tl!("Field of view"))).changed();
            if changed {
                ed.touch();
            }
        }
        }
    });
}

/// Skeleton display, what dragging does, and saved poses.
fn pose_panel(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    let mut changed = false;
    changed |= ui.checkbox(&mut ed.show_bones, tl!("Show skeleton")).changed();
    ui.horizontal(|ui| {
        ui.radio_value(&mut ed.pose_mode, PoseMode::Ik, tl!("Move (IK)"));
        ui.radio_value(&mut ed.pose_mode, PoseMode::Rotate, tl!("Rotate"));
    });
    if ed.pose_mode == PoseMode::Ik {
        ui.add(egui::Slider::new(&mut ed.ik_chain, 1..=8).text(tl!("IK chain")));
    }
    ui.weak(tl!("Drag a bone dot in the viewport."));
    if ui.button(tl!("Reset pose")).clicked() {
        ed.scene.reset_pose();
        changed = true;
    }
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut ed.pose_name);
        if ui.button(tl!("Save pose")).clicked() && !ed.pose_name.trim().is_empty() {
            let name = ed.pose_name.trim().to_string();
            ed.scene.capture_pose(&name);
        }
    });
    let (mut apply, mut delete) = (None, None);
    for (i, p) in ed.scene.poses.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(&p.name);
            if ui.small_button(tl!("Apply")).clicked() {
                apply = Some(i);
            }
            if ui.small_button(tl!("Delete")).clicked() {
                delete = Some(i);
            }
        });
    }
    if let Some(i) = apply {
        match ed.scene.apply_pose(i) {
            Ok(()) => changed = true,
            Err(e) => ed.status = e.to_string(),
        }
    }
    if let Some(i) = delete {
        ed.scene.poses.remove(i);
    }
    if changed {
        ed.touch();
    }
}

enum Action {
    Import,
    Render,
    New,
    Save,
    Open,
}

fn footer(ui: &mut egui::Ui, ed: &mut Scene3dEditor, have_doc: bool) -> Option<Action> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui.button(tl!("New")).clicked() {
            action = Some(Action::New);
        }
        if ui.button(tl!("Open Scene…")).clicked() {
            action = Some(Action::Open);
        }
        if ui.button(tl!("Save Scene…")).clicked() {
            action = Some(Action::Save);
        }
        ui.separator();
        ui.label(tl!("Size"));
        ui.add(egui::DragValue::new(&mut ed.out_width).range(1..=photocraft_render3d::MAX_EDGE).speed(4));
        ui.label("×");
        ui.add(egui::DragValue::new(&mut ed.out_height).range(1..=photocraft_render3d::MAX_EDGE).speed(4));
        ui.add(egui::Slider::new(&mut ed.out_ssaa, 1..=4).text(tl!("Smoothing")));
        ui.checkbox(&mut ed.out_transparent, tl!("Transparent sky"));
        let render = ui.add_enabled(have_doc, egui::Button::new(tl!("Render to Layer")));
        if render.on_disabled_hover_text(tl!("Open a document first.")).clicked() {
            action = Some(Action::Render);
        }
    });
    if let Some(name) = &ed.file_name {
        ui.weak(name);
    }
    if !ed.status.is_empty() {
        ui.colored_label(Color32::from_rgb(230, 110, 90), &ed.status);
    }
    action
}

fn run_action(app: &mut PhotocraftApp, ctx: &egui::Context, action: Action) {
    match action {
        Action::Import => {
            let asked = app.pick_file_bytes_filtered(&["glb", "vrm", "gltf"], |app, name, bytes| {
                app.scene3d.import_model(&name, &bytes)?;
                app.scene3d.status.clear();
                Ok(json!({"imported": name}))
            });
            if let Err(e) = asked {
                app.scene3d.status = e;
            }
        }
        Action::New => {
            let (w, h, s, t) = (app.scene3d.out_width, app.scene3d.out_height, app.scene3d.out_ssaa, app.scene3d.out_transparent);
            let open = app.scene3d.open;
            app.scene3d = Scene3dEditor { open, out_width: w, out_height: h, out_ssaa: s, out_transparent: t, ..Scene3dEditor::default() };
        }
        Action::Render => {
            let ed = &app.scene3d;
            let params = json!({
                "scene": serde_json::to_value(&ed.scene).unwrap_or_default(),
                "width": ed.out_width, "height": ed.out_height, "ssaa": ed.out_ssaa, "transparent": ed.out_transparent,
            });
            app.scene3d.status = match crate::menus::invoke(app, ctx, "scene3d.renderToLayer", params) {
                Ok(_) => String::new(),
                Err(e) => e,
            };
        }
        Action::Save => {
            let asked = app.pick_save("scene.pcscene", |app, path| {
                let bytes = app.scene3d.scene.to_json().map_err(|e| e.to_string())?.into_bytes();
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&path, &bytes)?;
                app.scene3d.file_name = Some(path.clone());
                Ok(json!({"path": path}))
            });
            if let Err(e) = asked {
                app.scene3d.status = e;
            }
        }
        Action::Open => {
            let asked = app.pick_file_bytes_filtered(&["pcscene"], |app, name, bytes| {
                let text = String::from_utf8(bytes).map_err(|_| format!("{name}: not a text scene file"))?;
                let mut scene = Scene::from_json(&text).map_err(|e| format!("{name}: {e}"))?;
                let problems = load_missing_assets(&mut scene);
                app.scene3d.load(scene, Some(name.clone()));
                app.scene3d.status = problems.join("; ");
                Ok(json!({"opened": name}))
            });
            if let Err(e) = asked {
                app.scene3d.status = e;
            }
        }
    }
}

/// Draws the editor window while it is open.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.scene3d.open {
        return;
    }
    let have_doc = app.session.active().is_some();
    let mut ed = std::mem::take(&mut app.scene3d);
    let mut open = true;
    let mut action = None;
    egui::Window::new(tl!("3D Pose Editor")).id(egui::Id::new("scene3d-editor")).open(&mut open).default_size(Vec2::new(1000.0, 640.0)).resizable(true).show(ctx, |ui| {
        egui::Panel::bottom("scene3d-footer").show(ui, |ui| action = footer(ui, &mut ed, have_doc));
        egui::Panel::right("scene3d-side").default_size(300.0).show(ui, |ui| side_panels(ui, &mut ed));
        egui::CentralPanel::default().show(ui, |ui| viewport(ui, &mut ed));
    });
    ed.open = open;
    let import = std::mem::take(&mut ed.want_import);
    app.scene3d = ed;
    if let Some(a) = action {
        run_action(app, ctx, a);
    }
    if import {
        run_action(app, ctx, Action::Import);
    }
}

#[cfg(test)]
#[path = "scene3d_ui_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "scene3d_ui_models_tests.rs"]
mod models_tests;
