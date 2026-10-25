//! Custom Shape tool additions kept out of the shared tool files: what a click or an off-canvas
//! drag does (fork-local, see MY_CHANGES.md).

use crate::PhotocraftApp;

/// End of a Custom Shape drag. Never opens a dialog: a drag smaller than 1 px places a square of
/// `custom_shape_default_size` centred on the click (nothing when that is 0), and a drag that
/// starts on the pasteboard is placed as drawn, the canvas clipping what hangs over its edge.
pub fn finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let Some(rect) = drag_rect(app, start, end, mods) else { return };
    let Some(st) = app.session.active() else { return };
    let (w, h) = (st.doc.size.width as f64, st.doc.size.height as f64);
    if rect[0] >= w || rect[1] >= h || rect[0] + rect[2] <= 0.0 || rect[1] + rect[3] <= 0.0 {
        // Nothing of it lands on the canvas: skip silently, no message.
        return;
    }
    if app.ui.tool_options.custom_shape_mode == PIXELS {
        fill_pixels(app, rect, mods.shift);
        return;
    }
    let (fill, stroke) = (crate::vector_ui::fill_param(app), crate::vector_ui::stroke_param(app));
    crate::preset_panels::finish_custom_shape(app, rect, mods.shift, fill, stroke);
}

const PIXELS: &str = "pixels";

/// Pixels mode: paint the shape on the active pixel layer (`shape.presets.fillPixels`).
fn fill_pixels(app: &mut PhotocraftApp, rect: [f64; 4], keep: bool) {
    let name = app.ui.presets_ui.shape().to_string();
    // The command reports a failure (locked layer, unknown shape) in the status bar.
    let _ = app.run("shape.presets.fillPixels", serde_json::json!({"preset": name, "rect": rect, "keepAspect": keep}));
}

/// The options bar's Shape / Pixels switch for the Custom Shape tool.
pub fn mode_dropdown(mode: &mut String, ui: &mut egui::Ui) {
    let items = [("shape".to_string(), tl!("Shape")), (PIXELS.to_string(), tl!("Pixels"))];
    crate::widgets::dropdown(ui, "custom-shape-mode", mode, &items, 80.0);
}

/// Thumbnails kept at once: enough for every cell in view, bounded however big the library.
const TEXTURES: usize = 512;

type TextureCache = Vec<((String, u64), egui::TextureHandle)>;

/// The thumbnail of shape `name` in `group`, made (and its path parsed) the first time it is
/// asked for and kept in a small least-recently-used cache. Callers ask only for visible cells.
pub fn texture(ctx: &egui::Context, session: &photocraft_engine::Session, group: &str, name: &str) -> Option<egui::TextureHandle> {
    const SIZE: u32 = 64;
    let id = egui::Id::new("custom-shape-thumbnails");
    let key = (format!("{group}/{name}"), session.presets.rev);
    let hit = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<TextureCache>(id);
        let at = cache.iter().position(|(k, _)| *k == key)?;
        let entry = cache.remove(at);
        let tex = entry.1.clone();
        cache.push(entry);
        Some(tex)
    });
    if hit.is_some() {
        return hit;
    }
    let shape = photocraft_engine::shape_index::find_shape(session, name, Some(group))?;
    let cov = photocraft_engine::presets::shapes::thumbnail(&shape.path, SIZE);
    let pixels = cov.iter().map(|a| egui::Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    let image = egui::ColorImage::new([SIZE as usize, SIZE as usize], pixels);
    let tex = ctx.load_texture(format!("custom-shape-{}", key.0), image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<TextureCache>(id);
        cache.push((key, tex.clone()));
        if cache.len() > TEXTURES {
            let _evicted = cache.remove(0);
        }
    });
    Some(tex)
}

/// The `[x, y, w, h]` a drag covers, or the click-placed default square, `None` for neither.
fn drag_rect(app: &PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<[f64; 4]> {
    let o = &app.ui.tool_options;
    if let Some([x0, y0, x1, y1]) = crate::vector_ui::shape_bounds(o, crate::state::Tool::CustomShape, start, end, mods) {
        return Some([x0, y0, x1 - x0, y1 - y0]);
    }
    let side = f64::from(o.custom_shape_default_size);
    (side.is_finite() && side >= 1.0).then(|| [(start[0] - side / 2.0).round(), (start[1] - side / 2.0).round(), side.round(), side.round()])
}

#[cfg(test)]
#[path = "custom_shape_ui_tests.rs"]
mod tests;
