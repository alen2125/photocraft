//! Window › 3D Pose Editor: build a scene of primitives and lights, frame it with an orbit camera,
//! and render it into a pixel layer (`scene3d.renderToLayer`). The scene saves to and opens from
//! a `.pcscene` file so a pose can be fixed later. The viewport preview uses the software
//! renderer (`photocraft-render3d`); nothing here touches the document until Render to Layer.

use egui::{Color32, Sense, Stroke, Vec2, pos2, vec2};
use photocraft_scene3d::{Camera, Light, LightKind, Material, Node, NodeId, NodeKind, Primitive, Scene, Transform};
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

fn viewport(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    let avail = ui.available_size_before_wrap().max(vec2(160.0, 120.0));
    let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let scale = (PREVIEW_EDGE / (rect.width() * ppp).max(rect.height() * ppp)).min(1.0);
    let size = (((rect.width() * ppp * scale) as u32).max(8), ((rect.height() * ppp * scale) as u32).max(8));

    let mut moved = false;
    let (dx, dy) = (resp.drag_delta().x, resp.drag_delta().y);
    if resp.dragged_by(egui::PointerButton::Primary) && (dx != 0.0 || dy != 0.0) {
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
        let (px, py) = ((p.x - rect.min.x) / rect.width() * size.0 as f32, (p.y - rect.min.y) / rect.height() * size.1 as f32);
        ed.selected = photocraft_render3d::pick(&ed.scene, size.0, size.1, px, py);
    }

    // Fast while the camera moves; anti-aliased once it rests.
    let ssaa = if resp.dragged() { 1 } else { 2 };
    refresh_preview(ui.ctx(), ed, size, ssaa);
    if let Some(p) = &ed.preview {
        ui.painter().image(p.texture.id(), rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        ui.painter().rect_filled(rect, 0.0, Color32::from_gray(40));
    }
    if let Some(id) = ed.selected
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
    egui::ScrollArea::vertical().id_salt("scene3d-nodes").max_height(160.0).show(ui, |ui| node_list(ui, ed));
    ui.separator();
    egui::ScrollArea::vertical().id_salt("scene3d-inspector").show(ui, |ui| match ed.selected {
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
    });
}

enum Action {
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
                let scene = Scene::from_json(&text).map_err(|e| format!("{name}: {e}"))?;
                app.scene3d.status.clear();
                app.scene3d.load(scene, Some(name.clone()));
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
    app.scene3d = ed;
    if let Some(a) = action {
        run_action(app, ctx, a);
    }
}

#[cfg(test)]
#[path = "scene3d_ui_tests.rs"]
mod tests;
