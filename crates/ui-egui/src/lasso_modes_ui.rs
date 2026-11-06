//! What a finished lasso selection does on its own (fork-local, see MY_CHANGES.md), from the
//! Lasso Draw plug-in: Clear deletes the pixels inside the outline, Draw paints them with the
//! foreground colour (with ⇧ it picks the area's dominant colour as the foreground instead),
//! Shading paints them with the mean of the area's two dominant colours, Custom Shape covers the
//! outline with a grid of custom shapes. Shared by the Lasso, Polygonal and Magnetic Lasso.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const OFF: &str = "off";
const CLEAR: &str = "clear";
const SHAPE: &str = "shape";
const DRAW: &str = "draw";
const SHADE: &str = "shade";
/// The cell heights offered as buttons (px).
const PRESETS: [f32; 5] = [20.0, 40.0, 60.0, 80.0, 100.0];
/// Same cap as the engine's `shape.presets.fillGrid`.
const MAX_CELLS: usize = 50_000;

fn lasso(tool: Tool) -> bool {
    matches!(tool, Tool::Lasso | Tool::PolygonLasso | Tool::MagneticLasso)
}

/// Is a mode other than Off on?
fn on(app: &PhotocraftApp) -> bool {
    matches!(app.ui.tool_options.lasso_mode.as_str(), CLEAR | SHAPE | DRAW | SHADE)
}

/// The selection mode a lasso outline is made in: with a mode on it always replaces
/// the selection, so the action covers only what was just drawn, whatever the modifier keys say.
pub fn mode(app: &PhotocraftApp, tool: Tool, chosen: impl Into<String>) -> String {
    if lasso(tool) && on(app) { "replace".to_owned() } else { chosen.into() }
}

/// The lasso modes in the order the cycle key steps through them.
const ORDER: [&str; 5] = [OFF, CLEAR, DRAW, SHADE, SHAPE];

/// The lasso mode commands the user can give keys to (listed in `SHELL_COMMANDS`, so Edit ›
/// Keyboard Shortcuts shows them; none has a key by default).
const COMMANDS: &[&str] = &[
    "ui.lasso.mode.off",
    "ui.lasso.mode.clear",
    "ui.lasso.mode.draw",
    "ui.lasso.mode.shade",
    "ui.lasso.mode.shape",
    "ui.lasso.mode.cycle",
];

/// Is `id` one of the lasso mode commands? They work whatever tool is active.
pub fn handles(id: &str) -> bool {
    COMMANDS.contains(&id)
}

/// The name of mode `mode` for the status bar.
fn said(mode: &str) -> String {
    match mode {
        CLEAR => tl!("Lasso: Clear"),
        DRAW => tl!("Lasso: Draw"),
        SHADE => tl!("Lasso: Shading"),
        SHAPE => tl!("Lasso: Custom Shape"),
        _ => tl!("Lasso: Off"),
    }
    .to_string()
}

/// Change the lasso mode from a key and say what it is now in the status bar (the options bar
/// shows it only while a lasso is selected). `None` for any other command.
pub fn invoke(app: &mut PhotocraftApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    if !handles(id) {
        return None;
    }
    let o = &mut app.ui.tool_options;
    let next = match id.strip_prefix("ui.lasso.mode.").unwrap_or_default() {
        "cycle" => {
            let at = ORDER.iter().position(|m| *m == o.lasso_mode).map_or(0, |i| i + 1);
            ORDER.get(at % ORDER.len()).copied().unwrap_or(OFF)
        }
        "clear" => CLEAR,
        "draw" => DRAW,
        "shade" => SHADE,
        "shape" => SHAPE,
        _ => OFF,
    };
    o.lasso_mode = next.into();
    app.ui.status = said(next);
    app.ui.status_error = false;
    Some(Ok(serde_json::Value::Null))
}

/// The lasso options: Mode (Off / Clear / Custom Shape) and, for Custom Shape, the cell height.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let label = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(text).color(t.text_dim).size(12.0));
    label(ui, tl!("Mode:"));
    let o = &mut app.ui.tool_options;
    let modes = [(OFF.to_string(), tl!("Off")), (CLEAR.to_string(), tl!("Clear")), (DRAW.to_string(), tl!("Draw")), (SHADE.to_string(), tl!("Shading")), (SHAPE.to_string(), tl!("Custom Shape"))];
    crate::widgets::dropdown(ui, "lasso-mode", &mut o.lasso_mode, &modes, 110.0);
    if o.lasso_mode == SHAPE {
        label(ui, tl!("Cell:"));
        crate::widgets::value_field(ui, &mut o.lasso_cell_px, 2.0..=2000.0, "px", 62.0);
        for size in PRESETS {
            if ui.selectable_label((o.lasso_cell_px - size).abs() < 0.5, format!("{size:.0}")).clicked() {
                o.lasso_cell_px = size;
            }
        }
    }
    crate::widgets::vline(ui, 22.0);
}

/// A lasso selection was just made: do what the mode says with it. `shift` is ⇧ held as the
/// outline closed (Draw then picks a colour instead of painting).
pub fn after_select(app: &mut PhotocraftApp, shift: bool) {
    if !app.session.active().is_some_and(|st| st.doc.selection.is_some()) {
        return;
    }
    match app.ui.tool_options.lasso_mode.as_str() {
        CLEAR => clear(app),
        DRAW if shift => pick_color(app),
        DRAW => draw(app),
        SHADE => shade(app),
        SHAPE => fill_shapes(app),
        _ => return,
    }
    if app.session.is_enabled("select.deselect") {
        let _ = app.run("select.deselect", json!({}));
    }
}

/// Delete the pixels inside the selection (`edit.clear`: locks, the Background and partly
/// selected pixels are handled there; a refusal shows in the status bar).
fn clear(app: &mut PhotocraftApp) {
    let _ = app.run("edit.clear", json!({}));
}

/// Paint the selection with the foreground colour (`edit.fill`).
fn draw(app: &mut PhotocraftApp) {
    let _ = app.run("edit.fill", json!({"contents": "foreground"}));
}

/// Make the dominant colour of the selected area the foreground colour; the pixels stay as they are.
fn pick_color(app: &mut PhotocraftApp) {
    let Ok(v) = app.run("select.dominantColor", json!({})) else { return };
    if let Some(c) = v.get("color").filter(|c| c.is_array()) {
        let _ = app.run("tools.setColors", json!({"foreground": c}));
    }
}

/// Paint the selection with the mean of its two dominant colours (`edit.fillShade`).
fn shade(app: &mut PhotocraftApp) {
    let _ = app.run("edit.fillShade", json!({}));
}

/// Cover the selection with the grid of shapes (`shape.presets.fillGrid`), one history step.
fn fill_shapes(app: &mut PhotocraftApp) {
    let cell = f64::from(app.ui.tool_options.lasso_cell_px).clamp(2.0, 2000.0);
    let Some(count) = cell_count(app, cell) else { return };
    let names = crate::custom_shape_ui::sequence(app, count);
    let p = json!({"shapes": names, "cell": cell, "antiAlias": app.ui.tool_options.anti_alias});
    // The engine reports a failure (a locked layer, an unknown shape) in the status bar.
    let _ = app.run("shape.presets.fillGrid", p);
}

/// How many cells the selection's box holds at `cell` px (at most the engine's limit), `None` for
/// an empty selection.
fn cell_count(app: &PhotocraftApp, cell: f64) -> Option<usize> {
    let st = app.session.active()?;
    let b = st.doc.selection.as_ref()?.content_bounds().intersect(&st.doc.bounds());
    if b.is_empty() {
        return None;
    }
    let (cols, rows) = ((f64::from(b.width()) / cell).ceil() as usize, (f64::from(b.height()) / cell).ceil() as usize);
    Some(cols.saturating_mul(rows).clamp(1, MAX_CELLS))
}

#[cfg(test)]
#[path = "lasso_modes_ui_tests.rs"]
mod tests;
