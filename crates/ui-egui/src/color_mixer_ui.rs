//! Color panel extras (after Coolorus): a mixer row between the foreground and background colours,
//! a brightness lock for hue changes, and a hue ring + triangle view of the colour field.
//!
//! Everything lives here; `panels.rs` only calls in (see `MY_CHANGES.md`).

use egui::ecolor::Hsva;
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::PhotocraftApp;
use crate::panels::{hsva_color32, hsva_srgb};
use crate::theme::Tokens;

/// Height the mixer rows take under the colour field (the field gives it up in `color_field_height`).
pub const MIXER_HEIGHT: f32 = 140.0;
const BUTTON_H: f32 = 44.0;
const STRIP_H: f32 = 30.0;
pub(crate) const MIN_STEPS: usize = 3;
pub(crate) const MAX_STEPS: usize = 29;
const DEFAULT_STEPS: usize = 7;

/// The mixer's step count: 0 (never set) means the default.
fn steps(app: &PhotocraftApp) -> usize {
    match app.ui.color_panel.mixer_steps {
        0 => DEFAULT_STEPS,
        n => n.clamp(MIN_STEPS, MAX_STEPS),
    }
}

/// `n` colours from `a` to `b` inclusive, blended on the sRGB-encoded values like a gradient.
pub fn mix_steps(a: [f32; 4], b: [f32; 4], n: usize) -> Vec<[f32; 4]> {
    let n = n.clamp(MIN_STEPS, MAX_STEPS);
    (0..n)
        .map(|i| {
            let f = i as f32 / (n - 1) as f32;
            [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * f)
        })
        .collect()
}

/// Relative luminance (Rec. 709 weights on linear light) of an sRGB-encoded colour.
pub fn luma(c: [f32; 4]) -> f32 {
    let l = |v: f32| photocraft_color::convert::srgb_to_linear(v.clamp(0.0, 1.0));
    0.2126 * l(c[0]) + 0.7152 * l(c[1]) + 0.0722 * l(c[2])
}

/// The brightness (V) that gives hue `h` and saturation `s` the luminance `target`, or the closest
/// V that can be reached. Luminance rises with V, so this is a bisection.
pub fn v_for_luma(h: f32, s: f32, target: f32) -> f32 {
    let at = |v: f32| luma(hsva_srgb(Hsva::new(h, s, v, 1.0)));
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if at(mid) <= target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

fn active(app: &PhotocraftApp) -> [f32; 4] {
    if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground }
}

/// Sets the colour the panel edits, with the hooks `color_field` runs for the foreground.
fn set_active(app: &mut PhotocraftApp, gesture: Option<String>, c: [f32; 4]) {
    if app.ui.color_panel.background {
        app.session.tools.background = c;
    } else {
        app.session.tools.foreground = c;
        crate::type_tool::foreground_changed(app);
        crate::shape_fill_ui::foreground_changed(app, gesture);
    }
}

/// Picks a swatch of the mixer row. A fixed row remembers the colours as they are now, so that
/// picking from it is not mistaken for a change made elsewhere.
fn pick(app: &mut PhotocraftApp, gesture: Option<String>, c: [f32; 4]) {
    set_active(app, gesture, [c[0], c[1], c[2], 1.0]);
    app.ui.color_panel.mixer_seen = [app.session.tools.foreground, app.session.tools.background];
}

/// The Eyedropper just set the foreground or background. A fixed row with its Eyedropper button off
/// takes the new colours as already seen, so it keeps the swatches it had.
pub(crate) fn eyedropper_sampled(app: &mut PhotocraftApp) {
    let cp = &mut app.ui.color_panel;
    if cp.mixer_fixed && !cp.mixer_eyedropper {
        cp.mixer_seen = [app.session.tools.foreground, app.session.tools.background];
    }
}

/// The swatches of the mixer row, foreground to background. A fixed row keeps the colours it had
/// until the foreground or background changes by any other means (wheel, hex, eyedropper, swap).
fn strip(app: &mut PhotocraftApp) -> Vec<[f32; 4]> {
    let n = steps(app);
    let now = [app.session.tools.foreground, app.session.tools.background];
    let cp = &mut app.ui.color_panel;
    if !cp.mixer_fixed || cp.mixer_seen != now {
        cp.mixer_ends = now;
        cp.mixer_seen = now;
    }
    mix_steps(cp.mixer_ends[0], cp.mixer_ends[1], n)
}

/// The swatch nearest the colour being edited, and whether it is exactly that colour.
fn selected(app: &PhotocraftApp, colors: &[[f32; 4]]) -> Option<(usize, bool)> {
    let a = active(app);
    let dist = |c: &[f32; 4]| (0..3).map(|k| (c[k] - a[k]).powi(2)).sum::<f32>();
    let (i, d) = colors.iter().enumerate().map(|(i, c)| (i, dist(c))).min_by(|x, y| x.1.total_cmp(&y.1))?;
    Some((i, d < 1e-6))
}

/// Moves the selection along the mixer row by `delta` steps (towards the background when positive),
/// stopping at the ends, and says where it is in the status bar. For the keys the user gives it.
fn step(app: &mut PhotocraftApp, delta: isize) {
    let colors = strip(app);
    let Some((i, _)) = selected(app, &colors) else { return };
    let last = colors.len().saturating_sub(1);
    let to = i.saturating_add_signed(delta).min(last);
    if let Some(c) = colors.get(to) {
        pick(app, None, *c);
    }
    let template = "Color Mixer: step {i} of {n}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("i", &(to + 1).to_string()), ("n", &colors.len().to_string())]);
    app.ui.status_error = false;
}

const COMMANDS: &[&str] = &["ui.colorMixer.next", "ui.colorMixer.prev"];

/// Is `id` one of the mixer's step commands? They have no key by default (Edit › Keyboard Shortcuts).
pub fn handles(id: &str) -> bool {
    COMMANDS.contains(&id)
}

/// Step the mixer selection from a key. `None` for any other command.
pub fn invoke(app: &mut PhotocraftApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    let delta = match id {
        "ui.colorMixer.next" => 1,
        "ui.colorMixer.prev" => -1,
        _ => return None,
    };
    step(app, delta);
    Some(Ok(serde_json::Value::Null))
}

/// With Lock brightness on, a hue change keeps the luminance the colour had when the gesture began
/// (`fresh`: its first frame, or a plain click); otherwise `hsva` comes back unchanged.
pub(crate) fn apply_lock(app: &PhotocraftApp, ui: &egui::Ui, mut hsva: Hsva, fresh: bool) -> Hsva {
    if !app.ui.color_panel.lock_brightness {
        return hsva;
    }
    let key = egui::Id::new("color-lock-luma");
    let target = match ui.data(|d| d.get_temp::<f32>(key)) {
        Some(t) if !fresh => t,
        _ => {
            let t = luma(active(app));
            ui.data_mut(|d| d.insert_temp(key, t));
            t
        }
    };
    hsva.v = v_for_luma(hsva.h, hsva.s, target);
    hsva
}

/// What the rows under the colour field took last frame, so the field leaves room for them and the
/// tab fits its group without scrolling. The default is a guess for the first frame.
pub(crate) fn reserved_height(ui: &egui::Ui) -> f32 {
    ui.data(|d| d.get_temp::<f32>(egui::Id::new("color-below-field"))).unwrap_or(MIXER_HEIGHT)
}

/// Remember how tall everything below the field was (`top`: where it started).
pub(crate) fn remember_height(ui: &egui::Ui, top: f32) {
    let h = (ui.cursor().top() - top).max(0.0);
    let key = egui::Id::new("color-below-field");
    if ui.data(|d| d.get_temp::<f32>(key)).is_none_or(|old| (old - h).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(key, h));
        ui.ctx().request_repaint();
    }
}

/// A toggle or step button, big enough for a stylus.
fn big_button(ui: &mut egui::Ui, label: &str, min_w: f32, on: bool, enabled: bool) -> bool {
    let b = egui::Button::new(egui::RichText::new(label).size(14.0)).min_size(vec2(min_w, BUTTON_H)).selected(on);
    ui.add_enabled(enabled, b).clicked()
}

/// The mixer row under the colour field: step buttons, the Lock brightness, Change colour and Wheel
/// toggles, and the swatches from foreground to background. Clicking one makes it the edited colour.
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        let n = steps(app);
        if big_button(ui, "−", BUTTON_H, false, n > MIN_STEPS) {
            app.ui.color_panel.mixer_steps = n - 1;
        }
        ui.label(egui::RichText::new(n.to_string()).font(crate::theme::mono(13.0)).color(t.text_faint));
        if big_button(ui, "+", BUTTON_H, false, n < MAX_STEPS) {
            app.ui.color_panel.mixer_steps = n + 1;
        }
        let cp = &mut app.ui.color_panel;
        if big_button(ui, tl!("Lock brightness"), 0.0, cp.lock_brightness, true) {
            cp.lock_brightness = !cp.lock_brightness;
        }
        // On: the row follows the colours. Off: it stays put until the colour changes elsewhere.
        if big_button(ui, tl!("Change color"), 0.0, !cp.mixer_fixed, true) {
            cp.mixer_fixed = !cp.mixer_fixed;
        }
        // Only a fixed row has a say: a following one rebuilds from whatever the colours are.
        if big_button(ui, tl!("Eyedropper"), 0.0, cp.mixer_eyedropper, cp.mixer_fixed) {
            cp.mixer_eyedropper = !cp.mixer_eyedropper;
        }
        if big_button(ui, tl!("Wheel"), 0.0, cp.wheel, true) {
            cp.wheel = !cp.wheel;
        }
        // HSB/RGB sliders: the small menu button, with the rest of the row.
        crate::panels::color_slider_mode_row(app, ui);
    });
    ui.add_space(4.0);
    let colors = strip(app);
    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), STRIP_H), Sense::click());
    let n = colors.len();
    let cell = |i: usize| row.left() + row.width() * i as f32 / n.max(1) as f32;
    for (i, c) in colors.iter().enumerate() {
        // A hair of overlap, so neighbouring swatches leave no seam.
        let r = Rect::from_min_max(pos2(cell(i), row.top()), pos2((cell(i + 1) + 0.5).min(row.right()), row.bottom()));
        ui.painter().rect_filled(r, 0.0, Color32::from_rgb(q(c[0]), q(c[1]), q(c[2])));
    }
    ui.painter().rect_stroke(row, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
    if let Some((i, true)) = selected(app, &colors) {
        let r = Rect::from_min_max(pos2(cell(i), row.top()), pos2(cell(i + 1), row.bottom())).shrink(1.0);
        ui.painter().rect_stroke(r, 0.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        ui.painter().rect_stroke(r.shrink(2.0), 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Inside);
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let i = (((p.x - row.left()) / row.width().max(1.0)) * n as f32) as usize;
        if let Some(c) = colors.get(i.min(n.saturating_sub(1))) {
            pick(app, crate::shape_fill_ui::gesture(ui), *c);
        }
    }
}

fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

// ----------------------------------------------------------------------------- hue ring + triangle

/// The triangle's corners for hue `h` (0..1) in a circle of radius `r` about `c`: pure hue, white,
/// black, going counter-clockwise from the direction `h` (a turn, 0 = right).
pub fn tri_vertices(c: egui::Pos2, r: f32, h: f32) -> [egui::Pos2; 3] {
    let at = |turn: f32| {
        let a = (h + turn) * std::f32::consts::TAU;
        pos2(c.x + r * a.cos(), c.y - r * a.sin())
    };
    [at(0.0), at(1.0 / 3.0), at(2.0 / 3.0)]
}

/// Saturation and brightness of the point `p` in the triangle `v` (pure, white, black): the point
/// is pulled inside first. `None` for a degenerate triangle.
pub fn sv_from_point(v: [egui::Pos2; 3], p: egui::Pos2) -> Option<(f32, f32)> {
    let [a, b, c] = v;
    let det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if det.abs() < 1e-6 {
        return None;
    }
    let wa = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / det;
    let wb = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / det;
    let wc = 1.0 - wa - wb;
    let (wa, wb, wc) = (wa.max(0.0), wb.max(0.0), wc.max(0.0));
    let sum = wa + wb + wc;
    if sum <= 0.0 {
        return None;
    }
    let (wa, wb) = (wa / sum, wb / sum);
    // pure = wa, white = wb, black = 1 - v.
    let value = (wa + wb).clamp(0.0, 1.0);
    let sat = if value > 1e-6 { (wa / value).clamp(0.0, 1.0) } else { 0.0 };
    Some((sat, value))
}

/// Where saturation `s` and brightness `value` sit in the triangle `v`.
pub fn point_from_sv(v: [egui::Pos2; 3], s: f32, value: f32) -> egui::Pos2 {
    let wa = s * value;
    let wb = value * (1.0 - s);
    let wc = 1.0 - value;
    pos2(wa * v[0].x + wb * v[1].x + wc * v[2].x, wa * v[0].y + wb * v[1].y + wc * v[2].y)
}

#[derive(Clone, Copy, PartialEq)]
enum Zone {
    None,
    Ring,
    Triangle,
}

/// The wheel view of the colour field, in place of the square and hue strip: a hue ring with the
/// saturation/brightness triangle inside. `hsva` is the colour as `color_field` worked it out;
/// `key` is its remembered (hue, saturation) slot.
pub(crate) fn wheel_field(app: &mut PhotocraftApp, ui: &mut egui::Ui, size: Vec2, hsva: Hsva, key: egui::Id) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let c = rect.center();
    let outer = (rect.width().min(rect.height()) * 0.5 - 2.0).max(0.0);
    let ring_w = (outer * 0.16).clamp(6.0, 22.0).min(outer);
    let inner = outer - ring_w;
    let tri_r = (inner - 4.0).max(0.0);
    let dir = |h: f32, r: f32| {
        let a = h * std::f32::consts::TAU;
        pos2(c.x + r * a.cos(), c.y - r * a.sin())
    };

    // Ring.
    let n = 96;
    let mut ring = egui::Mesh::default();
    for k in 0..=n {
        let h = k as f32 / n as f32;
        let col = hsva_color32(Hsva::new(h.min(0.9999), 1.0, 1.0, 1.0));
        ring.colored_vertex(dir(h, outer), col);
        ring.colored_vertex(dir(h, inner), col);
    }
    for k in 0..n {
        let a = (k * 2) as u32;
        ring.add_triangle(a, a + 1, a + 3);
        ring.add_triangle(a, a + 3, a + 2);
    }
    ui.painter().add(ring);

    // Triangle: pure hue, white, black corners; the GPU blend is the saturation/brightness field.
    // The triangle stays put (pure hue at the right) while the ring marker turns, as in Coolorus.
    let verts = tri_vertices(c, tri_r, 0.0);
    let mut tri = egui::Mesh::default();
    tri.colored_vertex(verts[0], hsva_color32(Hsva::new(hsva.h, 1.0, 1.0, 1.0)));
    tri.colored_vertex(verts[1], Color32::WHITE);
    tri.colored_vertex(verts[2], Color32::BLACK);
    tri.add_triangle(0, 1, 2);
    ui.painter().add(tri);
    ui.painter().add(egui::Shape::closed_line(verts.to_vec(), Stroke::new(1.0, t.separator)));

    // Input: where a press starts decides whether it drags the ring or the triangle.
    let zone_key = egui::Id::new("color-wheel-zone");
    if resp.drag_started() || resp.clicked() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        let zone = origin.map_or(Zone::None, |p| {
            let d = p.distance(c);
            if d > outer + 2.0 {
                Zone::None
            } else if d >= inner {
                Zone::Ring
            } else {
                Zone::Triangle
            }
        });
        ui.data_mut(|d| d.insert_temp(zone_key, zone as u8));
    }
    let zone = match ui.data(|d| d.get_temp::<u8>(zone_key)) {
        Some(1) => Zone::Ring,
        Some(2) => Zone::Triangle,
        _ => Zone::None,
    };
    let mut hsva = hsva;
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        match zone {
            Zone::Ring => {
                let h = (-(p.y - c.y)).atan2(p.x - c.x) / std::f32::consts::TAU;
                hsva.h = h.rem_euclid(1.0).min(0.9999);
                hsva = apply_lock(app, ui, hsva, resp.drag_started() || !resp.dragged());
                changed = true;
            }
            Zone::Triangle => {
                if let Some((s, v)) = sv_from_point(verts, p) {
                    hsva.s = s;
                    hsva.v = v;
                    changed = true;
                }
            }
            Zone::None => {}
        }
    }

    // Markers.
    let ring_mid = dir(hsva.h, (outer + inner) * 0.5);
    let knob = point_from_sv(verts, hsva.s, hsva.v);
    for m in [ring_mid, knob] {
        ui.painter().circle_stroke(m, 5.0, Stroke::new(1.5, Color32::WHITE));
        ui.painter().circle_stroke(m, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
    }

    if changed {
        set_active(app, crate::shape_fill_ui::gesture(ui), hsva_srgb(hsva));
        ui.data_mut(|d| d.insert_temp(key, (hsva.h, hsva.s)));
    }
}

#[cfg(test)]
#[path = "color_mixer_ui_tests.rs"]
mod tests;
