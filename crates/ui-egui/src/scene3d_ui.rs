//! Window › 3D Pose Editor: build a scene of primitives, lights and imported models (glTF, GLB,
//! VRM), pose a model's skeleton by turning bones or dragging them (inverse kinematics), frame
//! it with an orbit camera, and render it into a pixel layer (`scene3d.renderToLayer`). The scene
//! saves to and opens from a `.pcscene` file so a pose can be fixed later. The viewport preview
//! uses the software renderer (`photocraft-render3d`); nothing here touches the document until
//! Render to Layer.

use std::sync::Arc;

use egui::{Color32, pos2};
use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{Camera, GizmoMode, Light, LightKind, Material, Node, NodeId, NodeKind, Primitive, RotationLimits, Scene, Transform};
use serde_json::json;

use crate::PhotocraftApp;

const OPEN_ID: &str = "ui.scene3d.open";
/// Whether `id` is a command this module carries out.
pub fn handles(id: &str) -> bool {
    id == OPEN_ID
}

pub fn invoke(app: &mut PhotocraftApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    if !handles(id) {
        return None;
    }
    let Some(st) = app.session.active() else { return Some(Err(tl!("Open a document first.").to_string())) };
    let (doc, size) = (st.doc.id, [st.doc.size.width, st.doc.size.height]);
    let ed = &mut app.scene3d;
    ed.open = true;
    ed.owner = Some(doc);
    ed.doc_size = size;
    ed.touch();
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
    pub(crate) selected: Option<NodeId>,
    pub(crate) orbit: Orbit,
    pub(crate) preview: Option<Preview>,
    /// Bumped on every change to the scene or camera; the preview re-renders when it moves.
    pub(crate) rev: u64,
    pub(crate) status: String,
    pub(crate) file_name: Option<String>,
    pub(crate) out_ssaa: u32,
    pub(crate) out_transparent: bool,
    /// How sharp the picture is while something moves (a fraction of the full size), and whether it
    /// is drawn at full size and smoothed then too. Kept between runs.
    pub(crate) motion_scale: f32,
    pub(crate) no_blur: bool,
    /// The quality settings were read back from the saved ones.
    pub(crate) quality_restored: bool,
    /// What the renderer was last asked for: its key and the camera it was given.
    pub(crate) submitted: Option<((u64, u32, u32, u32), photocraft_scene3d::Camera)>,
    pub(crate) show_bones: bool,
    pub(crate) pose_mode: PoseMode,
    /// How many bones above the dragged one turn in IK mode.
    pub(crate) ik_chain: usize,
    pub(crate) pose_name: String,
    /// Set by the side panel; carried out after the window is drawn (it opens a file dialog).
    pub(crate) want_import: bool,
    /// Same for importing a VaM look (`.vap`).
    pub(crate) want_look: bool,
    pub(crate) worker: crate::scene3d_worker::PreviewWorker,
    /// `egui` time of the last camera or pose change; the preview is coarse until it settles.
    pub(crate) last_change: f64,
    /// The document the mode was started on; leaving it closes the mode.
    pub(crate) owner: Option<photocraft_doc::DocId>,
    /// Size in pixels of that document: the scene is rendered over all of it.
    pub(crate) doc_size: [u32; 2],
    /// The pressed pointer, between a press and its release.
    pub(crate) pointer: Option<crate::scene3d_canvas::PointerState>,
    /// A camera or pose change happened this frame (the preview goes coarse for a moment).
    pub(crate) pending_change: bool,
    /// Text typed in the pose-slider filter box.
    pub(crate) control_filter: String,
    /// The Virt-A-Mate folder and the morphs read from it.
    pub(crate) vam: crate::scene3d_vam::VamState,
    /// Physics feel: gravity as a multiple of Earth's, and how quickly limbs lose speed.
    pub(crate) gravity_scale: f32,
    pub(crate) damping_scale: f32,
    /// Strength of the trunk, neck and leg muscles, a multiplier of VaM's numbers.
    pub(crate) muscle_scale: f32,
    /// The search text of the Link-to list.
    pub(crate) link_filter: String,
    /// The physics runs all the time (as in VaM) until this is switched off.
    pub(crate) live: bool,
    /// Bumped by every step of the physics and every target dragged while it runs: the picture
    /// changes, but the scene was not edited (no undo step, no new world).
    pub(crate) sim_rev: u64,
    /// The scene revision the running world was built from (`None`: none was built).
    pub(crate) world_rev: Option<u64>,
    /// A node that is On only while it is held with the pointer.
    pub(crate) temp_hold: Option<NodeId>,
    /// Frames since anything moved: the physics stops drawing when it has been quiet a while.
    pub(crate) quiet_frames: u32,
    /// What the gizmo does (W/E/R) and whether its axes are the node's own.
    pub(crate) gizmo_mode: GizmoMode,
    pub(crate) gizmo_local: bool,
    /// Scene edits for undo and redo: the scene before each edit, the scene as last seen, and
    /// the revision that was last looked at.
    pub(crate) history: Vec<Scene>,
    pub(crate) future: Vec<Scene>,
    pub(crate) committed: Option<Scene>,
    pub(crate) history_rev: u64,
    /// The running physics (gravity, collisions), the scene as it was when it started (for
    /// Revert and for one undo step), and time not yet stepped.
    pub(crate) physics: Option<photocraft_physics3d::World>,
    pub(crate) play_base: Option<Scene>,
    pub(crate) play_acc: f32,
}

/// What dragging a bone in the viewport does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoseMode {
    /// Turn the bone about the camera's axes (forward kinematics).
    Rotate,
    /// Pull the bone to the pointer by turning the bones above it.
    Ik,
}

/// A bone drag in progress: the plane facing the camera through the bone where it was grabbed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BoneDrag {
    pub(crate) id: NodeId,
    pub(crate) plane_point: Vec3,
}

pub(crate) struct Preview {
    pub(crate) texture: egui::TextureHandle,
    pub(crate) key: (u64, u32, u32, u32),
    /// The camera the picture was drawn with: the skeleton over it is drawn with this one too, so it
    /// stays on the body while a newer picture is still being made.
    pub(crate) camera: Option<photocraft_scene3d::Camera>,
}

impl Default for Scene3dEditor {
    fn default() -> Self {
        // Just a sun: on a canvas the scene sits over the picture, so nothing else is added.
        let scene = Scene::new();
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
            out_ssaa: 2,
            out_transparent: true,
            motion_scale: 0.5,
            no_blur: false,
            quality_restored: false,
            submitted: None,
            show_bones: true,
            pose_mode: PoseMode::Ik,
            ik_chain: 2,
            pose_name: "Pose 1".into(),
            want_import: false,
            want_look: false,
            worker: Default::default(),
            last_change: f64::NEG_INFINITY,
            owner: None,
            doc_size: [1024, 768],
            pointer: None,
            pending_change: false,
            control_filter: String::new(),
            gravity_scale: 1.0,
            damping_scale: 1.0,
            muscle_scale: photocraft_physics3d::DEFAULT_MUSCLE,
            link_filter: String::new(),
            vam: Default::default(),
            live: true,
            sim_rev: 0,
            world_rev: None,
            temp_hold: None,
            quiet_frames: 0,
            gizmo_mode: GizmoMode::Both,
            gizmo_local: false,
            history: Vec::new(),
            future: Vec::new(),
            committed: None,
            history_rev: 0,
            physics: None,
            play_base: None,
            play_acc: 0.0,
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

    pub(crate) fn touch(&mut self) {
        self.rev += 1;
    }

    /// An edit that does not change what the physics world was built from (the camera, a node's
    /// target, a node switched On or Off): an undo step and a new picture, but the world goes on.
    pub(crate) fn touch_keep_world(&mut self) {
        let in_sync = self.world_rev == Some(self.rev);
        self.rev += 1;
        if in_sync {
            self.world_rev = Some(self.rev);
        }
    }

    /// What the preview is of: the scene revision and the physics' progress.
    pub(crate) fn preview_rev(&self) -> u64 {
        self.rev.wrapping_mul(1 << 24).wrapping_add(self.sim_rev)
    }

    /// Adds a model from file bytes (`name` is the path on the desktop) and frames its skeleton.
    pub(crate) fn import_model(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let model = photocraft_scene3d::model3d::load(name, bytes).map_err(|e| format!("{name}: {e}"))?;
        // A path lets the scene file find the model again; the browser gives a bare name.
        let path = std::path::Path::new(name).is_absolute().then(|| name.to_string());
        let root = self.scene.add_model(Arc::new(model), path).map_err(|e| e.to_string())?;
        self.selected = Some(root);
        // With the physics always running a fresh figure stands on its feet: feet and hips On.
        if self.live {
            self.scene.set_holds_below(Some(root), Some(&["hips", "leftFoot", "rightFoot"]));
        }
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

    pub(crate) fn add_primitive(&mut self, p: Primitive) {
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
pub(crate) fn refresh_preview(ctx: &egui::Context, ed: &mut Scene3dEditor, size: (u32, u32), ssaa: u32) {
    // Show what the renderer has finished.
    if let Some((key, result)) = ed.worker.poll() {
        match result {
            Ok(img) => {
                let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.rgba);
                let camera = ed.submitted.as_ref().filter(|(k, _)| *k == key).map(|(_, c)| c.clone());
                match ed.preview.as_mut() {
                    Some(p) => {
                        p.texture.set(color, egui::TextureOptions::LINEAR);
                        p.key = key;
                        p.camera = camera;
                    }
                    None => ed.preview = Some(Preview { texture: ctx.load_texture("scene3d-preview", color, egui::TextureOptions::LINEAR), key, camera }),
                }
            }
            Err(e) => ed.status = e,
        }
    }
    // Ask for the newest state once the renderer is free (it asks for a repaint when it ends).
    let want = (ed.preview_rev(), size.0, size.1, ssaa);
    if ed.preview.as_ref().is_none_or(|p| p.key != want) && !ed.worker.busy() && !ed.worker.is_last(want) {
        let opts = photocraft_render3d::RenderOptions { width: size.0, height: size.1, ssaa, transparent: ed.out_transparent };
        ed.submitted = Some((want, ed.scene.camera.clone()));
        ed.worker.submit(ctx, want, ed.scene.clone(), opts);
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
    egui::CollapsingHeader::new(tl!("Exact values")).default_open(false).show(ui, |ui| {
        changed |= vec3_row(ui, tl!("Position"), &mut node.transform.translation, 0.02);
        changed |= vec3_row(ui, tl!("Rotation"), &mut node.transform.rotation, 0.5);
        changed |= vec3_row(ui, tl!("Scale"), &mut node.transform.scale, 0.01);
    });
    ui.separator();
    if matches!(node.kind, NodeKind::Primitive { .. }) {
        changed |= ui.checkbox(&mut node.pinned, tl!("On (stays put)")).on_hover_text(tl!("Off: it falls when you press Play.")).changed();
    }
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
            if let Some(j) = &mut node.joint {
                changed |= ui.checkbox(&mut j.enabled, tl!("Anatomical limit")).on_hover_text(tl!("The joint stops where a body's joint would. Untick to move it past that.")).changed();
            }
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

/// What the selected node is tied to and how it is held, kept above the pose panels so it is
/// always in reach: Position and Orientation On or Off (bones), then the link to another node.
fn node_controls(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    let Some(id) = ed.selected else { return };
    let Some(name) = ed.scene.node(id).map(|n| n.name.clone()) else { return };
    let mut changed = false;
    // A node switched On or Off: the running world takes it up by itself.
    let mut held = false;
    ui.label(egui::RichText::new(name).strong());
    if ed.scene.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Bone)) {
        // Position and orientation each On or Off, as in VaM. On: held in space; the limb turns to
        // keep it there while you move the bones above it, and it stays put in Play. Off: it follows
        // the bones above it, and hangs and swings in Play.
        let (pos_before, rot_before) = ed.scene.node(id).map_or((false, false), |n| (n.hold.is_some(), n.hold_rot.is_some()));
        let (mut pos_on, mut rot_on) = (pos_before, rot_before);
        ui.horizontal(|ui| {
            ui.add_sized([80.0, 18.0], egui::Label::new(tl!("Position")));
            ui.radio_value(&mut pos_on, true, tl!("On")).on_hover_text(tl!("The bone stays at this point in space while you move the bones above it, and does not fall in Play."));
            ui.radio_value(&mut pos_on, false, tl!("Off")).on_hover_text(tl!("The bone follows the bones above it, and hangs from them in Play."));
        });
        ui.horizontal(|ui| {
            ui.add_sized([80.0, 18.0], egui::Label::new(tl!("Orientation")));
            ui.radio_value(&mut rot_on, true, tl!("On")).on_hover_text(tl!("The bone keeps the way it faces while the bones above it move or turn, and does not spin in Play."));
            ui.radio_value(&mut rot_on, false, tl!("Off")).on_hover_text(tl!("The bone turns with the bones above it, and swings freely in Play."));
        });
        if pos_on != pos_before {
            match ed.scene.set_hold(id, pos_on) {
                Ok(()) => held = true,
                Err(e) => ed.status = e.to_string(),
            }
        }
        if rot_on != rot_before {
            match ed.scene.set_hold_rotation(id, rot_on) {
                Ok(()) => held = true,
                Err(e) => ed.status = e.to_string(),
            }
        }
    }
    // Some(None) = unlink, Some(Some(t)) = link to `t`; applied once the node borrow ends.
    let mut link_action: Option<Option<NodeId>> = None;
    let others: Vec<(NodeId, String)> = ed.scene.nodes.iter().filter(|n| n.id != id).map(|n| (n.id, n.name.clone())).collect();
    let mut filter = std::mem::take(&mut ed.link_filter);
    if let Some(node) = ed.scene.node_mut(id) {
        link_section(ui, node, &others, &mut filter, &mut link_action, &mut changed);
    }
    ed.link_filter = filter;
    if let Some(action) = link_action {
        let done = match action {
            Some(target) => ed.scene.link_to(id, target),
            None => ed.scene.unlink(id),
        };
        match done {
            Ok(()) => changed = true,
            Err(e) => ed.status = e.to_string(),
        }
    }
    if changed {
        ed.touch();
    } else if held {
        ed.touch_keep_world();
    }
    ui.separator();
}

/// Ties the node to another at a distance and angle (a sun on a face, orbiting it), and edits
/// that relation. Picking a target keeps the node where it is; the numbers then take over.
fn link_section(ui: &mut egui::Ui, node: &mut Node, others: &[(NodeId, String)], filter: &mut String, action: &mut Option<Option<NodeId>>, changed: &mut bool) {
    let current = node.link.as_ref().and_then(|l| others.iter().find(|o| o.0 == l.target)).map_or_else(|| tl!("(none)").to_string(), |o| o.1.clone());
    egui::ComboBox::from_label(tl!("Link to")).selected_text(current).show_ui(ui, |ui| {
        ui.add(egui::TextEdit::singleline(filter).hint_text(tl!("Search…")).desired_width(160.0));
        if ui.selectable_label(node.link.is_none(), tl!("(none)")).clicked() {
            *action = Some(None);
        }
        let wanted = filter.trim().to_lowercase();
        for (oid, name) in others.iter().filter(|(_, n)| wanted.is_empty() || n.to_lowercase().contains(&wanted)) {
            if ui.selectable_label(node.link.as_ref().is_some_and(|l| l.target == *oid), name).clicked() {
                *action = Some(Some(*oid));
            }
        }
    });
    let Some(l) = &mut node.link else { return };
    ui.horizontal(|ui| {
        ui.add_sized([62.0, 18.0], egui::Label::new(tl!("Distance")));
        *changed |= ui.add(egui::DragValue::new(&mut l.distance).speed(0.02).range(0.01..=1000.0).max_decimals(3)).changed();
    });
    *changed |= ui.add(egui::Slider::new(&mut l.yaw, -180.0..=180.0).text(tl!("Orbit"))).changed();
    *changed |= ui.add(egui::Slider::new(&mut l.pitch, -89.0..=89.0).text(tl!("Height angle"))).changed();
    *changed |= ui.checkbox(&mut l.look_at, tl!("Aim at target")).changed();
    let mut follow = l.space == photocraft_scene3d::LinkSpace::Target;
    if ui.checkbox(&mut follow, tl!("Follow target's turn")).changed() {
        l.space = if follow { photocraft_scene3d::LinkSpace::Target } else { photocraft_scene3d::LinkSpace::World };
        *changed = true;
    }
    ui.weak(tl!("Its position follows the link; drag it on the canvas to change the link."));
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
        // Small joints and the model's meshes (one per material slot) stay out of the list unless picked.
        if (n.minor || matches!(n.kind, NodeKind::Mesh(_))) && ed.selected != Some(id) {
            continue;
        }
        let icon = match n.kind {
            NodeKind::Empty => "◇",
            NodeKind::Primitive { .. } => "■",
            NodeKind::Light(_) => "☀",
            NodeKind::Bone => "◦",
            NodeKind::Mesh(_) => "▲",
        };
        let hidden = if n.visible { "" } else { " ·" };
        let falls = if !n.pinned && matches!(n.kind, NodeKind::Primitive { .. } | NodeKind::Bone) { " ↓" } else { "" };
        let label = format!("{}{icon}  {}{hidden}{falls}", "    ".repeat(depth_of(&ed.scene, id)), n.name);
        if ui.selectable_label(ed.selected == Some(id), label).clicked() {
            clicked = Some(id);
        }
    }
    if clicked.is_some() {
        ed.selected = clicked;
    }
}

/// Every bone's parent and its position in preview pixels (bones out of view are left out).
pub(crate) fn bone_screen(scene: &Scene, size: (u32, u32)) -> Vec<(NodeId, Option<NodeId>, (f32, f32))> {
    let list = scene.bone_positions();
    let pts: Vec<Vec3> = list.iter().map(|b| b.2).collect();
    let screen = photocraft_render3d::project_points(scene, size.0, size.1, &pts);
    list.iter().zip(screen).filter_map(|(b, s)| s.map(|s| (b.0, b.1, s))).collect()
}

/// Where each bone's cube is drawn, in preview pixels: a bone whose Position is On shows its cube
/// at its target (which may be far from the body), so that is where it is picked.
pub(crate) fn cube_screen(scene: &Scene, size: (u32, u32)) -> Vec<(NodeId, Option<NodeId>, (f32, f32))> {
    let list = scene.bone_positions();
    let pts: Vec<Vec3> = list.iter().map(|b| scene.node(b.0).and_then(|n| n.hold).map(Vec3::from).filter(|t| t.is_finite()).unwrap_or(b.2)).collect();
    let screen = photocraft_render3d::project_points(scene, size.0, size.1, &pts);
    list.iter().zip(screen).filter_map(|(b, s)| s.map(|s| (b.0, b.1, s))).collect()
}

/// The bone dot closest to `(x, y)` within `radius` preview pixels.
pub(crate) fn nearest_bone(bones: &[(NodeId, Option<NodeId>, (f32, f32))], x: f32, y: f32, radius: f32) -> Option<NodeId> {
    bones
        .iter()
        .map(|b| (b.0, (b.2.0 - x).hypot(b.2.1 - y)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The camera's right and up axes and its view direction in the world.
pub(crate) fn camera_axes(c: &Camera) -> (Vec3, Vec3, Vec3) {
    let f = (Vec3::from(c.target) - Vec3::from(c.position)).normalize_or_zero();
    let f = if f == Vec3::ZERO { Vec3::NEG_Z } else { f };
    let right = f.cross(Vec3::Y).normalize_or_zero();
    let right = if right == Vec3::ZERO { Vec3::X } else { right };
    (right, right.cross(f), f)
}

/// Applies one pointer step of a bone drag: pull it to the pointer (IK, or sliding the top
/// bone) or turn it about the camera's axes.
pub(crate) fn drag_bone(ed: &mut Scene3dEditor, drag: BoneDrag, size: (u32, u32), pointer: (f32, f32), delta: (f32, f32)) {
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
    if let Err(e) = result.and_then(|()| photocraft_scene3d::enforce_holds(&mut ed.scene, drag.id, ed.pose_mode == PoseMode::Rotate, 12)) {
        ed.status = e.to_string();
    }
}

fn side_panels(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    // A scene with a VaM figure has a second page, for its morphs.
    if crate::scene3d_vam::page_switch(ui, ed) {
        return;
    }
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
    ui.checkbox(&mut ed.live, tl!("Live physics")).on_hover_text(tl!("The physics runs all the time, as in VaM: pull a node and the body follows. Untick to pose with IK and use Play."));
    ui.horizontal(|ui| {
        if ed.live && ed.play_base.is_none() {
            // The physics is running by itself: nothing to start or stop.
        } else if ed.physics.is_none() {
            if ui.button(tl!("▶ Play")).on_hover_text(tl!("Everything that is Off falls and collides; what is On stays put; hair swings.")).clicked()
                && let Err(e) = crate::scene3d_canvas::play(ed)
            {
                ed.status = e;
            }
        } else {
            if ui.button(tl!("■ Stop (keep)")).clicked() {
                crate::scene3d_canvas::stop(ed, true);
            }
            if ui.button(tl!("↺ Revert")).clicked() {
                crate::scene3d_canvas::stop(ed, false);
            }
        }
    });
    quality_panel(ui, ed);
    // How the physics feels; takes effect at once, also while it runs.
    ui.horizontal(|ui| {
        let g = ui.add(egui::Slider::new(&mut ed.gravity_scale, 0.0..=3.0).text(tl!("Gravity")).max_decimals(2));
        let d = ui.add(egui::Slider::new(&mut ed.damping_scale, 0.1..=3.0).logarithmic(true).text(tl!("Damping")).max_decimals(2));
        let m = ui.add(egui::Slider::new(&mut ed.muscle_scale, 0.0..=2.0).text(tl!("Muscles")).max_decimals(2));
        if (g.changed() || d.changed() || m.changed())
            && let Some(world) = ed.physics.as_mut()
        {
            world.set_tuning(ed.gravity_scale, ed.damping_scale);
            world.set_muscle(ed.muscle_scale);
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().id_salt("scene3d-nodes").max_height(200.0).show(ui, |ui| node_list(ui, ed));
    ui.separator();
    egui::ScrollArea::vertical().id_salt("scene3d-inspector").max_height(380.0).show(ui, |ui| {
        node_controls(ui, ed);
        if ed.scene.nodes.iter().any(|n| matches!(n.kind, NodeKind::Bone)) {
            egui::CollapsingHeader::new(tl!("Pose")).default_open(true).show(ui, |ui| pose_panel(ui, ed));
            ui.separator();
            if ed.scene.assets.iter().any(|a| ed.scene.model(a.id).is_some_and(|m| !m.pose_controls.is_empty())) {
                egui::CollapsingHeader::new(tl!("Pose sliders")).default_open(false).show(ui, |ui| controls_panel(ui, ed));
                ui.separator();
            }

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

/// The sliders a model ships with (fingers, toes, tongue…), grouped, with a filter box.
fn controls_panel(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    let mut changed = false;
    let mut error = None;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut ed.control_filter).hint_text(tl!("Filter…")).desired_width(140.0));
        if ui.button(tl!("Reset sliders")).clicked() {
            for a in ed.scene.assets.clone() {
                if let Err(e) = ed.scene.reset_controls(a.id) {
                    error = Some(e.to_string());
                }
            }
            changed = true;
        }
    });
    let filter = ed.control_filter.trim().to_lowercase();
    for asset in ed.scene.assets.clone() {
        let Some(model) = ed.scene.model(asset.id).cloned() else { continue };
        if model.pose_controls.is_empty() {
            continue;
        }
        let mut groups: Vec<&str> = Vec::new();
        for c in &model.pose_controls {
            if !groups.contains(&c.group.as_str()) {
                groups.push(c.group.as_str());
            }
        }
        for group in groups {
            let shown: Vec<_> = model.pose_controls.iter().filter(|c| c.group == group && (filter.is_empty() || c.label.to_lowercase().contains(&filter))).collect();
            if shown.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(group.replace('/', " · ")).id_salt((asset.id, group)).default_open(!filter.is_empty()).show(ui, |ui| {
                for c in shown {
                    let mut v = ed.scene.control_value(asset.id, &c.name);
                    let (lo, hi) = if c.min <= c.max { (c.min, c.max) } else { (c.max, c.min) };
                    if ui.add(egui::Slider::new(&mut v, lo..=hi).text(&c.label).max_decimals(2)).changed() {
                        match ed.scene.set_control(asset.id, &c.name, v) {
                            Ok(()) => changed = true,
                            Err(e) => error = Some(e.to_string()),
                        }
                    }
                }
            });
        }
    }
    if let Some(e) = error {
        ed.status = e;
    }
    if changed {
        ed.touch();
        ed.pending_change = true;
    }
}

/// Skeleton display, what dragging does, and saved poses.
fn pose_panel(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    let mut changed = false;
    changed |= ui.checkbox(&mut ed.show_bones, tl!("Show skeleton")).changed();
    // One switch for every joint that carries anatomical limits (the figure's own numbers).
    let any = ed.scene.nodes.iter().any(|n| n.joint.is_some());
    if any {
        let mut on = ed.scene.nodes.iter().filter_map(|n| n.joint.as_ref()).all(|j| j.enabled);
        if ui.checkbox(&mut on, tl!("Anatomical limits")).on_hover_text(tl!("Joints stop where a body's would. Untick to pose without limits.")).changed() {
            for j in ed.scene.nodes.iter_mut().filter_map(|n| n.joint.as_mut()) {
                j.enabled = on;
            }
            changed = true;
        }
    }
    ui.horizontal(|ui| {
        ui.radio_value(&mut ed.pose_mode, PoseMode::Ik, tl!("Move (IK)"));
        ui.radio_value(&mut ed.pose_mode, PoseMode::Rotate, tl!("Rotate"));
    });
    if ed.pose_mode == PoseMode::Ik {
        ui.add(egui::Slider::new(&mut ed.ik_chain, 1..=8).text(tl!("IK chain")));
    }
    ui.weak(tl!("Drag a bone dot or a gizmo handle on the canvas. W / E / R: move, rotate, scale."));
    ui.weak(tl!("A bone whose Position or Orientation is On stays put while you move the bones above it and in Play."));
    ui.horizontal_wrapped(|ui| {
        if ui.button(tl!("All On")).clicked() {
            ed.scene.set_all_holds(None);
            changed = true;
        }
        if ui.button(tl!("All Off")).clicked() {
            ed.scene.set_all_holds(Some(&[]));
            changed = true;
        }
        if ui.button(tl!("Feet and head only")).on_hover_text(tl!("Switch everything Off except the feet and the head.")).clicked() {
            ed.scene.set_all_holds(Some(&["head", "leftFoot", "rightFoot"]));
            changed = true;
        }
    });
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

pub(crate) enum Action {
    Import,
    ImportLook,
    Bake,
    Close,
    New,
    Save,
    Open,
}

fn footer(ui: &mut egui::Ui, ed: &mut Scene3dEditor) -> Option<Action> {
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
    });
    ui.horizontal(|ui| {
        if ui.button(tl!("Bake to Layer")).clicked() {
            action = Some(Action::Bake);
        }
        if ui.button(tl!("Cancel")).clicked() {
            action = Some(Action::Close);
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

pub(crate) fn run_action(app: &mut PhotocraftApp, ctx: &egui::Context, action: Action) {
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
        Action::ImportLook => {
            let asked = app.pick_file_bytes_filtered(&["vap"], |app, name, bytes| {
                let summary = crate::scene3d_vam::import_look(&mut app.scene3d, &name, &bytes)?;
                app.scene3d.status = summary.clone();
                Ok(json!({"imported": name, "summary": summary}))
            });
            if let Err(e) = asked {
                app.scene3d.status = e;
            }
        }
        Action::New => {
            let old = &app.scene3d;
            let kept = Scene3dEditor { open: old.open, owner: old.owner, doc_size: old.doc_size, out_ssaa: old.out_ssaa, out_transparent: old.out_transparent, ..Scene3dEditor::default() };
            app.scene3d = kept;
        }
        Action::Bake => crate::scene3d_canvas::bake(app, ctx),
        Action::Close => crate::scene3d_canvas::close(&mut app.scene3d),
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
                crate::scene3d_vam::restore_morphs(&mut app.scene3d);
                Ok(json!({"opened": name}))
            });
            if let Err(e) = asked {
                app.scene3d.status = e;
            }
        }
    }
}

/// Every frame: keeps the on-canvas mode going (preview, document check) and draws its panel,
/// the node list, inspector and poses, as a floating window next to the canvas.
fn quality_setting_id() -> egui::Id {
    egui::Id::new("scene3d-quality")
}

/// What is drawn while things move, and how much of the hair: kept between runs.
fn quality_panel(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    if !ed.quality_restored {
        ed.quality_restored = true;
        if let Some((scale, no_blur, hair)) = ui.ctx().data_mut(|d| d.get_persisted::<(f32, bool, f32)>(quality_setting_id())) {
            ed.motion_scale = scale.clamp(0.25, 1.0);
            ed.no_blur = no_blur;
            let _ = ed.scene.set_hair_quality(hair);
        }
    }
    let mut changed = false;
    egui::CollapsingHeader::new(tl!("Quality")).default_open(false).show(ui, |ui| {
        changed |= ui.checkbox(&mut ed.no_blur, tl!("Do not coarsen while moving")).on_hover_text(tl!("Draw at full size and smoothed while the camera or a pose moves. Sharper, but slower.")).changed();
        changed |= ui.add_enabled(!ed.no_blur, egui::Slider::new(&mut ed.motion_scale, 0.25..=1.0).text(tl!("Sharpness while moving")).max_decimals(2)).changed();
        let mut hair = ed.scene.hair_quality;
        if ui.add(egui::Slider::new(&mut hair, 0.0..=1.0).text(tl!("Hair quality")).max_decimals(2)).on_hover_text(tl!("How much of the hair is drawn: all of it at 1, the guide strands only at 0. Lower it if it is slow.")).changed() && ed.scene.set_hair_quality(hair).is_ok() {
            ed.touch();
            ed.pending_change = true;
            changed = true;
        }
    });
    if changed {
        let value = (ed.motion_scale, ed.no_blur, ed.scene.hair_quality);
        ui.ctx().data_mut(|d| d.insert_persisted(quality_setting_id(), value));
        ed.pending_change = true;
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::scene3d_canvas::frame(app, ctx);
    if !app.scene3d.open {
        return;
    }
    let mut ed = std::mem::take(&mut app.scene3d);
    let mut open = true;
    let mut action = None;
    let screen = ctx.content_rect();
    egui::Window::new(tl!("3D Pose Editor"))
        .id(egui::Id::new("scene3d-panel"))
        .open(&mut open)
        .default_pos(pos2((screen.right() - 340.0).max(0.0), screen.top() + 110.0))
        .default_width(320.0)
        .resizable(true)
        .show(ctx, |ui| {
            side_panels(ui, &mut ed);
            ui.separator();
            action = footer(ui, &mut ed);
        });
    if !open {
        crate::scene3d_canvas::close(&mut ed);
    }
    let import = std::mem::take(&mut ed.want_import);
    let look = std::mem::take(&mut ed.want_look);
    app.scene3d = ed;
    if let Some(a) = action {
        run_action(app, ctx, a);
    }
    if import {
        run_action(app, ctx, Action::Import);
    }
    if look {
        run_action(app, ctx, Action::ImportLook);
    }
}

#[cfg(test)]
#[path = "scene3d_ui_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "scene3d_ui_models_tests.rs"]
mod models_tests;
