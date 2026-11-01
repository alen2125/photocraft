//! Sliders under the colour wheel (after Coolorus): RGB, HSV, Lab, CMYK or B/W, with the row of
//! modes that picks the scheme. `panels.rs` only calls `panel` (see `MY_CHANGES.md`).

use egui::ecolor::Hsva;
use egui::{Color32, RichText};
use photocraft_color::convert::{cmyk_to_rgb, lab_to_srgb, rgb_to_cmyk, rgb_to_gray, srgb_to_lab, with_cmyk_space};

use crate::PhotocraftApp;
use crate::color_mixer_ui::{active, set_active};
use crate::panels::{hsva_color32, hsva_srgb, srgb_hsva};
use crate::state::ColorPanelSliderMode as Mode;
use crate::theme::Tokens;
use crate::widgets;

const MODES: [(Mode, &str); 5] = [(Mode::Rgb, "RGB"), (Mode::Hsb, "HSV"), (Mode::Lab, "LAB"), (Mode::Cmyk, "CMYK"), (Mode::Bw, "B/W")];

fn bytes(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

fn col32(c: [f32; 3]) -> Color32 {
    let [r, g, b] = bytes(c);
    Color32::from_rgb(r, g, b)
}

fn rgb_of(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]]
}

/// `n` evenly spaced stops from `f(0)` to `f(1)`, for a slider track a colour model bends.
fn stops(n: usize, f: impl Fn(f32) -> [f32; 3]) -> Vec<Color32> {
    let last = n.saturating_sub(1).max(1) as f32;
    (0..n).map(|i| col32(f(i as f32 / last))).collect()
}

/// What the sliders last showed, while it still gives the colour: a grey has no hue, and a colour
/// out of the CMYK or Lab range moves when it comes back, so a drag would otherwise fight itself.
fn remembered(ui: &egui::Ui, key: egui::Id, current: [f32; 3], back: impl Fn([f32; 4]) -> [f32; 3]) -> Option<[f32; 4]> {
    let stored: [f32; 4] = ui.data(|d| d.get_temp(key))?;
    (bytes(back(stored)) == bytes(current)).then_some(stored)
}

/// The slider tracks for the scheme in `mode`, and the colour an edit gives (`None`: no edit).
fn sliders(ui: &mut egui::Ui, mode: Mode, cur: [f32; 4], background: bool, dynamic: bool) -> Option<[f32; 4]> {
    let rgb = rgb_of(cur);
    let key = egui::Id::new(("color-wheel-sliders", background, mode as u8));
    let track = |s: &[Color32]| dynamic.then(|| s.to_vec());
    let mut edited: Option<[f32; 3]> = None;
    match mode {
        Mode::Rgb => {
            let mut v = bytes(rgb).map(f32::from);
            let [r, g, b] = bytes(rgb);
            let mut changed = false;
            let rows: [(&str, [Color32; 2]); 3] = [
                ("Red", [Color32::from_rgb(0, g, b), Color32::from_rgb(255, g, b)]),
                ("Green", [Color32::from_rgb(r, 0, b), Color32::from_rgb(r, 255, b)]),
                ("Blue", [Color32::from_rgb(r, g, 0), Color32::from_rgb(r, g, 255)]),
            ];
            for (i, (label, s)) in rows.iter().enumerate() {
                if let Some(x) = v.get_mut(i) {
                    changed |= widgets::slider_row(ui, label, x, 0.0..=255.0, "", track(s).as_deref()).changed();
                }
            }
            if changed {
                edited = Some(v.map(|x| x.clamp(0.0, 255.0).round() / 255.0));
            }
        }
        Mode::Hsb => {
            let h0 = remembered(ui, key, rgb, |s| rgb_of(hsva_srgb(Hsva::new(s[0], s[1], s[2], 1.0))))
                .map_or_else(|| srgb_hsva(cur), |s| Hsva::new(s[0], s[1], s[2], 1.0));
            let (mut h, mut s, mut v) = (h0.h * 360.0, h0.s * 100.0, h0.v * 100.0);
            let hue = widgets::hue_stops();
            let sat = [Hsva::new(h0.h, 0.0, h0.v.max(0.2), 1.0), Hsva::new(h0.h, 1.0, h0.v.max(0.2), 1.0)].map(hsva_color32);
            let val = [Color32::BLACK, hsva_color32(Hsva::new(h0.h, h0.s, 1.0, 1.0))];
            let mut changed = widgets::slider_row(ui, "Hue", &mut h, 0.0..=360.0, "°", dynamic.then_some(&hue[..])).changed();
            changed |= widgets::slider_row(ui, "Saturation", &mut s, 0.0..=100.0, "%", track(&sat).as_deref()).changed();
            changed |= widgets::slider_row(ui, "Brightness", &mut v, 0.0..=100.0, "%", track(&val).as_deref()).changed();
            if changed {
                let n = Hsva::new((h / 360.0).clamp(0.0, 0.9999), (s / 100.0).clamp(0.0, 1.0), (v / 100.0).clamp(0.0, 1.0), 1.0);
                ui.data_mut(|d| {
                    d.insert_temp(key, [n.h, n.s, n.v, 0.0]);
                    // The wheel keeps the hue and saturation it last showed for greys and black.
                    d.insert_temp(egui::Id::new(("color-field-hue", background)), (n.h, n.s));
                });
                edited = Some(rgb_of(hsva_srgb(n)));
            }
        }
        Mode::Lab => {
            let lab = remembered(ui, key, rgb, |s| lab_to_srgb([s[0], s[1], s[2]])).map_or_else(|| srgb_to_lab(rgb), |s| [s[0], s[1], s[2]]);
            let mut v = lab;
            let l_stops = stops(5, |f| lab_to_srgb([f * 100.0, lab[1], lab[2]]));
            let a_stops = stops(9, |f| lab_to_srgb([lab[0], -128.0 + f * 255.0, lab[2]]));
            let b_stops = stops(9, |f| lab_to_srgb([lab[0], lab[1], -128.0 + f * 255.0]));
            let mut changed = widgets::slider_row(ui, "L", &mut v[0], 0.0..=100.0, "", track(&l_stops).as_deref()).changed();
            changed |= widgets::slider_row(ui, "a", &mut v[1], -128.0..=127.0, "", track(&a_stops).as_deref()).changed();
            changed |= widgets::slider_row(ui, "b", &mut v[2], -128.0..=127.0, "", track(&b_stops).as_deref()).changed();
            if changed {
                ui.data_mut(|d| d.insert_temp(key, [v[0], v[1], v[2], 0.0]));
                edited = Some(lab_to_srgb(v));
            }
        }
        Mode::Cmyk => {
            let k0 = remembered(ui, key, rgb, cmyk_to_rgb).unwrap_or_else(|| rgb_to_cmyk(rgb));
            let mut v = k0.map(|x| x * 100.0);
            let mut changed = false;
            for (i, label) in ["C", "M", "Y", "K"].into_iter().enumerate() {
                let s = stops(5, |f| {
                    let mut c = k0;
                    if let Some(x) = c.get_mut(i) {
                        *x = f;
                    }
                    cmyk_to_rgb(c)
                });
                if let Some(x) = v.get_mut(i) {
                    changed |= widgets::slider_row(ui, label, x, 0.0..=100.0, "%", track(&s).as_deref()).changed();
                }
            }
            if changed {
                let n = v.map(|x| (x / 100.0).clamp(0.0, 1.0));
                ui.data_mut(|d| d.insert_temp(key, n));
                edited = Some(cmyk_to_rgb(n).map(|x| x.clamp(0.0, 1.0)));
            }
        }
        Mode::Bw => {
            let mut k = (1.0 - rgb_to_gray(rgb)).clamp(0.0, 1.0) * 100.0;
            let s = [Color32::WHITE, Color32::BLACK];
            if widgets::slider_row(ui, "K", &mut k, 0.0..=100.0, "%", track(&s).as_deref()).changed() {
                let g = 1.0 - (k / 100.0).clamp(0.0, 1.0);
                edited = Some([g, g, g]);
            }
        }
    }
    edited.map(|c| [c[0], c[1], c[2], 1.0])
}

/// The sliders of the scheme chosen, with the row of schemes under them.
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(8.0);
    let bg = app.ui.color_panel.background;
    let mode = app.ui.color_panel.slider_mode;
    let dynamic = app.session.prefs().interface.dynamic_color_sliders;
    let cur = active(app);
    let space = app.session.active().and_then(|st| photocraft_compose::cmyk_space(&st.doc));
    if let Some(c) = with_cmyk_space(space.as_ref(), || sliders(ui, mode, cur, bg, dynamic)) {
        set_active(app, crate::shape_fill_ui::gesture(ui), c);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(tl!("Mode:")).size(11.0).color(t.text_faint));
        for (m, label) in MODES {
            let b = egui::Button::new(RichText::new(label).size(12.0)).selected(mode == m);
            if ui.add(b).clicked() {
                app.ui.color_panel.slider_mode = m;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_in_every_scheme() {
        let red = [1.0, 0.0, 0.0];
        let lab = srgb_to_lab(red);
        // D50, as Photoshop shows it: 54 / 81 / 70.
        assert!((lab[0] - 54.0).abs() < 1.0 && (lab[1] - 81.0).abs() < 1.5 && (lab[2] - 70.0).abs() < 1.5, "{lab:?}");
        let hsv = srgb_hsva([1.0, 0.0, 0.0, 1.0]);
        assert!(hsv.h.abs() < 1e-4 && (hsv.s - 1.0).abs() < 1e-4 && (hsv.v - 1.0).abs() < 1e-4);
        let cmyk = rgb_to_cmyk(red);
        assert!(cmyk[0] < 0.15 && cmyk[1] > 0.8 && cmyk[2] > 0.8 && cmyk[3] < 0.15, "{cmyk:?}");
    }

    #[test]
    fn every_scheme_draws_its_rows_and_the_mode_row_switches() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(300.0, 420.0))
            .build_ui_state(|ui, app: &mut PhotocraftApp| panel(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        for (mode, rows) in [
            (Mode::Rgb, &["Red", "Green", "Blue"][..]),
            (Mode::Hsb, &["Hue", "Saturation", "Brightness"][..]),
            (Mode::Lab, &["L", "a", "b"][..]),
            (Mode::Cmyk, &["C", "M", "Y", "K"][..]),
            (Mode::Bw, &["K"][..]),
        ] {
            h.state_mut().ui.color_panel.slider_mode = mode;
            h.run_steps(3);
            for label in rows {
                h.get_by_label(label);
            }
        }
        h.get_by_label("CMYK").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.color_panel.slider_mode, Mode::Cmyk);
    }

    #[test]
    fn stops_run_from_first_to_last() {
        let s = stops(5, |f| [f, f, f]);
        assert_eq!(s.len(), 5);
        assert_eq!(s.first().copied(), Some(Color32::BLACK));
        assert_eq!(s.last().copied(), Some(Color32::WHITE));
    }
}
