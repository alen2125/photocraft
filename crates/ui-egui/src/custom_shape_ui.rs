//! Custom Shape tool additions kept out of the shared tool files: what a click or an off-canvas
//! drag does (fork-local, see MY_CHANGES.md).

use photocraft_doc::Path;
use photocraft_geom::Point;
use photocraft_vector::warp::{Leaf, warp_path};

use crate::PhotocraftApp;

/// End of a Custom Shape drag. Never opens a dialog: a drag smaller than 1 px places a square of
/// `custom_shape_default_size` centred on the click (nothing when that is 0), and a drag that
/// starts on the pasteboard is placed as drawn, the canvas clipping what hangs over its edge.
pub fn finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    if is_leaf(app) {
        return finish_leaf(app, start, end, mods);
    }
    let Some(rect) = drag_rect(app, start, end, mods) else { return };
    let Some(st) = app.session.active() else { return };
    let (w, h) = (st.doc.size.width as f64, st.doc.size.height as f64);
    if rect[0] >= w || rect[1] >= h || rect[0] + rect[2] <= 0.0 || rect[1] + rect[3] <= 0.0 {
        // Nothing of it lands on the canvas: skip silently, no message.
        return;
    }
    let placed = if app.ui.tool_options.custom_shape_mode == PIXELS { fill_pixels(app, rect, mods.shift) } else { place_box(app, rect, mods.shift) };
    if placed {
        after_place(app);
    }
}

/// Shape mode: the selected shape as a vector layer in `rect`; whether one was placed.
fn place_box(app: &mut PhotocraftApp, rect: [f64; 4], keep: bool) -> bool {
    let name = app.ui.presets_ui.shape().to_string();
    let (fill, stroke) = (crate::vector_ui::fill_param(app), crate::vector_ui::stroke_param(app));
    let p = serde_json::json!({"preset": name, "rect": rect, "keepAspect": keep, "fill": fill, "stroke": stroke});
    if app.run("shape.presets.place", p).is_ok() {
        return true;
    }
    if photocraft_engine::shape_index::find_shape(&app.session, &name, None).is_none() {
        let template = "No custom shape \"{name}\": pick one in the options bar or Window › Shapes";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
    }
    false
}

const PIXELS: &str = "pixels";
const LEAF: &str = "leaf";

fn is_leaf(app: &PhotocraftApp) -> bool {
    app.ui.tool_options.custom_shape_fit == LEAF
}

/// The leaf a drag from `start` to `end` makes with the options bar's width and fullness: ⇧ turns
/// AB to a multiple of 45°, ⌥ takes the press point as the middle of AB. `None` under 1 px long.
fn leaf_of_drag(app: &PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<Leaf> {
    let o = &app.ui.tool_options;
    let leaf =
        Leaf::from_drag(Point::new(start[0], start[1]), Point::new(end[0], end[1]), mods.shift, mods.alt, f64::from(o.leaf_width), f64::from(o.leaf_fullness));
    let long_enough = (leaf.b.x - leaf.a.x).hypot(leaf.b.y - leaf.a.y) >= 1.0;
    long_enough.then_some(leaf).filter(|l| l.outline().is_some())
}

/// The selected shape bent onto the leaf being dragged (the drag preview); `None` unless the
/// Custom Shape tool is in Leaf mode.
pub fn leaf_path(app: &PhotocraftApp, tool: crate::state::Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<Path> {
    if tool != crate::state::Tool::CustomShape || !is_leaf(app) {
        return None;
    }
    let leaf = leaf_of_drag(app, start, end, mods)?;
    let shape = photocraft_engine::shape_index::find_shape(&app.session, app.ui.presets_ui.shape(), None)?;
    warp_path(&shape.path, &leaf)
}

/// End of a Leaf-mode drag: a leaf shorter than 1 px (a click) places nothing.
fn finish_leaf(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let Some(leaf) = leaf_of_drag(app, start, end, mods) else { return };
    let Some(st) = app.session.active() else { return };
    let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    let on_canvas = leaf.outline().and_then(|o| o.control_bounds()).is_some_and(|(x0, y0, x1, y1)| x1 > 0.0 && y1 > 0.0 && x0 < w && y0 < h);
    if !on_canvas {
        return;
    }
    let mut p = serde_json::json!({
        "preset": app.ui.presets_ui.shape(),
        "leaf": {"a": [leaf.a.x, leaf.a.y], "b": [leaf.b.x, leaf.b.y], "width": leaf.width, "fullness": leaf.fullness},
    });
    let command = if app.ui.tool_options.custom_shape_mode == PIXELS {
        "shape.presets.fillPixels"
    } else {
        p["fill"] = crate::vector_ui::fill_param(app);
        p["stroke"] = crate::vector_ui::stroke_param(app);
        "shape.presets.placeLeaf"
    };
    // A failure (locked layer, unknown shape) is reported in the status bar by `run`.
    if app.run(command, p).is_ok_and(|r| r["empty"] != true) {
        after_place(app);
    }
}

/// The Custom Shape options after the shape picker: Fit (Box / Leaf), the leaf's width and
/// fullness, and the size a click places.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let label = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(text).color(t.text_dim).size(12.0));
    crate::widgets::vline(ui, 22.0);
    label(ui, tl!("Fit:"));
    let o = &mut app.ui.tool_options;
    let fits = [("box".to_string(), tl!("Box")), (LEAF.to_string(), tl!("Leaf"))];
    crate::widgets::dropdown(ui, "custom-shape-fit", &mut o.custom_shape_fit, &fits, 70.0);
    if o.custom_shape_fit == LEAF {
        label(ui, tl!("Width:"));
        crate::widgets::value_field(ui, &mut o.leaf_width, 0.0..=4.0, "", 52.0);
        label(ui, tl!("Fullness:"));
        crate::widgets::value_field(ui, &mut o.leaf_fullness, 0.1..=3.0, "", 52.0);
    } else {
        label(ui, tl!("Click size:"));
        crate::widgets::value_field(ui, &mut o.custom_shape_default_size, 0.0..=10000.0, "px", 62.0);
    }
    crate::widgets::vline(ui, 22.0);
    label(ui, tl!("Randomize every:"));
    let mut every = o.shape_random_every as f32;
    if crate::widgets::value_field(ui, &mut every, 0.0..=1000.0, "", 46.0).changed() {
        o.shape_random_every = every.round().clamp(0.0, 1000.0) as u32;
    }
    if o.shape_random_every > 0 {
        let sources = [(SOURCE_LIBRARY.to_string(), tl!("Library")), (SOURCE_GROUP.to_string(), tl!("Group")), (SOURCE_SET.to_string(), tl!("Set"))];
        crate::widgets::dropdown(ui, "custom-shape-random-source", &mut o.shape_random_source, &sources, 80.0);
    }
    if o.shape_random_every > 0 && o.shape_random_source == SOURCE_SET {
        let cur = app.ui.presets_ui.shape().to_string();
        if ui.small_button(tl!("Add current")).clicked() && !o.shape_random_set.contains(&cur) && o.shape_random_set.len() < MAX_SET {
            o.shape_random_set.push(cur);
        }
        if ui.small_button(tl!("Clear set")).clicked() {
            o.shape_random_set.clear();
        }
        label(ui, &format!("{}", o.shape_random_set.len()));
    }
}

const SOURCE_LIBRARY: &str = "library";
const SOURCE_GROUP: &str = "group";
const SOURCE_SET: &str = "set";
/// Most shapes in the random set.
const MAX_SET: usize = 10_000;

/// Where Randomize is: shapes placed since the last change and the generator. Not saved, and equal
/// to any other so that counting a shape does not look like a change to the options.
#[derive(Clone, Debug, Default)]
pub struct RandomizeState {
    placed: u32,
    rng: Option<u64>,
}

impl PartialEq for RandomizeState {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// splitmix64: small, fast, and the same sequence for the same seed.
fn next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// An index below `len` other than `avoid` (when there is another one): `None` for no shapes.
pub fn pick(len: usize, avoid: Option<usize>, rng: &mut u64) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let i = (next_u64(rng) % len as u64) as usize;
    Some(if len > 1 && avoid == Some(i) { (i + 1) % len } else { i })
}

/// The names Randomize may pick from, by name only: nothing is parsed.
fn candidates(app: &PhotocraftApp) -> Vec<String> {
    let o = &app.ui.tool_options;
    let index = photocraft_engine::shape_index::index(&app.session);
    let cur = app.ui.presets_ui.shape();
    match o.shape_random_source.as_str() {
        SOURCE_SET => o.shape_random_set.iter().filter(|n| index.iter().any(|g| g.items.contains(n))).cloned().collect(),
        SOURCE_GROUP => match index.iter().find(|g| g.items.iter().any(|n| n == cur)) {
            Some(g) => g.items.clone(),
            None => index.into_iter().flat_map(|g| g.items).collect(),
        },
        _ => index.into_iter().flat_map(|g| g.items).collect(),
    }
}

/// Count a placed shape; every `shape_random_every`th one switches the tool to a random shape.
pub fn after_place(app: &mut PhotocraftApp) {
    let o = &mut app.ui.tool_options;
    if o.shape_random_every == 0 {
        return;
    }
    o.shape_random.placed = o.shape_random.placed.saturating_add(1);
    if o.shape_random.placed < o.shape_random_every {
        return;
    }
    o.shape_random.placed = 0;
    let names = candidates(app);
    let cur = app.ui.presets_ui.shape().to_string();
    let avoid = names.iter().position(|n| *n == cur);
    let o = &mut app.ui.tool_options;
    let seed = o.shape_random_seed;
    let rng = o.shape_random.rng.get_or_insert_with(|| seed.unwrap_or_else(entropy));
    if let Some(i) = pick(names.len(), avoid, rng)
        && let Some(name) = names.get(i)
    {
        app.ui.presets_ui.custom_shape = name.clone();
    }
}

/// A seed that differs from run to run (a fixed one on targets without a random source).
fn entropy() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

/// Pixels mode: paint the shape on the active pixel layer (`shape.presets.fillPixels`).
fn fill_pixels(app: &mut PhotocraftApp, rect: [f64; 4], keep: bool) -> bool {
    let name = app.ui.presets_ui.shape().to_string();
    // The command reports a failure (locked layer, unknown shape) in the status bar.
    app.run("shape.presets.fillPixels", serde_json::json!({"preset": name, "rect": rect, "keepAspect": keep})).is_ok_and(|r| r["empty"] != true)
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Load,
    Replace,
    Save,
}

/// Load / Replace / Save Shapes under the Shapes panel: the library lives in the session, the
/// file is a `.csh`.
pub fn library_row(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for (act, label) in [(Act::Load, tl!("Load Shapes…")), (Act::Replace, tl!("Replace Shapes…")), (Act::Save, tl!("Save Shapes…"))] {
            if ui.small_button(label).clicked() {
                library_action(app, act);
            }
        }
    });
}

fn library_action(app: &mut PhotocraftApp, act: Act) {
    let r = match act {
        Act::Load | Act::Replace => app.pick_file_bytes_filtered(&["csh"], move |app, name, bytes| {
            let mut p = crate::swatches_ui::source(&name, &bytes);
            p["mode"] = serde_json::json!(if act == Act::Replace { "replace" } else { "append" });
            let shown = crate::file_open::display_name(&name);
            if let Some(stem) = std::path::Path::new(&shown).file_stem().and_then(|s| s.to_str()) {
                p["name"] = serde_json::json!(stem);
            }
            let r = app.run("shape.presets.cshLoad", p)?;
            // After Replace the built-in shape is gone: start on the file's first one.
            if act == Act::Replace
                && let Some(first) = r["first"].as_str()
            {
                app.ui.presets_ui.custom_shape = first.to_string();
            }
            let template = "Loaded shapes from {file}";
            app.ui.status_error = false;
            app.ui.status = crate::i18n::fmt(tl!(template), &[("file", &shown)]);
            Ok(r)
        }),
        Act::Save => app.run("shape.presets.cshSave", serde_json::json!({})).and_then(|r| {
            let bytes = r["data"].as_str().and_then(photocraft_engine::paint::tile::b64_decode).ok_or("the export returned no data")?;
            app.pick_save("Shapes.csh", move |app, path| {
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&path, &bytes)?;
                let template = "Saved shapes to {file}";
                app.ui.status_error = false;
                app.ui.status = crate::i18n::fmt(tl!(template), &[("file", &crate::file_open::display_name(&path))]);
                Ok(serde_json::json!({"path": path}))
            })
        }),
    };
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status_error = true;
        app.ui.status = e;
    }
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
