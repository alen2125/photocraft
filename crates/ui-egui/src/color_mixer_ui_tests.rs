use egui::ecolor::Hsva;
use egui::pos2;

use super::*;
use crate::panels::hsva_srgb;

#[test]
fn mix_steps_run_from_the_first_colour_to_the_second() {
    let a = [1.0, 0.0, 0.0, 1.0];
    let b = [0.0, 0.0, 1.0, 1.0];
    let m = mix_steps(a, b, 5);
    assert_eq!(m.len(), 5);
    assert_eq!(m[0], a);
    assert_eq!(m[4], b);
    assert!((m[2][0] - 0.5).abs() < 1e-6 && (m[2][2] - 0.5).abs() < 1e-6);
}

#[test]
fn mix_steps_clamp_the_count_to_3_through_29() {
    let (a, b) = ([0.0; 4], [1.0; 4]);
    assert_eq!(mix_steps(a, b, 0).len(), 3);
    assert_eq!(mix_steps(a, b, 2).len(), 3);
    assert_eq!(mix_steps(a, b, 29).len(), 29);
    assert_eq!(mix_steps(a, b, 500).len(), 29);
}

#[test]
fn mix_of_one_colour_is_that_colour() {
    let a = [0.2, 0.4, 0.6, 1.0];
    assert!(mix_steps(a, a, 9).iter().all(|c| *c == a));
}

#[test]
fn locked_brightness_keeps_the_luminance_across_hues() {
    // A dark, moderately saturated colour: every hue can reach its luminance.
    let start = hsva_srgb(Hsva::new(0.1, 0.5, 0.4, 1.0));
    let target = luma(start);
    for k in 0..36 {
        let h = k as f32 / 36.0;
        assert!(luma(hsva_srgb(Hsva::new(h, 0.5, 1.0, 1.0))) > target, "hue {h} can reach it");
        let v = v_for_luma(h, 0.5, target);
        let c = hsva_srgb(Hsva::new(h, 0.5, v, 1.0));
        assert!((luma(c) - target).abs() < 0.005, "hue {h}: {} vs {target}", luma(c));
    }
}

#[test]
fn unreachable_luminance_gives_the_nearest_brightness() {
    // Saturated blue can't be as light as white, and nothing is darker than black.
    assert!(v_for_luma(0.667, 1.0, 0.9) > 0.99);
    assert!(v_for_luma(0.667, 1.0, -1.0) < 0.001);
}

#[test]
fn triangle_corners_map_to_pure_white_and_black() {
    let v = tri_vertices(pos2(100.0, 100.0), 50.0, 0.0);
    let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
    assert!(near(sv_from_point(v, v[0]).unwrap_or_default(), (1.0, 1.0)));
    assert!(near(sv_from_point(v, v[1]).unwrap_or_default(), (0.0, 1.0)));
    assert!(near(sv_from_point(v, v[2]).unwrap_or_default(), (0.0, 0.0)));
}

#[test]
fn triangle_point_and_sv_are_inverses() {
    let v = tri_vertices(pos2(80.0, 90.0), 60.0, 0.3);
    for (s, val) in [(0.2, 0.9), (0.7, 0.4), (1.0, 1.0), (0.5, 0.5)] {
        let p = point_from_sv(v, s, val);
        let (s2, v2) = sv_from_point(v, p).unwrap_or_default();
        assert!((s - s2).abs() < 1e-3 && (val - v2).abs() < 1e-3, "{s},{val} -> {s2},{v2}");
    }
}

#[test]
fn a_point_outside_the_triangle_is_pulled_in() {
    let v = tri_vertices(pos2(0.0, 0.0), 50.0, 0.0);
    let (s, val) = sv_from_point(v, pos2(1000.0, 1000.0)).unwrap_or_default();
    assert!((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&val));
}

#[test]
fn a_degenerate_triangle_gives_nothing() {
    let v = tri_vertices(pos2(5.0, 5.0), 0.0, 0.2);
    assert!(sv_from_point(v, pos2(5.0, 5.0)).is_none());
}

#[test]
fn older_settings_without_the_new_fields_load() {
    let s: crate::state::ColorPanelState = serde_json::from_str(r#"{"background":true}"#).unwrap_or_default();
    assert!(s.background && !s.lock_brightness && !s.wheel && s.mixer_steps == 0);
}
