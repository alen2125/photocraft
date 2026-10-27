//! What a finished lasso selection does on its own (fork-local, see MY_CHANGES.md), from the
//! Lasso Draw plug-in: Clear deletes the pixels inside the outline, Custom Shape covers the
//! outline with a grid of custom shapes. Shared by the Lasso, Polygonal and Magnetic Lasso.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const OFF: &str = "off";
const CLEAR: &str = "clear";
const SHAPE: &str = "shape";
/// The cell heights offered as buttons (px).
const PRESETS: [f32; 5] = [20.0, 40.0, 60.0, 80.0, 100.0];
/// Same cap as the engine's `shape.presets.fillGrid`.
const MAX_CELLS: usize = 50_000;

fn lasso(tool: Tool) -> bool {
    matches!(tool, Tool::Lasso | Tool::PolygonLasso | Tool::MagneticLasso)
}

/// Is a mode other than Off on?
fn on(app: &PhotocraftApp) -> bool {
    matches!(app.ui.tool_options.lasso_mode.as_str(), CLEAR | SHAPE)
}

/// The selection mode a lasso outline is made in: with Clear or Custom Shape on it always replaces
/// the selection, so the action covers only what was just drawn, whatever the modifier keys say.
pub fn mode(app: &PhotocraftApp, tool: Tool, chosen: impl Into<String>) -> String {
    if lasso(tool) && on(app) { "replace".to_owned() } else { chosen.into() }
}

/// The lasso options: Mode (Off / Clear / Custom Shape) and, for Custom Shape, the cell height.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let label = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(text).color(t.text_dim).size(12.0));
    label(ui, tl!("Mode:"));
    let o = &mut app.ui.tool_options;
    let modes = [(OFF.to_string(), tl!("Off")), (CLEAR.to_string(), tl!("Clear")), (SHAPE.to_string(), tl!("Custom Shape"))];
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

/// A lasso selection was just made: do what the mode says with it.
pub fn after_select(app: &mut PhotocraftApp) {
    if !app.session.active().is_some_and(|st| st.doc.selection.is_some()) {
        return;
    }
    match app.ui.tool_options.lasso_mode.as_str() {
        CLEAR => clear(app),
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
