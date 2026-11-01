//! Flying the 3D Pose Editor's camera, and camera views kept on the number keys.
//!
//! Tab switches the fly mode on and off. While it is on W A S D move the camera (forward, left,
//! back, right), Z and X move it up and down, C and V bank it left and right (hold for a smooth
//! turn, Shift+press for exactly 15° at a time), a plain drag looks around from where the camera
//! is, and the wheel sets the speed. Ctrl+1…9 saves the camera in a slot and 1…9 goes back to it
//! with a short glide, in either mode. The views are part of the scene (`Scene::views`), so they
//! are saved in the `.pcscene` file; they are not undo steps, like the camera itself.

use egui::{Event, Key, Modifiers};
use photocraft_scene3d::glam::{Quat, Vec3};
use photocraft_scene3d::{Camera, MAX_VIEWS};

use crate::scene3d_ui::{Orbit, Scene3dEditor};

/// A Shift+C or Shift+V press banks the camera to the next multiple of this many degrees.
pub(crate) const ROLL_STEP: f32 = 15.0;
/// Degrees per second while C or V is held.
const ROLL_SPEED: f32 = 60.0;
/// Camera speed in scene units per second, and its limits.
const SPEED_DEFAULT: f32 = 2.0;
const SPEED_MIN: f32 = 0.1;
const SPEED_MAX: f32 = 50.0;
/// Seconds a jump to a saved view takes.
const VIEW_SECONDS: f32 = 0.25;
/// The number keys, slot 1 first.
const NUMBER_KEYS: [Key; MAX_VIEWS] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];

#[derive(Clone, Debug)]
pub(crate) struct FlyState {
    pub(crate) active: bool,
    /// Scene units per second.
    pub(crate) speed: f32,
}

impl Default for FlyState {
    fn default() -> Self {
        Self { active: false, speed: SPEED_DEFAULT }
    }
}

/// A glide from one camera to a saved one.
#[derive(Clone, Debug)]
pub(crate) struct ViewTween {
    from: Camera,
    to: Camera,
    /// 0 at the start, 1 at the end.
    t: f32,
}

/// What the held keys ask for this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FlyInput {
    /// Right, up and forward, each −1, 0 or 1.
    pub(crate) local: Vec3,
    /// −1 banks left, 1 banks right.
    pub(crate) roll: f32,
}

/// Brings an angle into (−180, 180].
fn wrap(deg: f32) -> f32 {
    let r = deg % 360.0;
    if r > 180.0 {
        r - 360.0
    } else if r <= -180.0 {
        r + 360.0
    } else {
        r
    }
}

/// The roll after one Shift+C (`dir` −1) or Shift+V (`dir` 1) press: the next multiple of
/// [`ROLL_STEP`] in that direction, so a banked camera lands on round numbers.
pub(crate) fn roll_step(roll: f32, dir: f32) -> f32 {
    let n = if roll.is_finite() { roll / ROLL_STEP } else { 0.0 };
    let snapped = if dir > 0.0 { (n + 1e-3).floor() + 1.0 } else { (n - 1e-3).ceil() - 1.0 };
    wrap(snapped * ROLL_STEP)
}

/// Changes the speed by a factor, within its limits.
pub(crate) fn scale_speed(fly: &mut FlyState, k: f32) {
    if k.is_finite() && k > 0.0 {
        fly.speed = (fly.speed * k).clamp(SPEED_MIN, SPEED_MAX);
    }
}

/// Turns the view by `dyaw` and `dpitch` degrees about the camera's own position (an orbit turns it
/// about the target, so the target is moved the other way to keep the camera where it is).
pub(crate) fn look(o: &mut Orbit, dyaw: f32, dpitch: f32) {
    if !dyaw.is_finite() || !dpitch.is_finite() || !o.distance.is_finite() {
        return;
    }
    let offset = |yaw: f32, pitch: f32| {
        let (y, p) = (yaw.to_radians(), pitch.clamp(-89.0, 89.0).to_radians());
        Vec3::new(y.sin() * p.cos(), p.sin(), y.cos() * p.cos()) * o.distance
    };
    let eye = Vec3::from(o.target) + offset(o.yaw, o.pitch);
    let (yaw, pitch) = (o.yaw - dyaw, (o.pitch + dpitch).clamp(-89.0, 89.0));
    let target = eye - offset(yaw, pitch);
    if target.is_finite() {
        o.yaw = yaw;
        o.pitch = pitch;
        o.target = target.to_array();
    }
}

fn changed(ed: &mut Scene3dEditor) {
    ed.touch_keep_world();
    ed.pending_change = true;
}

/// Sets the bank of the camera, in degrees.
pub(crate) fn set_roll(ed: &mut Scene3dEditor, roll: f32) {
    if roll.is_finite() {
        ed.scene.camera.roll = wrap(roll);
        changed(ed);
    }
}

/// Moves and banks the camera for `dt` seconds of the given input. True when it moved.
pub(crate) fn step(ed: &mut Scene3dEditor, input: FlyInput, dt: f32) -> bool {
    if !dt.is_finite() || dt <= 0.0 || !input.local.is_finite() {
        return false;
    }
    let mut moved = false;
    let (right, _, fwd) = ed.scene.camera.axes();
    let dir = fwd * input.local.z + right * input.local.x + Vec3::Y * input.local.y;
    let delta = dir.normalize_or_zero() * ed.fly.speed * dt;
    let target = Vec3::from(ed.orbit.target) + delta;
    if delta != Vec3::ZERO && target.is_finite() {
        ed.orbit.target = target.to_array();
        ed.orbit.apply(&mut ed.scene.camera);
        moved = true;
    }
    if input.roll != 0.0 && input.roll.is_finite() {
        let roll = if ed.scene.camera.roll.is_finite() { ed.scene.camera.roll } else { 0.0 };
        ed.scene.camera.roll = wrap(roll + input.roll.signum() * ROLL_SPEED * dt);
        moved = true;
    }
    if moved {
        ed.tween = None;
        changed(ed);
    }
    moved
}

/// Slot `slot` (0–8) keeps the camera as it is now.
pub(crate) fn save_view(ed: &mut Scene3dEditor, slot: usize) {
    let camera = ed.scene.camera.clone();
    ed.scene.save_view(slot, &camera);
}

/// Glides the camera to the view in `slot` (0–8). An empty slot does nothing but say so.
pub(crate) fn recall_view(ed: &mut Scene3dEditor, slot: usize) {
    let usable = |c: &&Camera| c.position.iter().chain(&c.target).all(|v| v.is_finite());
    match ed.scene.view(slot).filter(usable).cloned() {
        Some(to) => ed.tween = Some(ViewTween { from: ed.scene.camera.clone(), to, t: 0.0 }),
        None => ed.status = format!("{} {}", tl!("Nothing saved in view"), slot + 1),
    }
}

fn distance(c: &Camera) -> f32 {
    let d = (Vec3::from(c.target) - Vec3::from(c.position)).length();
    if d.is_finite() && d > 1e-4 { d } else { 1.0 }
}

/// The camera `s` (0–1) of the way from `a` to `b`: the position, distance, field of view and bank
/// change evenly and the view direction turns by the shortest way.
fn blend(a: &Camera, b: &Camera, s: f32) -> Camera {
    let lerp = |x: f32, y: f32| if x.is_finite() && y.is_finite() { x + (y - x) * s } else { y };
    let pos = Vec3::from(a.position).lerp(Vec3::from(b.position), s);
    let (fa, fb) = (a.axes().2, b.axes().2);
    let turn = Quat::IDENTITY.slerp(Quat::from_rotation_arc(fa, fb), s);
    let fwd = (turn * fa).normalize_or_zero();
    let fwd = if fwd == Vec3::ZERO { fb } else { fwd };
    let (ra, rb) = (if a.roll.is_finite() { a.roll } else { 0.0 }, if b.roll.is_finite() { b.roll } else { 0.0 });
    Camera {
        position: pos.to_array(),
        target: (pos + fwd * lerp(distance(a), distance(b))).to_array(),
        fov: lerp(a.fov, b.fov),
        near: lerp(a.near, b.near),
        far: lerp(a.far, b.far),
        roll: wrap(ra + wrap(rb - ra) * s),
    }
}

/// Moves a running glide on by `dt` seconds. True while there is one.
pub(crate) fn advance_tween(ed: &mut Scene3dEditor, dt: f32) -> bool {
    let Some(mut t) = ed.tween.take() else { return false };
    t.t += if dt.is_finite() { dt.max(0.0) / VIEW_SECONDS } else { 1.0 };
    let running = t.t < 1.0;
    ed.scene.camera = if running {
        let s = t.t * t.t * (3.0 - 2.0 * t.t);
        blend(&t.from, &t.to, s)
    } else {
        t.to.clone()
    };
    ed.orbit = Orbit::from_camera(&ed.scene.camera);
    if running {
        ed.tween = Some(t);
    }
    changed(ed);
    running
}

/// Once per frame while the mode is open: the held keys fly the camera, and a glide goes on.
pub(crate) fn frame(ed: &mut Scene3dEditor, ctx: &egui::Context) {
    let dt = ctx.input(|i| i.stable_dt);
    let dt = if dt.is_finite() { dt.clamp(0.0, 0.1) } else { 0.0 };
    if ed.fly.active && !ctx.text_edit_focused() {
        let input = ctx.input(|i| {
            let down = |k: Key| i.key_down(k) && !i.modifiers.command && !i.modifiers.alt;
            let axis = |plus: Key, minus: Key| f32::from(u8::from(down(plus))) - f32::from(u8::from(down(minus)));
            // Shift+C and Shift+V bank by a fixed step instead (see `keys`).
            let roll = if i.modifiers.shift { 0.0 } else { axis(Key::V, Key::C) };
            FlyInput { local: Vec3::new(axis(Key::D, Key::A), axis(Key::Z, Key::X), axis(Key::W, Key::S)), roll }
        });
        if input != FlyInput::default() {
            step(ed, input, dt);
            ctx.request_repaint();
        }
    }
    if advance_tween(ed, dt) {
        ctx.request_repaint();
    }
}

/// Takes one key press (not a release) out of this frame's events, so nothing else sees it.
fn take_press(ctx: &egui::Context, key: Key, ok: impl Fn(Modifiers) -> bool) -> Option<(Modifiers, bool)> {
    ctx.input_mut(|i| {
        let at = i.events.iter().position(|e| matches!(e, Event::Key { key: k, pressed: true, modifiers, .. } if *k == key && ok(*modifiers)))?;
        match i.events.remove(at) {
            Event::Key { modifiers, repeat, .. } => Some((modifiers, repeat)),
            _ => None,
        }
    })
}

/// The keys of this module: Tab, the number keys, and, while flying, W A S D Z X C V (taken here
/// so the tools those letters pick elsewhere are not switched). True when a key was used.
pub(crate) fn keys(ed: &mut Scene3dEditor, ctx: &egui::Context) -> bool {
    let mut used = false;
    if take_press(ctx, Key::Tab, |m| !m.command && !m.alt && !m.shift).is_some() {
        ed.fly.active = !ed.fly.active;
        used = true;
    }
    for (slot, key) in NUMBER_KEYS.iter().enumerate() {
        while let Some((m, repeat)) = take_press(ctx, *key, |m| !m.alt) {
            used = true;
            if repeat {
                continue;
            }
            if m.command {
                save_view(ed, slot);
            } else {
                recall_view(ed, slot);
            }
        }
    }
    if ed.fly.active {
        for key in [Key::W, Key::A, Key::S, Key::D, Key::Z, Key::X] {
            while take_press(ctx, key, |m| !m.command && !m.alt).is_some() {
                used = true;
            }
        }
        for (key, dir) in [(Key::C, -1.0), (Key::V, 1.0)] {
            while let Some((m, repeat)) = take_press(ctx, key, |m| !m.command && !m.alt) {
                used = true;
                if m.shift && !repeat {
                    let roll = ed.scene.camera.roll;
                    set_roll(ed, roll_step(roll, dir));
                    ed.tween = None;
                }
            }
        }
    }
    used
}

/// The panel block: the fly switch and speed, and the nine view slots.
pub(crate) fn panel(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    ui.horizontal_wrapped(|ui| {
        ui.toggle_value(&mut ed.fly.active, tl!("✈ Fly (Tab)")).on_hover_text(tl!("W A S D move, Z X up and down, C V bank (Shift: 15° at a time), drag to look, wheel for speed."));
        ui.add(egui::DragValue::new(&mut ed.fly.speed).range(SPEED_MIN..=SPEED_MAX).speed(0.05).max_decimals(2).suffix(" /s"));
        if ui.add_enabled(ed.scene.camera.roll != 0.0, egui::Button::new(tl!("Level"))).on_hover_text(tl!("Turn the camera upright again.")).clicked() {
            set_roll(ed, 0.0);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tl!("Views"));
        for slot in 0..MAX_VIEWS {
            let filled = ed.scene.view(slot).is_some();
            let b = ui.add(egui::Button::new((slot + 1).to_string()).selected(filled));
            let b = b.on_hover_text(tl!("Click: go to this view. Shift+click (or Ctrl+number): save the camera here. Right-click: clear."));
            if b.clicked() {
                if ui.input(|i| i.modifiers.shift) {
                    save_view(ed, slot);
                } else {
                    recall_view(ed, slot);
                }
            }
            if b.secondary_clicked() {
                ed.scene.clear_view(slot);
            }
        }
    });
}

#[cfg(test)]
#[path = "scene3d_fly_tests.rs"]
mod tests;
