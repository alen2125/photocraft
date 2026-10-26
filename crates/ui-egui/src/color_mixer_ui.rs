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
pub const MIXER_HEIGHT: f32 = 58.0;
const MIN_STEPS: usize = 3;
const MAX_STEPS: usize = 29;
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
fn set_active(app: &mut PhotocraftApp, ui: &egui::Ui, c: [f32; 4]) {
    if app.ui.color_panel.background {
        app.session.tools.background = c;
    } else {
        app.session.tools.foreground = c;
        crate::type_tool::foreground_changed(app);
        crate::shape_fill_ui::foreground_changed(app, crate::shape_fill_ui::gesture(ui));
    }
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

/// The mixer row under the colour field: step buttons, the Lock brightness and Wheel toggles, and
/// the swatches from foreground to background. Clicking one makes it the edited colour.
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let n = steps(app);
        if ui.add_enabled(n > MIN_STEPS, egui::Button::new("−").small()).clicked() {
            app.ui.color_panel.mixer_steps = n - 1;
        }
        ui.label(egui::RichText::new(n.to_string()).font(crate::theme::mono(11.0)).color(t.text_faint));
        if ui.add_enabled(n < MAX_STEPS, egui::Button::new("+").small()).clicked() {
            app.ui.color_panel.mixer_steps = n + 1;
        }
        let lock = app.ui.color_panel.lock_brightness;
        if ui.add(egui::Button::new(tl!("Lock brightness")).small().selected(lock)).clicked() {
            app.ui.color_panel.lock_brightness = !lock;
        }
        let wheel = app.ui.color_panel.wheel;
        if ui.add(egui::Button::new(tl!("Wheel")).small().selected(wheel)).clicked() {
            app.ui.color_panel.wheel = !wheel;
        }
    });
    ui.add_space(4.0);
    let colors = mix_steps(app.session.tools.foreground, app.session.tools.background, steps(app));
    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
    let n = colors.len();
    for (i, c) in colors.iter().enumerate() {
        let x0 = row.left() + row.width() * i as f32 / n as f32;
        let x1 = row.left() + row.width() * (i + 1) as f32 / n as f32;
        // A hair of overlap, so neighbouring swatches leave no seam.
        let r = Rect::from_min_max(pos2(x0, row.top()), pos2((x1 + 0.5).min(row.right()), row.bottom()));
        ui.painter().rect_filled(r, 0.0, Color32::from_rgb(q(c[0]), q(c[1]), q(c[2])));
    }
    ui.painter().rect_stroke(row, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let i = (((p.x - row.left()) / row.width().max(1.0)) * n as f32) as usize;
        if let Some(c) = colors.get(i.min(n - 1)) {
            set_active(app, ui, [c[0], c[1], c[2], 1.0]);
        }
    }
}

fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

// ----------------------------------------------------------------------------- hue ring + triangle

/// The triangle's corners for hue `h` (0..1) in a circle of radius `r` about `c`: pure hue, white,
/// black, going counter-clockwise from the hue's direction on the ring.
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
    let verts = tri_vertices(c, tri_r, hsva.h);
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
    let knob = point_from_sv(tri_vertices(c, tri_r, hsva.h), hsva.s, hsva.v);
    for m in [ring_mid, knob] {
        ui.painter().circle_stroke(m, 5.0, Stroke::new(1.5, Color32::WHITE));
        ui.painter().circle_stroke(m, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
    }

    if changed {
        set_active(app, ui, hsva_srgb(hsva));
        ui.data_mut(|d| d.insert_temp(key, (hsva.h, hsva.s)));
    }
}

#[cfg(test)]
#[path = "color_mixer_ui_tests.rs"]
mod tests;
