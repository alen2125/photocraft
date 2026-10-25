//! Custom Shape tool additions kept out of the shared tool files: what a click or an off-canvas
//! drag does (fork-local, see MY_CHANGES.md).

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_doc::Path;
use photocraft_geom::Point;
use photocraft_vector::warp::{Leaf, clip_to_leaf, warp_path};

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
    match app.run("shape.presets.place", p) {
        Ok(_) => return true,
        Err(e) => note_if_too_detailed(app, &e),
    }
    if photocraft_engine::shape_index::find_shape(&app.session, &name, None).is_none() {
        let template = "No custom shape \"{name}\": pick one in the options bar or Window › Shapes";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
    }
    false
}

const PIXELS: &str = "pixels";
const LEAF: &str = "leaf";
const ZIPPER: &str = "zipper";

/// Leaf or zipper: the drag is two points, A and B, and the shape goes in the leaf they make.
fn is_leaf(app: &PhotocraftApp) -> bool {
    matches!(app.ui.tool_options.custom_shape_fit.as_str(), LEAF | ZIPPER)
}

/// The zipper: the shape is laid in the leaf undeformed and cut off by the leaf's outline.
fn is_zipper(app: &PhotocraftApp) -> bool {
    app.ui.tool_options.custom_shape_fit == ZIPPER
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
    let (shape, _) = proxy_path(app)?;
    if is_zipper(app) { clip_to_leaf(&shape, &leaf) } else { warp_path(&shape, &leaf) }
}

/// The thinned shape kept for the drag preview: its key (name, library revision), the path and
/// whether the shape is dense.
type Proxy = ((String, u64), Arc<Path>, bool);

/// The selected shape as the drag preview draws it, and whether it is a dense (traced) drawing: a
/// dense shape is thinned to a couple of points per thousandth of its size, which looks the same
/// on screen and draws in a fraction of the time. Kept for as long as the shape and the shapes
/// library stay the same.
fn proxy_path(app: &PhotocraftApp) -> Option<(Arc<Path>, bool)> {
    static PROXY: Mutex<Option<Proxy>> = Mutex::new(None);
    let name = app.ui.presets_ui.shape();
    let key = (name.to_string(), app.session.presets.rev);
    if let Some((k, path, dense)) = PROXY.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
        && *k == key
    {
        return Some((path.clone(), *dense));
    }
    let full = photocraft_engine::shape_index::shape_path(&app.session, name, None)?;
    let dense = full.subpaths.iter().map(|s| s.knots.len()).sum::<usize>() > photocraft_vector::thin::DENSE;
    let proxy = match full.control_bounds() {
        Some((x0, y0, x1, y1)) if dense => {
            let diag = (x1 - x0).hypot(y1 - y0);
            Arc::new(photocraft_vector::thin::thin(&photocraft_vector::thin::flatten(&full, diag * 1e-3), diag * 2e-3))
        }
        _ => full,
    };
    *PROXY.lock().unwrap_or_else(PoisonError::into_inner) = Some((key, proxy.clone(), dense));
    Some((proxy, dense))
}

/// Longest side (px) the picture of a dense shape is drawn at for the drag preview.
const PREVIEW_SIDE: f64 = 512.0;

/// Cells per side of the mesh a leaf preview is drawn with.
const MESH: usize = 24;

/// The drag preview of a dense shape: the shape is drawn once into a picture, and the picture is
/// stretched into the dragged box (Box mode) or bent onto the dragged leaf by a mesh (Leaf mode),
/// so each pointer move costs nothing. `true` when it handled the preview (other shapes keep the
/// exact outline-and-fill preview).
pub fn draw_dense_preview(
    app: &PhotocraftApp,
    painter: &egui::Painter,
    xf: &crate::canvas::ViewXform,
    tool: crate::state::Tool,
    start: [f64; 2],
    end: [f64; 2],
    mods: egui::Modifiers,
) -> bool {
    if tool != crate::state::Tool::CustomShape {
        return false;
    }
    let Some((path, true)) = proxy_path(app) else { return false };
    let leaf = if is_leaf(app) { leaf_of_drag(app, start, end, mods) } else { None };
    let boxed = if leaf.is_none() { crate::vector_ui::shape_bounds(&app.ui.tool_options, tool, start, end, mods) } else { None };
    if is_leaf(app) && leaf.is_none() || !is_leaf(app) && boxed.is_none() {
        return true;
    }
    let fg = app.session.tools.foreground;
    let id = egui::Id::new("custom-shape-dense-preview");
    let key = (app.ui.presets_ui.shape().to_string(), app.session.presets.rev, fg.map(f32::to_bits));
    let cached = painter.ctx().data(|d| d.get_temp::<(Key, egui::TextureHandle, f32)>(id));
    let (tex, aspect) = match cached.filter(|c| c.0 == key) {
        Some((_, tex, aspect)) => (tex, aspect),
        None => {
            let Some((bx0, by0, bx1, by1)) = path.control_bounds() else { return true };
            let (w, h) = ((bx1 - bx0).max(1e-6), (by1 - by0).max(1e-6));
            let s = PREVIEW_SIDE / w.max(h);
            let (rw, rh) = (((w * s).ceil() as i32).clamp(1, 4096), ((h * s).ceil() as i32).clamp(1, 4096));
            let fitted = photocraft_engine::presets::shapes::fit(&path, [0.0, 0.0, f64::from(rw), f64::from(rh)], false);
            let cov = photocraft_vector::path_coverage(&fitted, photocraft_geom::Rect::new(0, 0, rw, rh));
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let pixels = cov.iter().map(|a| egui::Color32::from_rgba_unmultiplied(q(fg[0]), q(fg[1]), q(fg[2]), q(a * fg[3]))).collect();
            let image = egui::ColorImage::new([rw as usize, rh as usize], pixels);
            let tex = painter.ctx().load_texture("custom-shape-dense", image, egui::TextureOptions::LINEAR);
            let aspect = (w / h) as f32;
            painter.ctx().data_mut(|d| d.insert_temp(id, (key, tex.clone(), aspect)));
            (tex, aspect)
        }
    };
    if let Some(leaf) = leaf {
        let mut mesh = egui::Mesh::with_texture(tex.id());
        for j in 0..=MESH {
            for i in 0..=MESH {
                let (u, v) = (i as f64 / MESH as f64, j as f64 / MESH as f64);
                let Some(p) = leaf.at(u, v) else { return true };
                // Leaf: the picture follows the mesh. Zipper: it lies straight in the leaf's box and
                // the mesh, which covers only the leaf, shows the part inside.
                let uv = if is_zipper(app) { leaf.box_coords(p).map_or((u, v), |(bu, bv)| (bu.clamp(0.0, 1.0), bv.clamp(0.0, 1.0))) } else { (u, v) };
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: xf.to_screen(p.x as f32, p.y as f32),
                    uv: egui::pos2(uv.0 as f32, uv.1 as f32),
                    color: egui::Color32::WHITE,
                });
            }
        }
        for j in 0..MESH {
            for i in 0..MESH {
                let a = (j * (MESH + 1) + i) as u32;
                let b = a + 1;
                let c = a + (MESH + 1) as u32;
                mesh.indices.extend([a, b, c, b, c + 1, c]);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
        return true;
    }
    let Some([x0, y0, x1, y1]) = boxed else { return true };
    let (a, b) = (xf.to_screen(x0 as f32, y0 as f32), xf.to_screen(x1 as f32, y1 as f32));
    let mut dest = egui::Rect::from_two_pos(a, b);
    if mods.shift {
        // ⇧ keeps the shape's proportions, centred in the box.
        let size = dest.size();
        let fit = if size.x / size.y.max(1e-3) > aspect { egui::vec2(size.y * aspect, size.y) } else { egui::vec2(size.x, size.x / aspect.max(1e-3)) };
        dest = egui::Rect::from_center_size(dest.center(), fit);
    }
    painter.image(tex.id(), dest, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    true
}

/// The outline of the leaf being dragged: the guide drawn over a leaf preview instead of the
/// outline of the bent shape (a traced drawing is thousands of slivers, which a thick line turns
/// into spikes). `None` unless the Custom Shape tool is in Leaf mode.
pub fn leaf_outline(app: &PhotocraftApp, tool: crate::state::Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<Path> {
    if tool != crate::state::Tool::CustomShape || !is_leaf(app) {
        return None;
    }
    leaf_of_drag(app, start, end, mods)?.outline()
}

type Key = (String, u64, [u32; 4]);

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
        "leaf": {"a": [leaf.a.x, leaf.a.y], "b": [leaf.b.x, leaf.b.y], "width": leaf.width, "fullness": leaf.fullness, "clip": is_zipper(app)},
    });
    let command = if app.ui.tool_options.custom_shape_mode == PIXELS {
        "shape.presets.fillPixels"
    } else {
        p["fill"] = crate::vector_ui::fill_param(app);
        p["stroke"] = crate::vector_ui::stroke_param(app);
        "shape.presets.placeLeaf"
    };
    // A failure (locked layer, unknown shape) is reported in the status bar by `run`.
    match app.run(command, p) {
        Ok(r) if r["empty"] != true => after_place(app),
        Ok(_) => {}
        Err(e) => note_if_too_detailed(app, &e),
    }
}

/// A traced drawing has tens of thousands of points; a vector shape layer holds at most 4096.
/// Say what to do about it instead of the engine's bare limit.
fn note_if_too_detailed(app: &mut PhotocraftApp, error: &str) {
    if error.contains("exceeds 4096") {
        app.ui.status = tl!("This shape has too many points for a vector layer (4096 at most): use Pixels mode").to_string();
        app.ui.status_error = true;
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
    let fits = [("box".to_string(), tl!("Box")), (LEAF.to_string(), tl!("Leaf")), (ZIPPER.to_string(), tl!("Zipper"))];
    crate::widgets::dropdown(ui, "custom-shape-fit", &mut o.custom_shape_fit, &fits, 70.0);
    if matches!(o.custom_shape_fit.as_str(), LEAF | ZIPPER) {
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

/// The Custom Shape options the user can give keys to (listed in `SHELL_COMMANDS`, so Edit ›
/// Keyboard Shortcuts shows them; none has a key by default).
const COMMANDS: &[&str] = &[
    "ui.customShape.mode.shape",
    "ui.customShape.mode.pixels",
    "ui.customShape.mode.toggle",
    "ui.customShape.fit.box",
    "ui.customShape.fit.leaf",
    "ui.customShape.fit.zipper",
    "ui.customShape.fit.cycle",
    "ui.customShape.random.toggle",
];

/// Is `id` one of the Custom Shape option commands? They work whatever tool is active.
pub fn handles(id: &str) -> bool {
    COMMANDS.contains(&id)
}

/// Change a Custom Shape option from a key, and say what it is now in the status bar (the options
/// bar shows it only while the tool is selected). `None` for any other command.
pub fn invoke(app: &mut PhotocraftApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    if !handles(id) {
        return None;
    }
    let o = &mut app.ui.tool_options;
    let said = match id {
        "ui.customShape.mode.shape" => {
            o.custom_shape_mode = "shape".into();
            tl!("Custom Shape: Shape layers").to_string()
        }
        "ui.customShape.mode.pixels" => {
            o.custom_shape_mode = PIXELS.into();
            tl!("Custom Shape: Pixels").to_string()
        }
        "ui.customShape.mode.toggle" => {
            let pixels = o.custom_shape_mode != PIXELS;
            o.custom_shape_mode = if pixels { PIXELS } else { "shape" }.into();
            if pixels { tl!("Custom Shape: Pixels") } else { tl!("Custom Shape: Shape layers") }.to_string()
        }
        "ui.customShape.fit.box" => {
            o.custom_shape_fit = "box".into();
            tl!("Custom Shape: Box").to_string()
        }
        "ui.customShape.fit.leaf" => {
            o.custom_shape_fit = LEAF.into();
            tl!("Custom Shape: Leaf").to_string()
        }
        "ui.customShape.fit.zipper" => {
            o.custom_shape_fit = ZIPPER.into();
            tl!("Custom Shape: Zipper").to_string()
        }
        "ui.customShape.fit.cycle" => {
            let (next, said) = match o.custom_shape_fit.as_str() {
                "box" => (LEAF, tl!("Custom Shape: Leaf")),
                LEAF => (ZIPPER, tl!("Custom Shape: Zipper")),
                _ => ("box", tl!("Custom Shape: Box")),
            };
            o.custom_shape_fit = next.into();
            said.to_string()
        }
        _ => {
            // Randomize off remembers its N, so on brings the same one back.
            if o.shape_random_every > 0 {
                o.shape_random.last_every = o.shape_random_every;
                o.shape_random_every = 0;
                o.shape_random.placed = 0;
                tl!("Custom Shape: Randomize off").to_string()
            } else {
                o.shape_random_every = o.shape_random.last_every.max(1);
                let template = "Custom Shape: Randomize every {n}";
                crate::i18n::fmt(tl!(template), &[("n", &o.shape_random_every.to_string())])
            }
        }
    };
    app.ui.status = said;
    app.ui.status_error = false;
    Some(Ok(serde_json::Value::Null))
}

/// Where Randomize is: shapes placed since the last change and the generator. Not saved, and equal
/// to any other so that counting a shape does not look like a change to the options.
#[derive(Clone, Debug, Default)]
pub struct RandomizeState {
    placed: u32,
    rng: Option<u64>,
    /// The N Randomize had when a key turned it off.
    last_every: u32,
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
        prewarm(app, name);
    }
}

/// Read the shape just picked from its library in the background, so the next drag does not wait
/// for the file (a traced drawing takes about 50 ms to read and decode). Not on wasm: no threads.
fn prewarm(app: &PhotocraftApp, name: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (lazy, name) = (app.session.presets.lazy_shapes.clone(), name.to_string());
        // A thread that cannot start only costs the head start.
        let _ = std::thread::Builder::new().name("shape-prewarm".into()).spawn(move || {
            let _ = lazy.path(None, &name);
        });
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, name);
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
