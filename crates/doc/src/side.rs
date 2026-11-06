//! Parallel layers: layers flagged [`Layer::side`] are left out of the document's own composite
//! and shown on a second canvas beside it. That canvas shows every visible layer up to and
//! including the topmost parallel layer, so a flower (layer 1), a parallel sun (layer 2) and grass
//! (layer 3) give a document of flower + grass and a side canvas of flower + sun.
//!
//! Both views are plain copies of the document with some layers hidden, so the normal compositor
//! renders them (adjustments, effects and blend modes included).

use crate::{DocId, Document, Layer, LayerPath};

/// Set on the id of [`Document::side_view`], so a renderer's per-document caches keep the side
/// canvas apart from the document itself. Ids are counted up from 1 and never reach this bit.
const SIDE_ID_BIT: u64 = 1 << 63;

impl Document {
    /// Whether any layer, visible or not, is flagged parallel. Allocation-free, for per-frame use.
    pub fn has_side_flag(&self) -> bool {
        fn any(layers: &[Layer]) -> bool {
            layers.iter().any(|l| l.side || l.children().is_some_and(any))
        }
        any(&self.layers)
    }

    /// Whether any visible layer is a parallel layer (the side canvas is shown).
    pub fn has_side_layers(&self) -> bool {
        self.walk().iter().any(|(_, _, l)| l.side && l.visible)
    }

    /// The path of the topmost visible parallel layer, in the bottom-to-top walk order.
    fn top_side_path(&self) -> Option<LayerPath> {
        self.walk().into_iter().rev().find(|(_, _, l)| l.side && l.visible).map(|(p, _, _)| p)
    }

    /// The document as it is shown on the main canvas: parallel layers hidden. `None` when it has
    /// none, so callers keep using the document itself.
    pub fn main_view(&self) -> Option<Document> {
        if !self.walk().iter().any(|(_, _, l)| l.side) {
            return None;
        }
        let mut copy = self.clone();
        let paths: Vec<LayerPath> = self.walk().into_iter().filter(|(_, _, l)| l.side).map(|(p, _, _)| p).collect();
        for p in paths {
            if let Some(l) = copy.layer_at_mut(&p) {
                l.visible = false;
            }
        }
        Some(copy)
    }

    /// The document as the side canvas shows it: everything above the topmost parallel layer
    /// hidden (its own contents, if it is a group, stay). `None` without a visible parallel
    /// layer. The copy has its own [`DocId`], stable for the document, for renderer caches.
    pub fn side_view(&self) -> Option<Document> {
        let top = self.top_side_path()?;
        let mut copy = self.clone();
        copy.id = DocId(self.id.0 | SIDE_ID_BIT);
        let walk = self.walk();
        let pos = walk.iter().position(|(p, _, _)| *p == top)?;
        // The walk lists a group before its children, so "after" the layer in composite order is
        // everything later in the walk except the layer's own contents.
        let later: Vec<LayerPath> = walk.get(pos + 1..).unwrap_or_default().iter().filter(|(p, _, _)| !p.starts_with(&top)).map(|(p, _, _)| p.clone()).collect();
        for p in later {
            if let Some(l) = copy.layer_at_mut(&p) {
                l.visible = false;
            }
        }
        Some(copy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Size;
    use photocraft_color::{ColorMode, SampleType};

    fn doc_with(names: &[(&str, bool)]) -> Document {
        let mut d = Document::new("t", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        for (n, side) in names {
            let mut l = Layer::raster(*n, d.pixel_format());
            l.side = *side;
            d.layers.push(l);
        }
        d
    }

    fn shown(d: &Document) -> Vec<String> {
        d.walk().into_iter().filter(|(_, _, l)| l.visible).map(|(_, _, l)| l.name.clone()).collect()
    }

    #[test]
    fn flower_sun_grass() {
        let d = doc_with(&[("flower", false), ("sun", true), ("grass", false)]);
        assert!(d.has_side_layers());
        assert_eq!(shown(&d.main_view().unwrap()), ["flower", "grass"]);
        assert_eq!(shown(&d.side_view().unwrap()), ["flower", "sun"]);
    }

    #[test]
    fn no_side_layers_means_no_views() {
        let d = doc_with(&[("a", false), ("b", false)]);
        assert!(!d.has_side_layers());
        assert!(d.main_view().is_none());
        assert!(d.side_view().is_none());
    }

    #[test]
    fn hidden_side_layer_has_no_canvas() {
        let mut d = doc_with(&[("a", false), ("sun", true)]);
        d.layers[1].visible = false;
        assert!(d.side_view().is_none());
        assert_eq!(shown(&d.main_view().unwrap()), ["a"]);
    }

    #[test]
    fn side_view_ends_at_the_topmost_side_layer() {
        let d = doc_with(&[("a", false), ("s1", true), ("b", false), ("s2", true), ("c", false)]);
        assert_eq!(shown(&d.side_view().unwrap()), ["a", "s1", "b", "s2"]);
        assert_eq!(shown(&d.main_view().unwrap()), ["a", "b", "c"]);
    }

    #[test]
    fn side_layer_inside_a_group() {
        let mut d = doc_with(&[("a", false)]);
        let mut inner = Layer::raster("sun", d.pixel_format());
        inner.side = true;
        let top = Layer::raster("above", d.pixel_format());
        d.layers.push(Layer::group("g", vec![Layer::raster("in", d.pixel_format()), inner, top]));
        d.layers.push(Layer::raster("grass", d.pixel_format()));
        assert_eq!(shown(&d.side_view().unwrap()), ["a", "g", "in", "sun"]);
        assert_eq!(shown(&d.main_view().unwrap()), ["a", "g", "in", "above", "grass"]);
    }

    #[test]
    fn side_group_keeps_its_children() {
        let mut d = doc_with(&[("a", false)]);
        let mut g = Layer::group("g", vec![Layer::raster("x", d.pixel_format()), Layer::raster("y", d.pixel_format())]);
        g.side = true;
        d.layers.push(g);
        d.layers.push(Layer::raster("grass", d.pixel_format()));
        assert_eq!(shown(&d.side_view().unwrap()), ["a", "g", "x", "y"]);
    }

    #[test]
    fn side_view_has_its_own_stable_id() {
        let d = doc_with(&[("a", false), ("s", true)]);
        let (v1, v2) = (d.side_view().unwrap(), d.side_view().unwrap());
        assert_ne!(v1.id, d.id);
        assert_eq!(v1.id, v2.id);
        assert_eq!(d.main_view().unwrap().id, d.id);
    }
}
