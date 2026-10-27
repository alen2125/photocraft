use photocraft_doc::LayerContent;
use serde_json::json;

use super::*;
use crate::canvas::{Drag, finish_gesture};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app
}

fn press(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    app.ui.tool = Tool::CustomShape;
    let d = Drag::new(Tool::CustomShape, start, vec![[start[0], start[1], 1.0], [end[0], end[1], 1.0]], egui::Modifiers::NONE, false);
    finish_gesture(app, d);
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

#[test]
fn a_click_opens_no_dialog_and_leaves_the_document_alone() {
    let mut app = app();
    let before = layers(&app);
    press(&mut app, [50.0, 50.0], [50.0, 50.0]);
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(layers(&app), before);
}

#[test]
fn a_click_places_the_default_size_when_one_is_set() {
    let mut app = app();
    app.ui.tool_options.custom_shape_default_size = 40.0;
    let before = layers(&app);
    press(&mut app, [100.0, 100.0], [100.0, 100.0]);
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(layers(&app), before + 1);
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(sh) = &l.content else { panic!("not a shape layer") };
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    assert!(x0 >= 79.9 && y0 >= 79.9 && x1 <= 120.1 && y1 <= 120.1, "{x0} {y0} {x1} {y1}");
}

#[test]
fn a_drag_from_the_pasteboard_places_the_shape_without_an_error() {
    let mut app = app();
    let before = layers(&app);
    press(&mut app, [-60.0, -40.0], [120.0, 90.0]);
    assert_eq!(layers(&app), before + 1);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn a_shape_wholly_outside_the_canvas_is_skipped_silently() {
    let mut app = app();
    let before = layers(&app);
    press(&mut app, [-200.0, -200.0], [-100.0, -100.0]);
    assert_eq!(layers(&app), before);
    assert!(app.ui.status.is_empty() && !app.ui.status_error);
    assert!(app.ui.dialogs.is_empty());
}

fn alpha_at(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

#[test]
fn pixels_mode_paints_the_active_pixel_layer_instead_of_making_a_shape_layer() {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.custom_shape_mode = "pixels".into();
    let before = layers(&app);
    press(&mut app, [100.0, 100.0], [220.0, 200.0]);
    assert_eq!(layers(&app), before, "no new layer: the active one is a pixel layer");
    assert!(app.ui.dialogs.is_empty());
    let painted = (100..220).any(|x| alpha_at(&app, x, 150) > 0.5);
    assert!(painted, "the shape was painted into the layer");
}

#[test]
fn pixels_mode_from_the_pasteboard_paints_the_part_on_the_canvas() {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.custom_shape_mode = "pixels".into();
    press(&mut app, [-80.0, -80.0], [120.0, 120.0]);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(alpha_at(&app, 40, 40) > 0.5);
}

#[test]
fn old_prefs_without_the_mode_load_as_shape() {
    let o: crate::state::ToolOptions = serde_json::from_value(json!({})).unwrap();
    assert_eq!(o.custom_shape_mode, "shape");
}

#[test]
fn the_shapes_panel_offers_load_replace_and_save() {
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::preset_panels::shapes_panel(app, ui);
            }
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    for label in ["Load Shapes…", "Replace Shapes…", "Save Shapes…"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
}

fn press_with(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    app.ui.tool = Tool::CustomShape;
    let d = Drag::new(Tool::CustomShape, start, vec![[start[0], start[1], 1.0], [end[0], end[1], 1.0]], mods, false);
    finish_gesture(app, d);
}

fn shape_bounds_of_active(app: &PhotocraftApp) -> (f64, f64, f64, f64) {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("not a shape layer") };
    sh.path.control_bounds().unwrap()
}

#[test]
fn leaf_mode_bends_the_shape_between_the_two_drag_points() {
    let mut app = app();
    app.ui.tool_options.custom_shape_fit = "leaf".into();
    let before = layers(&app);
    press(&mut app, [50.0, 100.0], [250.0, 100.0]);
    assert_eq!(layers(&app), before + 1);
    let (x0, y0, x1, y1) = shape_bounds_of_active(&app);
    assert!(x0 >= 49.9 && x1 <= 250.1, "{x0} {x1}: the leaf runs from A to B");
    assert!(y0 >= 100.0 - 50.1 && y1 <= 100.0 + 50.1, "{y0} {y1}: half the width 0.5 · 200 either side");
}

#[test]
fn leaf_mode_shift_snaps_the_axis_and_alt_draws_from_the_middle() {
    let mut app = app();
    app.ui.tool_options.custom_shape_fit = "leaf".into();
    let snapped = leaf_of_drag(&app, [100.0, 100.0], [200.0, 108.0], egui::Modifiers::SHIFT).unwrap();
    assert!((snapped.a.y - snapped.b.y).abs() < 1e-9, "AB snapped flat: {snapped:?}");
    let free = leaf_of_drag(&app, [100.0, 100.0], [200.0, 108.0], egui::Modifiers::NONE).unwrap();
    assert!((free.a.y - free.b.y).abs() > 1.0, "without ⇧ AB keeps its slope");
    let centred = leaf_of_drag(&app, [200.0, 150.0], [250.0, 150.0], egui::Modifiers::ALT).unwrap();
    assert_eq!((centred.a.x, centred.b.x), (150.0, 250.0), "A is B mirrored about the press point");
    // And the drag itself places a layer for each.
    let before = layers(&app);
    press_with(&mut app, [100.0, 100.0], [200.0, 108.0], egui::Modifiers::SHIFT);
    press_with(&mut app, [200.0, 150.0], [250.0, 150.0], egui::Modifiers::ALT);
    assert_eq!(layers(&app), before + 2);
}

#[test]
fn leaf_mode_click_places_nothing_and_a_leaf_off_the_canvas_is_skipped() {
    let mut app = app();
    app.ui.tool_options.custom_shape_fit = "leaf".into();
    let before = layers(&app);
    press(&mut app, [50.0, 50.0], [50.0, 50.0]);
    press(&mut app, [-300.0, -300.0], [-100.0, -250.0]);
    assert_eq!(layers(&app), before);
    assert!(app.ui.status.is_empty() && app.ui.dialogs.is_empty());
}

#[test]
fn leaf_mode_with_pixels_paints_the_leaf() {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.custom_shape_fit = "leaf".into();
    app.ui.tool_options.custom_shape_mode = "pixels".into();
    press(&mut app, [50.0, 100.0], [250.0, 100.0]);
    assert!(alpha_at(&app, 150, 100) > 0.5);
    assert_eq!(alpha_at(&app, 150, 20), 0.0);
}

#[test]
fn leaf_mode_preview_is_the_bent_shape_and_box_mode_has_none() {
    let mut app = app();
    let mods = egui::Modifiers::NONE;
    assert!(leaf_path(&app, Tool::CustomShape, [50.0, 100.0], [250.0, 100.0], mods).is_none());
    app.ui.tool_options.custom_shape_fit = "leaf".into();
    let p = leaf_path(&app, Tool::CustomShape, [50.0, 100.0], [250.0, 100.0], mods).unwrap();
    let (x0, _, x1, _) = p.control_bounds().unwrap();
    assert!(x0 >= 49.9 && x1 <= 250.1);
    assert!(leaf_path(&app, Tool::CustomShape, [50.0, 100.0], [50.0, 100.0], mods).is_none(), "a zero-length leaf has no preview");
    assert!(leaf_path(&app, Tool::Rectangle, [50.0, 100.0], [250.0, 100.0], mods).is_none());
}

#[test]
fn old_prefs_load_with_the_box_fit() {
    let o: crate::state::ToolOptions = serde_json::from_value(json!({})).unwrap();
    assert_eq!((o.custom_shape_fit.as_str(), o.leaf_width, o.leaf_fullness), ("box", 0.5, 1.0));
}

/// Drag a shape out and return the shape the tool has selected afterwards.
fn place_and_read(app: &mut PhotocraftApp) -> String {
    press(app, [20.0, 20.0], [120.0, 100.0]);
    app.ui.presets_ui.shape().to_string()
}

fn randomizing(every: u32, seed: u64) -> PhotocraftApp {
    let mut app = app();
    app.ui.tool_options.shape_random_every = every;
    app.ui.tool_options.shape_random_seed = Some(seed);
    app.ui.presets_ui.custom_shape = "Heart".into();
    app
}

#[test]
fn the_shape_changes_exactly_every_n_placements() {
    let mut app = randomizing(3, 7);
    let names: Vec<String> = (0..9).map(|_| place_and_read(&mut app)).collect();
    // Placements 1 and 2 keep the shape; the 3rd, 6th and 9th change it.
    assert_eq!(names[0], "Heart");
    assert_eq!(names[1], "Heart");
    assert_ne!(names[2], "Heart");
    assert_eq!(names[3], names[2]);
    assert_eq!(names[4], names[2]);
    assert_ne!(names[5], names[2]);
    assert_eq!(names[6], names[5]);
    assert_eq!(names[7], names[5]);
    assert_ne!(names[8], names[5]);
}

#[test]
fn n_of_1_changes_the_shape_every_time_and_0_never() {
    let mut app = randomizing(1, 11);
    let mut last = "Heart".to_string();
    for _ in 0..6 {
        let now = place_and_read(&mut app);
        assert_ne!(now, last);
        last = now;
    }
    let mut off = randomizing(0, 11);
    for _ in 0..4 {
        assert_eq!(place_and_read(&mut off), "Heart");
    }
}

#[test]
fn a_fixed_seed_repeats_the_sequence_and_another_seed_differs() {
    let run = |seed| {
        let mut app = randomizing(1, seed);
        (0..8).map(|_| place_and_read(&mut app)).collect::<Vec<_>>()
    };
    assert_eq!(run(5), run(5));
    assert_ne!(run(5), run(6));
}

#[test]
fn the_source_limits_the_pick_to_the_group_or_the_set() {
    let mut app = randomizing(1, 3);
    app.ui.tool_options.shape_random_source = "group".into();
    let index = photocraft_engine::shape_index::index(&app.session);
    let group = index.iter().find(|g| g.items.iter().any(|n| n == "Heart")).unwrap();
    for _ in 0..12 {
        let now = place_and_read(&mut app);
        assert!(group.items.contains(&now), "{now} is not in {}", group.name);
    }
    let mut app = randomizing(1, 3);
    app.ui.tool_options.shape_random_source = "set".into();
    app.ui.tool_options.shape_random_set = vec!["Star".into(), "Cloud".into(), "No such shape".into()];
    for _ in 0..12 {
        let now = place_and_read(&mut app);
        assert!(now == "Star" || now == "Cloud", "{now}");
    }
}

#[test]
fn an_empty_library_or_set_changes_nothing_and_does_not_panic() {
    let mut app = randomizing(1, 3);
    app.ui.tool_options.shape_random_source = "set".into();
    for _ in 0..3 {
        after_place(&mut app);
    }
    assert_eq!(app.ui.presets_ui.shape(), "Heart", "an empty set leaves the shape alone");
    app.ui.tool_options.shape_random_source = "library".into();
    app.session.presets.shapes.clear();
    for _ in 0..3 {
        after_place(&mut app);
    }
    assert_eq!(app.ui.presets_ui.shape(), "Heart");
    let mut rng = 1u64;
    assert_eq!(pick(0, None, &mut rng), None);
    assert_eq!(pick(1, Some(0), &mut rng), Some(0), "the only shape is picked even if it is the current one");
}

#[test]
fn a_placement_that_changes_nothing_does_not_count_and_the_count_is_not_a_saved_option() {
    let mut app = randomizing(2, 9);
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.custom_shape_mode = "pixels".into();
    press(&mut app, [-300.0, -300.0], [-200.0, -200.0]);
    press(&mut app, [-300.0, -300.0], [-200.0, -200.0]);
    assert_eq!(app.ui.presets_ui.shape(), "Heart", "shapes wholly off the canvas are not placed, so not counted");
    let before = app.ui.tool_options.clone();
    app.ui.tool_options.shape_random.placed = 1;
    assert_eq!(app.ui.tool_options, before, "the counter is not part of the options");
    let json = serde_json::to_value(&app.ui.tool_options).unwrap();
    assert!(json.get("shape_random").is_none());
}

#[test]
fn picking_from_a_big_lazy_library_parses_only_the_shapes_used() {
    let mut app = randomizing(1, 21);
    let shapes: Vec<(String, photocraft_psd::csh::CshShape)> = (0..5000)
        .map(|i| {
            let k = |x: f64, y: f64| photocraft_psd::path::PsdKnot { linked: false, pre: (x, y), anchor: (x, y), post: (x, y) };
            let sp = photocraft_psd::path::PsdSubpath { closed: true, operation: 1, knots: vec![k(0.0, 0.0), k(1.0, 0.0), k(1.0, 1.0), k(0.0, 1.0)] };
            (format!("Sq{i}"), photocraft_psd::csh::CshShape { bounds: [0, 0, 20, 20], subpaths: vec![sp] })
        })
        .collect();
    let bytes = photocraft_psd::csh::write(&shapes).unwrap();
    app.run("shape.presets.cshLoad", json!({"data": photocraft_engine::paint::tile::b64_encode(&bytes), "name": "Big"})).unwrap();
    for _ in 0..6 {
        place_and_read(&mut app);
    }
    // Each pick is read once by the background thread and then served from the cache: never the 5000.
    assert!(app.session.presets.lazy_shapes.parse_count() <= 14, "parsed {}", app.session.presets.lazy_shapes.parse_count());
}

#[test]
fn a_shape_too_detailed_for_a_vector_layer_says_to_use_pixels_mode() {
    let mut app = app();
    let knot = photocraft_psd::path::PsdKnot { linked: false, pre: (0.5, 0.5), anchor: (0.5, 0.5), post: (0.5, 0.5) };
    let angle = |i: usize| i as f64 / 5000.0 * std::f64::consts::TAU;
    let knots = (0..5000).map(|i| photocraft_psd::path::PsdKnot { anchor: (0.5 + 0.4 * angle(i).cos(), 0.5 + 0.4 * angle(i).sin()), ..knot }).collect();
    let dense =
        photocraft_psd::csh::CshShape { bounds: [0, 0, 100, 100], subpaths: vec![photocraft_psd::path::PsdSubpath { closed: true, operation: 1, knots }] };
    let bytes = photocraft_psd::csh::write(&[("Dense".to_string(), dense)]).unwrap();
    app.run("shape.presets.cshLoad", json!({"data": photocraft_engine::paint::tile::b64_encode(&bytes), "mode": "replace"})).unwrap();
    app.ui.presets_ui.custom_shape = "Dense".into();
    let before = layers(&app);
    press(&mut app, [20.0, 20.0], [120.0, 100.0]);
    assert_eq!(layers(&app), before, "a vector layer cannot hold it");
    assert!(app.ui.status.contains("Pixels mode") && app.ui.status_error, "{}", app.ui.status);
    // In Pixels mode the same shape paints.
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.custom_shape_mode = "pixels".into();
    app.ui.status.clear();
    press(&mut app, [20.0, 20.0], [120.0, 100.0]);
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
    assert!((20..120).any(|x| alpha_at(&app, x, 60) > 0.5));
}

#[test]
fn a_dense_shape_previews_from_a_thinned_copy_and_an_ordinary_one_from_itself() {
    let mut app = app();
    let (ordinary, dense) = proxy_path(&app).unwrap();
    assert!(!dense && ordinary.subpaths.iter().map(|s| s.knots.len()).sum::<usize>() < 100, "Heart is not dense");
    let k = |x: f64, y: f64| photocraft_psd::path::PsdKnot { linked: false, pre: (x, y), anchor: (x, y), post: (x, y) };
    let knots = (0..20_000)
        .map(|i| {
            let a = f64::from(i) / 20_000.0 * std::f64::consts::TAU;
            k(0.5 + 0.4 * a.cos(), 0.5 + 0.4 * a.sin())
        })
        .collect();
    let shape =
        photocraft_psd::csh::CshShape { bounds: [0, 0, 1000, 1000], subpaths: vec![photocraft_psd::path::PsdSubpath { closed: true, operation: 1, knots }] };
    let bytes = photocraft_psd::csh::write(&[("Ring".to_string(), shape)]).unwrap();
    app.run("shape.presets.cshLoad", json!({"data": photocraft_engine::paint::tile::b64_encode(&bytes), "mode": "replace"})).unwrap();
    app.ui.presets_ui.custom_shape = "Ring".into();
    let (thinned, dense) = proxy_path(&app).unwrap();
    assert!(dense);
    let n: usize = thinned.subpaths.iter().map(|s| s.knots.len()).sum();
    assert!((100..2000).contains(&n), "thinned from 20000 to {n}");
    // The leaf preview of it is built from the thinned copy and stays inside the leaf.
    app.ui.tool_options.custom_shape_fit = "leaf".into();
    let p = leaf_path(&app, Tool::CustomShape, [50.0, 100.0], [250.0, 100.0], egui::Modifiers::NONE).unwrap();
    assert!(p.subpaths.iter().map(|s| s.knots.len()).sum::<usize>() < 4000);
}

#[test]
fn alt_held_before_the_press_samples_a_colour_with_the_custom_shape_tool() {
    use crate::canvas::{ToolEvent, tool_event};
    let alt = egui::Modifiers::ALT;
    assert!(crate::canvas::alt_samples(Tool::CustomShape, alt));
    assert!(!crate::canvas::alt_samples(Tool::CustomShape, egui::Modifiers::NONE));
    assert!(!crate::canvas::alt_samples(Tool::CustomShape, egui::Modifiers { alt: true, ctrl: true, ..Default::default() }), "Control+Alt is the brush resize");
    let mut app = app();
    // A red patch to sample, then the tool.
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#ffffff"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"contents": "foreground"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
    app.ui.tool = Tool::CustomShape;
    let (shapes, rev) = (layers(&app), app.session.active().unwrap().revision);
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 60.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 60.0 }, alt);
    assert_eq!(app.session.tools.foreground, [1.0, 0.0, 0.0, 1.0], "the red under the pointer was sampled");
    assert_eq!((layers(&app), app.session.active().unwrap().revision), (shapes, rev), "and nothing was placed");
    // Without Alt the same drag places a shape; Alt pressed mid-drag still means "from the centre".
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 90.0, y: 90.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 90.0 }, alt);
    assert!(app.session.active().unwrap().revision > rev, "a shape was placed");
}

/// What the drag preview draws for a leaf of a few pixels, on a real traced drawing:
/// `CSH_SAMPLE=<real .csh> cargo test -p photocraft-ui-egui --lib leaf_preview_geometry -- --ignored --nocapture`.
#[test]
#[ignore = "needs a real Photoshop .csh file in CSH_SAMPLE"]
fn leaf_preview_geometry() {
    let Some(path) = std::env::var_os("CSH_SAMPLE") else { return };
    let mut app = app();
    let r = app.run("shape.presets.cshLoad", json!({"path": path.to_string_lossy(), "mode": "replace"})).unwrap();
    app.ui.presets_ui.custom_shape = r["first"].as_str().unwrap().to_string();
    app.ui.tool_options.custom_shape_fit = "leaf".into();
    for (a, b) in [([100.0, 100.0], [103.0, 101.0]), ([100.0, 100.0], [100.0, 160.0]), ([100.0, 100.0], [300.0, 230.0]), ([100.0, 100.0], [101.0, 101.0])] {
        let t = std::time::Instant::now();
        let sh = crate::vector_ui::shape_preview(&app, Tool::CustomShape, a, b, egui::Modifiers::NONE);
        let took = t.elapsed();
        let Some(sh) = sh else {
            println!("{a:?}->{b:?}: no preview ({took:?})");
            continue;
        };
        let knots: usize = sh.path.subpaths.iter().map(|s| s.knots.len()).sum();
        let bounds = sh.path.control_bounds();
        let longest = sh
            .path
            .subpaths
            .iter()
            .flat_map(|sp| sp.knots.windows(2).map(|k| (k[0].anchor.x - k[1].anchor.x).hypot(k[0].anchor.y - k[1].anchor.y)))
            .fold(0.0, f64::max);
        let open = sh.path.subpaths.iter().filter(|s| !s.closed).count();
        println!("{a:?}->{b:?}: {knots} knots in {} subpaths ({open} open), box {bounds:?}, longest piece {longest:.2}, {took:?}", sh.path.subpaths.len());
    }
}

#[test]
fn the_shape_picked_at_random_is_read_in_the_background() {
    let mut app = randomizing(1, 4);
    let shapes: Vec<_> = (0..40)
        .map(|i| {
            let k = |x: f64, y: f64| photocraft_psd::path::PsdKnot { linked: false, pre: (x, y), anchor: (x, y), post: (x, y) };
            let sp = photocraft_psd::path::PsdSubpath { closed: true, operation: 1, knots: vec![k(0.0, 0.0), k(1.0, 0.0), k(1.0, 1.0), k(0.0, 1.0)] };
            (format!("Sq{i}"), photocraft_psd::csh::CshShape { bounds: [0, 0, 20, 20], subpaths: vec![sp] })
        })
        .collect();
    let bytes = photocraft_psd::csh::write(&shapes).unwrap();
    app.run("shape.presets.cshLoad", json!({"data": photocraft_engine::paint::tile::b64_encode(&bytes), "mode": "replace"})).unwrap();
    app.ui.presets_ui.custom_shape = "Sq0".into();
    assert_eq!(app.session.presets.lazy_shapes.parse_count(), 0);
    after_place(&mut app);
    // The pick is made now; the thread reads it a moment later.
    let waited = std::time::Instant::now();
    while app.session.presets.lazy_shapes.parse_count() == 0 && waited.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.session.presets.lazy_shapes.parse_count(), 1, "read once, ahead of the drag");
}

fn press_command(app: &mut PhotocraftApp, id: &str) {
    let ctx = egui::Context::default();
    crate::shortcut_dispatch::dispatch(app, &ctx, id);
}

#[test]
fn the_options_can_be_changed_from_keys_and_each_says_what_it_is_now() {
    let mut app = app();
    assert_eq!((app.ui.tool_options.custom_shape_mode.as_str(), app.ui.tool_options.custom_shape_fit.as_str()), ("shape", "box"));
    press_command(&mut app, "ui.customShape.mode.toggle");
    assert_eq!(app.ui.tool_options.custom_shape_mode, "pixels");
    assert!(app.ui.status.contains("Pixels") && !app.ui.status_error, "{}", app.ui.status);
    press_command(&mut app, "ui.customShape.mode.toggle");
    assert_eq!(app.ui.tool_options.custom_shape_mode, "shape");
    press_command(&mut app, "ui.customShape.mode.pixels");
    press_command(&mut app, "ui.customShape.mode.pixels");
    assert_eq!(app.ui.tool_options.custom_shape_mode, "pixels", "setting twice is the same as once");
    press_command(&mut app, "ui.customShape.mode.shape");
    assert_eq!(app.ui.tool_options.custom_shape_mode, "shape");
    let mut seen = Vec::new();
    for _ in 0..4 {
        press_command(&mut app, "ui.customShape.fit.cycle");
        seen.push(app.ui.tool_options.custom_shape_fit.clone());
    }
    assert_eq!(seen, ["leaf", "zipper", "box", "leaf"]);
    for fit in ["box", "leaf", "zipper"] {
        press_command(&mut app, &format!("ui.customShape.fit.{fit}"));
        assert_eq!(app.ui.tool_options.custom_shape_fit, fit);
    }
}

#[test]
fn randomize_toggles_off_and_back_on_to_the_same_n() {
    let mut app = app();
    press_command(&mut app, "ui.customShape.random.toggle");
    assert_eq!(app.ui.tool_options.shape_random_every, 1, "on from off starts at every shape");
    app.ui.tool_options.shape_random_every = 4;
    press_command(&mut app, "ui.customShape.random.toggle");
    assert_eq!(app.ui.tool_options.shape_random_every, 0);
    assert!(app.ui.status.contains("off"), "{}", app.ui.status);
    press_command(&mut app, "ui.customShape.random.toggle");
    assert_eq!(app.ui.tool_options.shape_random_every, 4, "back to the N it had");
    assert!(app.ui.status.contains('4'), "{}", app.ui.status);
}

#[test]
fn the_commands_are_listed_for_the_keyboard_shortcuts_editor_and_work_with_any_tool() {
    let mut app = app();
    app.ui.tool = Tool::Brush;
    let listed: Vec<String> = crate::prefs_ui::shortcut_items(&app).into_iter().map(|i| i.0).collect();
    for id in COMMANDS {
        assert!(listed.iter().any(|l| l == id), "{id} is not in the editor's list");
        assert!(crate::menus::is_enabled(&app, id), "{id} is disabled");
    }
    press_command(&mut app, "ui.customShape.mode.toggle");
    assert_eq!(app.ui.tool_options.custom_shape_mode, "pixels", "no matter which tool is active");
    assert!(!app.ui.status_error);
}

/// A library of one shape, "Box": a square that fills its box.
fn use_a_box(app: &mut PhotocraftApp) {
    let k = |x: f64, y: f64| photocraft_psd::path::PsdKnot { linked: false, pre: (x, y), anchor: (x, y), post: (x, y) };
    let sp = photocraft_psd::path::PsdSubpath { closed: true, operation: 1, knots: vec![k(0.0, 0.0), k(1.0, 0.0), k(1.0, 1.0), k(0.0, 1.0)] };
    let bytes = photocraft_psd::csh::write(&[("Box".to_string(), photocraft_psd::csh::CshShape { bounds: [0, 0, 100, 100], subpaths: vec![sp] })]).unwrap();
    app.run("shape.presets.cshLoad", json!({"data": photocraft_engine::paint::tile::b64_encode(&bytes), "mode": "replace"})).unwrap();
    app.ui.presets_ui.custom_shape = "Box".into();
}

#[test]
fn zipper_mode_lays_the_shape_straight_in_the_leaf_and_cuts_it_off() {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.custom_shape_fit = "zipper".into();
    app.ui.tool_options.custom_shape_mode = "pixels".into();
    // A square is the box itself: the zipper of it is the leaf, with the box's corners cut.
    use_a_box(&mut app);
    press(&mut app, [50.0, 200.0], [350.0, 200.0]);
    assert!(alpha_at(&app, 200, 200) > 0.99, "the middle is filled");
    assert!(alpha_at(&app, 200, 130) > 0.99, "the box reaches its width in the middle");
    assert_eq!(alpha_at(&app, 55, 130), 0.0, "the corners of the box are cut by the leaf's pointed ends");
    assert_eq!(alpha_at(&app, 345, 270), 0.0);
    // The same drag in Leaf mode gives the same outline for a rectangle (a bent square is the leaf).
    let zipper_area: f32 = (0..400).flat_map(|x| (0..300).map(move |y| (x, y))).map(|(x, y)| alpha_at(&app, x, y)).sum();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.custom_shape_fit = "leaf".into();
    press(&mut app, [50.0, 200.0], [350.0, 200.0]);
    let leaf_area: f32 = (0..400).flat_map(|x| (0..300).map(move |y| (x, y))).map(|(x, y)| alpha_at(&app, x, y)).sum();
    assert!((zipper_area - leaf_area).abs() / leaf_area < 0.03, "{zipper_area} vs {leaf_area}");
}

#[test]
fn zipper_mode_in_shape_mode_makes_one_vector_layer_cut_by_the_leaf() {
    let mut app = app();
    app.ui.tool_options.custom_shape_fit = "zipper".into();
    use_a_box(&mut app);
    let before = layers(&app);
    press(&mut app, [50.0, 200.0], [350.0, 200.0]);
    assert_eq!(layers(&app), before + 1);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("not a shape layer") };
    assert!(sh.path.subpaths.iter().any(|s| s.op == photocraft_doc::PathOp::Intersect), "the leaf intersects the shape");
}

#[test]
fn zipper_preview_has_the_leaf_outline_and_old_prefs_still_load() {
    let mut app = app();
    app.ui.tool_options.custom_shape_fit = "zipper".into();
    let mods = egui::Modifiers::NONE;
    let outline = leaf_outline(&app, Tool::CustomShape, [50.0, 100.0], [250.0, 100.0], mods).unwrap();
    assert!(outline.control_bounds().is_some());
    let preview = leaf_path(&app, Tool::CustomShape, [50.0, 100.0], [250.0, 100.0], mods).unwrap();
    assert!(preview.subpaths.iter().any(|s| s.op == photocraft_doc::PathOp::Intersect));
    assert!(leaf_path(&app, Tool::CustomShape, [50.0, 100.0], [50.0, 100.0], mods).is_none());
}

#[test]
fn fixed_width_keeps_the_leaf_as_wide_whatever_the_drag_length() {
    let mut app = app();
    app.ui.tool_options.custom_shape_fit = LEAF.into();
    app.ui.tool_options.leaf_width = 0.5;
    app.ui.tool_options.leaf_fixed = true;
    app.ui.tool_options.leaf_width_px = 80.0;
    for len in [10.0, 100.0, 300.0] {
        let leaf = leaf_of_drag(&app, [10.0, 10.0], [10.0 + len, 10.0], egui::Modifiers::NONE).unwrap();
        assert!((leaf.width * len - 80.0).abs() < 1e-9, "length {len}: {}", leaf.width * len);
    }
    // ⌥ draws from the centre, so AB is twice the drag: the width in px is still the same.
    let leaf = leaf_of_drag(&app, [100.0, 100.0], [150.0, 100.0], egui::Modifiers::ALT).unwrap();
    assert!((leaf.width * (leaf.b.x - leaf.a.x).abs() - 80.0).abs() < 1e-9);
    app.ui.tool_options.leaf_fixed = false;
    let leaf = leaf_of_drag(&app, [10.0, 10.0], [110.0, 10.0], egui::Modifiers::NONE).unwrap();
    assert_eq!(leaf.width, 0.5, "off: the width stays a share of the length");
}

#[test]
fn erase_mode_clears_pixels_with_the_shape_in_box_and_leaf_modes() {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
    app.run("edit.fill", json!({"contents": "foreground"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool_options.custom_shape_mode = "pixels".into();
    app.ui.tool_options.custom_shape_erase = true;
    press(&mut app, [100.0, 100.0], [220.0, 200.0]);
    assert!(alpha_at(&app, 160, 150) < 0.01, "erased under the shape");
    assert!(alpha_at(&app, 10, 10) > 0.99);
    app.ui.tool_options.custom_shape_fit = LEAF.into();
    press(&mut app, [20.0, 250.0], [200.0, 250.0]);
    assert!(alpha_at(&app, 110, 250) < 0.01, "erased under the leaf");
    // Erase is a Pixels option: in Shape mode a shape layer is made as before.
    app.ui.tool_options.custom_shape_mode = "shape".into();
    app.ui.tool_options.custom_shape_fit = "box".into();
    let before = layers(&app);
    press(&mut app, [300.0, 20.0], [380.0, 80.0]);
    assert_eq!(layers(&app), before + 1);
}
