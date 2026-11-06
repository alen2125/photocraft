//! What the lasso Draw and Shading modes (fork-local, see MY_CHANGES.md) need from the engine:
//! the dominant colours under the selection of the visible image, and a fill with the mean of two
//! of them.
//!
//! A colour is dominant when it is the peak of a histogram of the selected pixels (5 bits per
//! channel). Every histogram bin whose mean lies near the peak joins it, so the smooth gradient of
//! a photo counts as one colour instead of many neighbouring bins. The second colour of Shading is
//! the peak of the bins that are clearly not the first.

use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Bins per channel (5 bits).
const LEVELS: usize = 32;
/// Selection coverage above which a pixel counts.
const COVERED: f32 = 0.5;
/// Bins whose mean is closer than this (RGB, 0..1) to a peak belong to its colour.
const JOIN: f32 = 0.12;
/// A second colour must be at least this far (RGB, 0..1) from the first.
const APART: f32 = 0.19;
/// Rows composited at a time, so a huge selection never needs the whole image in memory.
const BAND: i32 = 256;

#[derive(Clone, Copy, Default)]
struct Bin {
    count: u64,
    sum: [f64; 3],
}

impl Bin {
    fn mean(&self) -> [f32; 3] {
        let n = self.count.max(1) as f64;
        std::array::from_fn(|c| (self.sum[c] / n) as f32)
    }
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn bin_of(p: &[f32; 4]) -> usize {
    let q = |v: f32| ((v.clamp(0.0, 1.0) * (LEVELS as f32 - 1.0)).round() as usize).min(LEVELS - 1);
    (q(p[0]) * LEVELS + q(p[1])) * LEVELS + q(p[2])
}

/// The histogram of the visible pixels `sel` covers.
fn histogram(doc: &Document, sel: &Surface) -> Result<Vec<Bin>> {
    let area = sel.content_bounds().intersect(&doc.bounds());
    if area.is_empty() {
        return Err(EngineError::Other("the selection is empty".into()));
    }
    let mut hist = vec![Bin::default(); LEVELS * LEVELS * LEVELS];
    let w = area.width() as usize;
    let mut coverage = vec![1.0f32; w.max(1)];
    let mut y = area.y0;
    while y < area.y1 {
        let y1 = y.saturating_add(BAND).min(area.y1);
        let band = Rect::new(area.x0, y, area.x1, y1);
        let buf = photocraft_compose::render(doc, band);
        for (j, row) in buf.px.chunks(w.max(1)).enumerate() {
            sel.sample_row_strided(y.saturating_add(j as i32), area.x0, 1, 0, &mut coverage);
            for (p, &k) in row.iter().zip(&coverage) {
                if k <= COVERED || !p[3].is_finite() || p[3] <= 0.0 || p[..3].iter().any(|v| !v.is_finite()) {
                    continue;
                }
                if let Some(bin) = hist.get_mut(bin_of(p)) {
                    bin.count += 1;
                    for (s, v) in bin.sum.iter_mut().zip(p) {
                        *s += f64::from(*v);
                    }
                }
            }
        }
        y = y1;
    }
    Ok(hist)
}

/// The colour around the fullest bin of `hist` that `skip` lets through, with its pixel count: the
/// mean of every such bin within [`JOIN`] of the peak.
fn peak(hist: &[Bin], skip: impl Fn([f32; 3]) -> bool) -> Option<([f32; 3], u64)> {
    let top = hist.iter().filter(|b| b.count > 0 && !skip(b.mean())).max_by_key(|b| b.count)?;
    let centre = top.mean();
    let mut total = Bin::default();
    for b in hist.iter().filter(|b| b.count > 0 && !skip(b.mean()) && distance(b.mean(), centre) <= JOIN) {
        total.count += b.count;
        for (t, s) in total.sum.iter_mut().zip(b.sum) {
            *t += s;
        }
    }
    Some((total.mean(), total.count))
}

/// The dominant colour of the selected pixels and, when the area holds a clearly different one,
/// the second (`None` for an area of one colour).
fn dominant(doc: &Document) -> Result<([f32; 3], Option<[f32; 3]>)> {
    let sel = doc.selection.as_ref().ok_or_else(|| EngineError::Other("no selection".into()))?;
    let hist = histogram(doc, sel)?;
    let (first, _) = peak(&hist, |_| false).ok_or_else(|| EngineError::Other("the selected area is transparent".into()))?;
    let second = peak(&hist, |m| distance(m, first) < APART).map(|(c, _)| c);
    Ok((first, second))
}

/// The middle between two colours, or the colour itself when the area has only one.
fn shade(first: [f32; 3], second: Option<[f32; 3]>) -> [f32; 3] {
    match second {
        Some(b) => std::array::from_fn(|c| (first[c] + b[c]) / 2.0),
        None => first,
    }
}

fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}

fn dominant_color(s: &mut Session, _p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (c, _) = dominant(&d.doc)?;
    Ok(json!({"color": [c[0], c[1], c[2], 1.0]}))
}

fn fill_shade(s: &mut Session, p: &Value) -> Result<Value> {
    if p.as_object().is_some_and(|o| o.contains_key("color")) {
        return Err(bad("edit.fillShade", "takes no `color`: it fills with the mean of the area's two dominant colours"));
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (first, second) = dominant(&d.doc)?;
    let c = shade(first, second);
    // The ordinary Fill: locks, the Background, targeted channels and partly selected pixels.
    let mut params = p.clone();
    if !params.is_object() {
        params = json!({});
    }
    params["contents"] = json!("color");
    params["color"] = json!([c[0], c[1], c[2], 1.0]);
    crate::fill_cmds::fill(s, &params)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "select.dominantColor",
            label: "Dominant Color of Selection",
            menu: &[],
            shortcut: None,
            params: r##"{} → {"color":[r,g,b,a]} (the dominant colour of the visible pixels under the selection; the document is not changed)"##,
            enabled: has_selection,
            run: dominant_color,
            journal: false,
        },
        CommandSpec {
            id: "edit.fillShade",
            label: "Fill with Shade of Selection",
            menu: &[],
            shortcut: None,
            params: r##"{"mode":"normal|multiply|…"="normal","opacity":0..100=100,"preserveTransparency":bool=false,"target":"pixels"|"mask"|"quickMask"|{"channel":i}?} (fills the selection with the mean of the two dominant colours of the visible pixels under it, as edit.fill does)"##,
            enabled: has_selection,
            run: fill_shade,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20×10 document: red on the left 14 columns, blue on the right 6, fully selected.
    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10, "background": "#ff0000"})).unwrap();
        s.execute("select.rect", json!({"x": 14, "y": 0, "width": 6, "height": 10})).unwrap();
        s.execute("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0]})).unwrap();
        s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        distance(a, b) < 0.03
    }

    #[test]
    fn dominant_is_the_larger_colour_and_second_the_other() {
        let s = session();
        let (a, b) = dominant(&s.active().unwrap().doc).unwrap();
        assert!(close(a, [1.0, 0.0, 0.0]), "{a:?}");
        assert!(close(b.unwrap(), [0.0, 0.0, 1.0]));
    }

    #[test]
    fn shade_is_the_mean_of_both() {
        let mut s = session();
        s.execute("edit.fillShade", json!({})).unwrap();
        let (a, b) = dominant(&s.active().unwrap().doc).unwrap();
        assert!(close(a, [0.5, 0.0, 0.5]), "{a:?}");
        assert!(b.is_none());
    }

    #[test]
    fn one_colour_keeps_its_colour() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "#00ff00"})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.fillShade", json!({})).unwrap();
        let (a, b) = dominant(&s.active().unwrap().doc).unwrap();
        assert!(close(a, [0.0, 1.0, 0.0]));
        assert!(b.is_none());
    }

    #[test]
    fn dominant_color_reports_it_and_changes_nothing() {
        let mut s = session();
        let v = s.execute("select.dominantColor", json!({})).unwrap();
        let c = &v["color"];
        assert!(c[0].as_f64().unwrap() > 0.95 && c[2].as_f64().unwrap() < 0.05);
    }

    #[test]
    fn fails_without_a_selection_or_with_a_colour_param() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        assert!(s.execute("select.dominantColor", json!({})).is_err());
        assert!(s.execute("edit.fillShade", json!({})).is_err());
        s.execute("select.all", json!({})).unwrap();
        assert!(s.execute("edit.fillShade", json!({"color": "#ff0000"})).is_err());
        assert!(s.execute("edit.fillShade", json!(null)).is_ok());
    }

    #[test]
    fn works_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 6, "height": 6, "depth": depth})).unwrap();
            s.execute("select.all", json!({})).unwrap();
            assert!(s.execute("edit.fillShade", json!({})).is_ok(), "depth {depth}");
        }
    }
}
