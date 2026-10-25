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
