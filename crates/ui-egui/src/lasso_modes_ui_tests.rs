use serde_json::json;

use super::*;
use crate::canvas::{Drag, finish_gesture};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    // A transparent layer painted solid in the middle, so clearing and filling show in its alpha.
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
    app.run("edit.fill", json!({"contents": "foreground"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app
}

fn alpha_at(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn has_selection(app: &PhotocraftApp) -> bool {
    app.session.active().unwrap().doc.selection.is_some()
}

/// A lasso drag round the square (100, 100) - (200, 200).
fn lasso_square(app: &mut PhotocraftApp) {
    app.ui.tool = Tool::Lasso;
    let pts = [[100.0, 100.0], [200.0, 100.0], [200.0, 200.0], [100.0, 200.0]];
    let d = Drag::new(Tool::Lasso, pts[0], pts.iter().map(|p| [p[0], p[1], 1.0]).collect(), egui::Modifiers::NONE, false);
    finish_gesture(app, d);
}

#[test]
fn off_only_selects() {
    let mut app = app();
    lasso_square(&mut app);
    assert!(has_selection(&app));
    assert_eq!(alpha_at(&app, 150, 150), 1.0);
}

#[test]
fn clear_deletes_only_the_pixels_inside_the_outline() {
    let mut app = app();
    app.ui.tool_options.lasso_mode = CLEAR.into();
    lasso_square(&mut app);
    assert_eq!(alpha_at(&app, 150, 150), 0.0, "inside is cleared");
    assert_eq!(alpha_at(&app, 50, 50), 1.0, "outside stays");
    assert_eq!(alpha_at(&app, 250, 250), 1.0);
    assert!(!has_selection(&app), "the selection is dropped after the clear");
}

#[test]
fn clear_forces_a_new_selection_even_with_a_modifier_mode() {
    let mut app = app();
    app.run("select.rect", json!({"x": 300, "y": 0, "width": 50, "height": 50})).unwrap();
    app.ui.tool_options.lasso_mode = CLEAR.into();
    lasso_square(&mut app);
    assert_eq!(alpha_at(&app, 320, 20), 1.0, "the earlier selection is not cleared with it");
    assert_eq!(alpha_at(&app, 150, 150), 0.0);
}

#[test]
fn clear_on_a_locked_layer_reports_and_does_not_panic() {
    let mut app = app();
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    app.ui.tool_options.lasso_mode = CLEAR.into();
    lasso_square(&mut app);
    assert_eq!(alpha_at(&app, 150, 150), 1.0);
}

#[test]
fn shapes_fill_the_outline_in_the_foreground_colour_over_the_background_colour() {
    let mut app = app();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    app.session.tools.background = [0.0, 0.0, 1.0, 1.0];
    app.ui.tool_options.lasso_mode = SHAPE.into();
    app.ui.tool_options.lasso_cell_px = 20.0;
    let name = photocraft_engine::shape_index::index(&app.session).into_iter().flat_map(|g| g.items).next().unwrap();
    app.ui.presets_ui.custom_shape = name;
    lasso_square(&mut app);
    assert_eq!(alpha_at(&app, 50, 50), 0.0, "nothing outside the outline");
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let (mut red, mut blue) = (0, 0);
    for y in 100..200 {
        for x in 100..200 {
            let c = surf.rgba(x, y);
            assert!(c[3] > 0.99, "every pixel inside is covered at ({x}, {y})");
            red += usize::from(c[0] > 0.9 && c[2] < 0.1);
            blue += usize::from(c[2] > 0.9 && c[0] < 0.1);
        }
    }
    assert!(red > 200, "shapes in the foreground colour: {red}");
    assert!(blue > 200, "the gaps in the background colour: {blue}");
    assert!(!has_selection(&app));
}

#[test]
fn shapes_are_laid_in_rows_from_the_top_left_of_the_outline() {
    let mut app = app();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool_options.lasso_mode = SHAPE.into();
    app.ui.tool_options.lasso_cell_px = 50.0;
    lasso_square(&mut app);
    // Two rows of two cells: the grid starts at the selection's corner, so each quarter has ink.
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    for (x0, y0) in [(100, 100), (150, 100), (100, 150), (150, 150)] {
        let ink = (y0..y0 + 50).any(|y| (x0..x0 + 50).any(|x| surf.rgba(x, y)[0] < 0.1 && surf.rgba(x, y)[3] > 0.9));
        assert!(ink, "cell at ({x0}, {y0}) has a shape");
    }
}

#[test]
fn a_lasso_without_a_selection_does_nothing() {
    let mut app = app();
    app.ui.tool_options.lasso_mode = CLEAR.into();
    after_select(&mut app);
    assert_eq!(alpha_at(&app, 150, 150), 1.0, "no selection means nothing is cleared");
}

#[test]
fn mode_replaces_only_when_an_action_is_on() {
    let mut app = app();
    assert_eq!(mode(&app, Tool::Lasso, "add"), "add");
    app.ui.tool_options.lasso_mode = SHAPE.into();
    assert_eq!(mode(&app, Tool::Lasso, "add"), "replace");
    assert_eq!(mode(&app, Tool::Patch, "add"), "add", "other tools keep theirs");
}
