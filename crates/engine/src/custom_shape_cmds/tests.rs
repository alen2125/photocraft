use serde_json::json;

use crate::Session;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn alpha(s: &Session, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn layers(s: &Session) -> usize {
    s.active().unwrap().doc.layers.len()
}

/// A heart's box: the shape's centre is inside it at every depth.
const RECT: [i32; 4] = [20, 20, 60, 60];

fn fill(s: &mut Session, extra: serde_json::Value) -> crate::Result<serde_json::Value> {
    let mut p = json!({"preset": "Heart", "rect": RECT, "color": "#ff0000"});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s.execute("shape.presets.fillPixels", p)
}

#[test]
fn fills_the_shape_on_the_active_pixel_layer_at_every_depth_and_undoes_in_one_step() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let before = layers(&s);
        let r = fill(&mut s, json!({})).unwrap();
        assert_eq!(r["empty"], json!(false));
        assert_eq!(layers(&s), before, "{depth}: the pixel layer is reused");
        let px = rgba(&s, 50, 45);
        assert!((px[0] - 1.0).abs() < 1e-2 && px[3] > 0.99, "{depth}: {px:?}");
        assert_eq!(alpha(&s, 5, 5), 0.0, "{depth}: outside the shape");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(alpha(&s, 50, 45), 0.0, "{depth}: one undo restores");
    }
}

#[test]
fn opacity_uses_the_foreground_when_no_colour_is_given() {
    let mut s = session(8);
    s.execute("tools.setColors", json!({"foreground": "#0000ff", "background": "#ffffff"})).unwrap();
    s.execute("shape.presets.fillPixels", json!({"preset": "Heart", "rect": RECT, "opacity": 50})).unwrap();
    let px = rgba(&s, 50, 45);
    assert!(px[2] > 0.99 && (px[3] - 0.5).abs() < 0.02, "{px:?}");
}

#[test]
fn a_non_pixel_layer_gets_a_new_pixel_layer_in_the_same_step() {
    let mut s = session(8);
    s.execute("shape.create", json!({"kind": "rect", "rect": [0, 0, 5, 5]})).unwrap();
    let before = layers(&s);
    fill(&mut s, json!({})).unwrap();
    assert_eq!(layers(&s), before + 1);
    assert!(alpha(&s, 50, 45) > 0.99);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layers(&s), before, "one undo removes the layer and the paint");
}

#[test]
fn the_selection_limits_the_fill() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 50, "height": 100})).unwrap();
        fill(&mut s, json!({})).unwrap();
        assert!(alpha(&s, 40, 45) > 0.99, "{depth}");
        assert_eq!(alpha(&s, 60, 45), 0.0, "{depth}: outside the selection");
    }
}

#[test]
fn locked_pixels_refuse_and_locked_transparency_keeps_the_alpha() {
    let mut s = session(8);
    s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
    fill(&mut s, json!({})).unwrap();
    assert_eq!(alpha(&s, 50, 45), 0.0, "transparent pixels stay transparent");
    let mut s = session(8);
    s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
    assert!(fill(&mut s, json!({})).is_err());
}

#[test]
fn a_shape_wholly_off_the_canvas_changes_nothing() {
    let mut s = session(8);
    let before = layers(&s);
    let r = fill(&mut s, json!({"rect": [-300, -300, 50, 50]})).unwrap();
    assert_eq!(r["empty"], json!(true));
    assert_eq!(layers(&s), before);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layers(&s), before - 1, "no step was recorded: undo removes the layer made in setup");
}

#[test]
fn a_shape_hanging_over_the_edge_is_clipped_without_error() {
    let mut s = session(8);
    fill(&mut s, json!({"rect": [-20, -20, 80, 80]})).unwrap();
    assert!(alpha(&s, 20, 20) > 0.99);
}

#[test]
fn bad_params_and_disabled_state() {
    let mut s = session(8);
    assert!(s.execute("shape.presets.fillPixels", json!({})).is_err());
    assert!(fill(&mut s, json!({"preset": "No such shape"})).is_err());
    assert!(fill(&mut s, json!({"rect": [0, 0, 0, 10]})).is_err());
    let mut none = Session::new();
    assert!(!none.is_enabled("shape.presets.fillPixels"));
    assert!(none.execute("shape.presets.fillPixels", json!({"preset": "Heart"})).is_err());
}

fn leaf(a: [f64; 2], b: [f64; 2]) -> serde_json::Value {
    json!({"a": a, "b": b, "width": 0.5})
}

#[test]
fn fill_pixels_on_a_leaf_paints_inside_the_leaf_only() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        // A leaf from (10, 50) to (90, 50): its middle is 40 px wide, its ends are points.
        s.execute("shape.presets.fillPixels", json!({"preset": "Heart", "leaf": leaf([10.0, 50.0], [90.0, 50.0]), "color": "#ff0000"})).unwrap();
        assert!(alpha(&s, 50, 50) > 0.99, "{depth}: the middle is covered");
        assert_eq!(alpha(&s, 50, 5), 0.0, "{depth}: far above the leaf");
        assert_eq!(alpha(&s, 5, 50), 0.0, "{depth}: beyond A");
        assert_eq!(alpha(&s, 95, 50), 0.0, "{depth}: beyond B");
        assert_eq!(alpha(&s, 50, 80), 0.0, "{depth}: far below the leaf");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(alpha(&s, 50, 50), 0.0, "{depth}: undone in one step");
    }
}

#[test]
fn place_leaf_makes_one_shape_layer_inside_the_leaf() {
    let mut s = session(8);
    let before = layers(&s);
    s.execute("shape.presets.placeLeaf", json!({"preset": "Heart", "leaf": leaf([10.0, 50.0], [90.0, 50.0])})).unwrap();
    assert_eq!(layers(&s), before + 1);
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    let photocraft_doc::LayerContent::Shape(sh) = &l.content else { panic!("not a shape layer") };
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    assert!(x0 >= 9.9 && x1 <= 90.1 && y0 >= 49.9 - 25.0 && y1 <= 50.1 + 25.0, "{x0} {y0} {x1} {y1}");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layers(&s), before);
}

#[test]
fn a_leaf_with_a_zero_width_is_a_line_and_does_not_fail() {
    let mut s = session(8);
    let mut l = leaf([10.0, 50.0], [90.0, 50.0]);
    l["width"] = json!(0.0);
    s.execute("shape.presets.fillPixels", json!({"preset": "Heart", "leaf": l})).unwrap();
}

#[test]
fn bad_leaves_are_refused() {
    let mut s = session(8);
    for l in [leaf([5.0, 5.0], [5.0, 5.0]), json!({"a": [1, 2]}), json!({"a": [1, 2], "b": "x"}), json!({"a": [1, 2], "b": [1]})] {
        assert!(s.execute("shape.presets.fillPixels", json!({"preset": "Heart", "leaf": l.clone()})).is_err(), "{l}");
        assert!(s.execute("shape.presets.placeLeaf", json!({"preset": "Heart", "leaf": l.clone()})).is_err(), "{l}");
    }
    assert!(s.execute("shape.presets.placeLeaf", json!({"preset": "Heart"})).is_err(), "a leaf is required");
    assert!(s.execute("shape.presets.placeLeaf", json!({"preset": "Nope", "leaf": leaf([0.0, 0.0], [9.0, 9.0])})).is_err());
}

#[test]
fn a_zipper_fill_is_the_shape_laid_straight_and_cut_by_the_leaf() {
    let mut s = session(8);
    // Heart in the leaf's box without bending: the bottom point stays on the bottom edge of the
    // box, which the leaf's pointed ends do not reach at x = 15.
    let mut leaf = leaf([10.0, 50.0], [90.0, 50.0]);
    leaf["clip"] = json!(true);
    leaf["width"] = json!(0.5);
    s.execute("shape.presets.fillPixels", json!({"preset": "Heart", "leaf": leaf.clone(), "color": "#000000"})).unwrap();
    assert!(alpha(&s, 50, 50) > 0.99, "the middle of the heart is filled");
    assert_eq!(alpha(&s, 12, 32), 0.0, "the corner of the box, where the leaf is pointed, is cut off");
    assert_eq!(alpha(&s, 50, 5), 0.0, "nothing beyond the box");
    // Bent, the same leaf gives a different picture.
    let mut t = session(8);
    leaf["clip"] = json!(false);
    t.execute("shape.presets.fillPixels", json!({"preset": "Heart", "leaf": leaf, "color": "#000000"})).unwrap();
    let area = |s: &Session| -> f32 { (0..100).flat_map(|x| (0..100).map(move |y| (x, y))).map(|(x, y)| alpha(s, x, y)).sum() };
    assert!((area(&s) - area(&t)).abs() > 20.0, "zipper {} vs leaf {}", area(&s), area(&t));
}

#[test]
fn a_zipper_vector_shape_is_one_layer_with_the_leaf_as_an_intersecting_subpath() {
    let mut s = session(8);
    let before = layers(&s);
    let mut l = leaf([10.0, 50.0], [90.0, 50.0]);
    l["clip"] = json!(true);
    s.execute("shape.presets.placeLeaf", json!({"preset": "Heart", "leaf": l})).unwrap();
    assert_eq!(layers(&s), before + 1);
    let d = s.active().unwrap();
    let photocraft_doc::LayerContent::Shape(sh) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("not a shape layer") };
    assert!(sh.path.subpaths.iter().any(|sp| sp.op == photocraft_doc::PathOp::Intersect));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layers(&s), before);
}

fn paint_all(s: &mut Session) {
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#00ff00", "background": "#ffffff"})).unwrap();
    s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
}

#[test]
fn erase_removes_alpha_under_the_shape_at_every_depth_in_one_step() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        paint_all(&mut s);
        let before = layers(&s);
        fill(&mut s, json!({"erase": true})).unwrap();
        assert_eq!(layers(&s), before, "{depth}: no new layer");
        assert!(alpha(&s, 50, 45) < 0.01, "{depth}: erased under the shape");
        assert!(alpha(&s, 5, 5) > 0.99, "{depth}: outside stays");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(alpha(&s, 50, 45) > 0.99, "{depth}: one undo restores");
    }
}

#[test]
fn erase_respects_opacity_selection_and_locks_and_needs_a_pixel_layer() {
    let mut s = session(8);
    paint_all(&mut s);
    fill(&mut s, json!({"erase": true, "opacity": 50})).unwrap();
    assert!((alpha(&s, 50, 45) - 0.5).abs() < 0.02);

    let mut s = session(8);
    paint_all(&mut s);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 50, "height": 100})).unwrap();
    fill(&mut s, json!({"erase": true})).unwrap();
    assert!(alpha(&s, 40, 45) < 0.01 && alpha(&s, 60, 45) > 0.99, "only inside the selection");

    let mut s = session(8);
    paint_all(&mut s);
    s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
    fill(&mut s, json!({"erase": true})).unwrap();
    assert!(alpha(&s, 50, 45) > 0.99, "locked transparency: nothing to erase");
    s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
    assert!(fill(&mut s, json!({"erase": true})).is_err());

    let mut s = session(8);
    s.execute("shape.create", json!({"kind": "rect", "rect": [0, 0, 5, 5]})).unwrap();
    let before = layers(&s);
    assert!(fill(&mut s, json!({"erase": true})).is_err(), "no pixel layer to erase");
    assert_eq!(layers(&s), before, "and no layer is made for it");
}

fn grid(s: &mut Session, extra: serde_json::Value) -> crate::Result<serde_json::Value> {
    let mut p = json!({"shapes": ["Heart"], "cell": 20, "color": "#ff0000", "background": "#0000ff"});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s.execute("shape.presets.fillGrid", p)
}

#[test]
fn fill_grid_paints_shapes_and_background_inside_the_selection_only() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
        let before = layers(&s);
        let r = grid(&mut s, json!({})).unwrap();
        assert_eq!(r["empty"], json!(false));
        assert_eq!(r["cells"], json!(6), "{depth}: 3 columns by 2 rows");
        assert_eq!(layers(&s), before, "{depth}: the pixel layer is reused");
        assert_eq!(alpha(&s, 5, 5), 0.0, "{depth}: outside the selection");
        assert_eq!(alpha(&s, 75, 30), 0.0, "{depth}: outside the selection");
        // The middle of the first cell (10..30, 10..30) is in the shape, its corner in the gap.
        let mid = rgba(&s, 20, 21);
        assert!(mid[0] > 0.99 && mid[2] < 0.01, "{depth}: heart in the foreground colour {mid:?}");
        let corner = rgba(&s, 10, 10);
        assert!(corner[2] > 0.99 && corner[0] < 0.01 && corner[3] > 0.99, "{depth}: gap in the background colour {corner:?}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(alpha(&s, 20, 21), 0.0, "{depth}: one undo restores");
    }
}

#[test]
fn fill_grid_takes_the_names_in_order_and_skips_cells_off_the_selection() {
    let mut s = session(8);
    // A diamond-ish selection leaves its corner cells empty.
    s.execute("select.lasso", json!({"points": [[50, 0], [100, 50], [50, 100], [0, 50]]})).unwrap();
    let r = grid(&mut s, json!({"shapes": ["Heart", "Heart"], "cell": 25})).unwrap();
    let cells = r["cells"].as_u64().unwrap();
    assert!(cells > 0 && cells < 16, "corner cells are skipped: {cells}");
    assert_eq!(alpha(&s, 2, 2), 0.0);
}

#[test]
fn fill_grid_fails_gracefully() {
    let mut s = session(8);
    assert!(grid(&mut s, json!({})).is_err(), "no selection");
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    assert!(grid(&mut s, json!({"shapes": []})).is_err());
    assert!(grid(&mut s, json!({"shapes": ["No Such Shape"]})).is_err());
    for cell in [json!(0), json!(-5), json!(1), json!(1e9), json!("x"), json!(null)] {
        assert!(grid(&mut s, json!({"cell": cell})).is_err(), "{cell}");
    }
    s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
    assert!(grid(&mut s, json!({})).is_err());
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    assert!(grid(&mut s, json!({"cell": 2})).is_ok(), "2 px cells: 2500 of them");
}
