//! The side canvas of parallel layers (`photocraft_doc::Document::side_view`): a second,
//! document-sized canvas to the left of the document, on the same zoom and pan, showing every
//! layer up to the topmost parallel one. The document's own canvas leaves parallel layers out
//! ([`main_view`]). Tools work on the side canvas through [`input_xf`], which maps its pixels to
//! document pixels, so painting, pasting and the Eyedropper act on the parallel layer.
//!
//! The side canvas is drawn on the CPU canvas path from a reduced composite, cached per document
//! snapshot, so it never touches the GPU canvas's per-document state.

use std::sync::{Arc, Weak};

use egui::{Color32, Stroke};
use photocraft_doc::Document;

use crate::PhotocraftApp;
use crate::canvas::{self, ViewXform};

/// Longest side, in texels, of the side canvas's texture.
const TEXTURE_MAX: u32 = 2048;

/// A document's side canvas texture, and what it was rendered from.
pub(crate) struct SideTexture {
    src: Weak<Document>,
    revision: u64,
    /// The preview key (with the display folded in) the texture shows.
    shown_key: u64,
    texture: egui::TextureHandle,
}

/// The empty space, in document pixels, between the side canvas and the document.
pub(crate) fn gap(width: u32) -> f32 {
    (width as f32 * 0.04).max(24.0).round()
}

/// The document-pixel x of the side canvas's left edge (it spans one document width from there).
pub(crate) fn origin_x(size: [u32; 2]) -> f32 {
    -(size[0] as f32 + gap(size[0]))
}

/// How much wider than the document the area to show is, and how far its centre lies left of the
/// document's: both zero for a document without a visible parallel layer.
pub(crate) fn span(doc: &Document) -> (f32, f32) {
    if !doc.has_side_layers() {
        return (0.0, 0.0);
    }
    let extra = doc.size.width as f32 + gap(doc.size.width);
    (extra, -extra / 2.0)
}

/// `xf` looking at the side canvas as if it were the document: a document point through it is the
/// point of the side canvas under the same screen pixel.
pub(crate) fn shifted(xf: &ViewXform, size: [u32; 2]) -> ViewXform {
    ViewXform { center: [xf.center[0] - origin_x(size), xf.center[1]], ..*xf }
}

/// Whether screen point `p` lies on the side canvas.
pub(crate) fn contains(xf: &ViewXform, size: [u32; 2], p: egui::Pos2) -> bool {
    let [x, y] = shifted(xf, size).to_doc(p);
    x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 && x < f64::from(size[0]) && y < f64::from(size[1])
}

/// The mapping tools take their document points through this frame, and whether the pointer is on
/// the side canvas: that is, when a gesture began there (until it ends), or the pointer hovers
/// there with none going on. Always `xf` itself without a visible parallel layer.
pub(crate) fn input_xf(ctx: &egui::Context, xf: &ViewXform, doc: &Document, canvas_rect: egui::Rect) -> (ViewXform, bool) {
    if !doc.has_side_layers() {
        return (*xf, false);
    }
    let size = [doc.size.width, doc.size.height];
    let at = ctx.input(|i| {
        let p = &i.pointer;
        if p.any_down() || p.any_released() { p.press_origin() } else { p.hover_pos() }
    });
    match at.filter(|p| canvas_rect.contains(*p)) {
        Some(p) if contains(xf, size, p) => (shifted(xf, size), true),
        _ => (*xf, false),
    }
}

/// Make the topmost visible parallel layer the active one, unless a parallel layer already is:
/// drawing on the side canvas paints there, whichever layer was selected before.
pub(crate) fn activate_side_layer(app: &mut PhotocraftApp) {
    let Some(st) = app.session.active() else { return };
    let active_is_side = st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.side);
    if active_is_side {
        return;
    }
    let Some(top) = st.doc.walk().into_iter().rev().find(|(_, _, l)| l.side && l.visible).map(|(_, _, l)| l.id) else { return };
    if let Err(e) = app.run("layer.select", serde_json::json!({"layer": top.0})) {
        log::warn!("side canvas: selecting the parallel layer failed: {e}");
    }
}

/// The document as the main canvas shows it: `doc` without its parallel layers, built once per
/// snapshot and key rather than on each of the several calls a frame.
pub(crate) fn main_view(app: &mut PhotocraftApp, doc: Arc<Document>, key: u64) -> (Arc<Document>, u64) {
    if !doc.has_side_flag() {
        app.side_memo.remove(&doc.id);
        return (doc, key);
    }
    if let Some((src, k, shown)) = app.side_memo.get(&doc.id)
        && *k == key
        && src.ptr_eq(&Arc::downgrade(&doc))
    {
        return (shown.clone(), key);
    }
    let Some(shown) = doc.main_view().map(Arc::new) else { return (doc, key) };
    app.side_memo.retain(|_, (src, _, _)| src.strong_count() > 0);
    app.side_memo.insert(doc.id, (Arc::downgrade(&doc), key, shown.clone()));
    (shown, key)
}

/// The side canvas texture of document `idx`, brought up to date: redrawn over the damage of the
/// edit or live stroke only (as the document's own CPU canvas is), whole when it can't tell;
/// `None` (and the cache dropped) without a visible parallel layer.
fn ensure_texture(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, output: Option<u32>) -> Option<egui::TextureId> {
    let (id, revision, last_damage) = {
        let st = app.session.documents().get(idx)?;
        (st.doc.id, st.revision, st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(canvas::effect_reach(&st.doc.layers)) }))
    };
    let (src, key) = canvas::display_doc_raw(app, idx);
    let Some(side) = src.side_view() else {
        app.side_textures.remove(&id);
        return None;
    };
    let (display, display_key) = canvas::canvas_display(app, &side, output);
    let shown_key = key ^ display_key;
    let seen = app.side_textures.get(&id).map(|t| (t.revision, t.shown_key));
    if let Some(t) = app.side_textures.get(&id)
        && t.revision == revision
        && t.shown_key == shown_key
        && t.src.ptr_eq(&Arc::downgrade(&src))
    {
        return Some(t.texture.id());
    }
    let damage = seen.and_then(|seen| canvas::damage_since(app, idx, seen, (revision, key), display_key, last_damage));
    let size = [side.size.width, side.size.height];
    let factor = canvas::texture_factor(size, canvas::texture_side(ctx).min(TEXTURE_MAX));
    let (w, h) = ((size[0] / factor).max(1), (size[1] / factor).max(1));
    let options = canvas::canvas_texture_options(factor);
    let mut cached = app.side_textures.remove(&id).filter(|t| t.texture.size() == [w as usize, h as usize]);
    let texture = match (damage, cached.as_mut()) {
        (Some(d), Some(t)) => {
            // Only what changed, with the values the whole reduction gives it.
            let buf = photocraft_compose::render_reduced_damage(&side, w, h, d);
            let r = buf.rect;
            if !r.is_empty() {
                t.texture.set_partial([r.x0 as usize, r.y0 as usize], canvas::display_image(display.as_deref(), &buf), options);
            }
            cached.map(|t| t.texture)
        }
        (_, Some(t)) => {
            t.texture.set(canvas::display_image(display.as_deref(), &photocraft_compose::render_reduced(&side, w, h)), options);
            cached.map(|t| t.texture)
        }
        (_, None) => None,
    };
    let texture = texture.unwrap_or_else(|| {
        let image = canvas::display_image(display.as_deref(), &photocraft_compose::render_reduced(&side, w, h));
        ctx.load_texture(format!("side-canvas-{}", id.0), image, options)
    });
    let shown = texture.id();
    app.side_textures.insert(id, SideTexture { src: Arc::downgrade(&src), revision, shown_key, texture });
    Some(shown)
}

/// Draw the side canvas left of the document, under the same view.
pub(crate) fn paint(app: &mut PhotocraftApp, ctx: &egui::Context, painter: &egui::Painter, xf: &ViewXform, idx: usize, doc: &Document, output: Option<u32>) {
    if !doc.has_side_layers() {
        app.side_textures.remove(&doc.id);
        return;
    }
    let Some(texture) = ensure_texture(app, ctx, idx, output) else { return };
    let checkerboard = app.session.prefs().transparency_and_gamut.square().map(|square| (canvas::checker(app, ctx), square));
    let rotated = xf.rotation.abs() > 0.05;
    let side = shifted(xf, [doc.size.width, doc.size.height]);
    canvas::paint_cpu_document(painter, &side, doc.bounds(), rotated, checkerboard, Some(texture));
    if !rotated {
        let outline = Stroke::new(1.0, Color32::from_black_alpha(90));
        painter.rect_stroke(side.doc_rect(doc.bounds()), 0, outline, egui::StrokeKind::Outside);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Layer, Size};

    fn xf() -> ViewXform {
        ViewXform {
            rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 600.0)),
            zoom: 1.0,
            center: [50.0, 40.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        }
    }

    #[test]
    fn side_points_map_to_document_points() {
        let size = [100, 80];
        let xf = xf();
        // The document's own left edge on screen is the side canvas's right edge plus the gap.
        let doc_left = xf.to_screen(0.0, 0.0);
        let gap = gap(100);
        let side_right = egui::pos2(doc_left.x - gap - 1.0, doc_left.y + 10.0);
        assert!(contains(&xf, size, side_right));
        assert!(!contains(&xf, size, egui::pos2(doc_left.x + 5.0, doc_left.y + 10.0)));
        let [x, y] = shifted(&xf, size).to_doc(side_right);
        assert!((x - 99.0).abs() < 1e-3 && (y - 10.0).abs() < 1e-3, "{x} {y}");
    }

    #[test]
    fn no_span_without_parallel_layers() {
        let mut d = Document::new("t", Size::new(100, 80), ColorMode::Rgb, SampleType::U8);
        d.layers.push(Layer::raster("a", d.pixel_format()));
        assert_eq!(span(&d), (0.0, 0.0));
        d.layers[0].side = true;
        let (extra, shift) = span(&d);
        assert_eq!(extra, 100.0 + gap(100));
        assert_eq!(shift, -extra / 2.0);
    }
}
