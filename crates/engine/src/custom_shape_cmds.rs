//! Custom Shape additions kept out of the shared preset files (fork-local, see MY_CHANGES.md):
//! `shape.presets.fillPixels` paints a preset shape straight onto a pixel layer.

use photocraft_algo::selection as sel;
use photocraft_color::BlendMode;
use photocraft_doc::{Layer, LayerId};
use photocraft_vector as vector;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::presets::{bad, has_layer, shapes};
use crate::{EngineError, Result, Session};

const FILL_PIXELS: &str = "shape.presets.fillPixels";

fn num(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64)
}

/// `shape.presets.fillPixels`: fits the preset into `rect` and paints it with the foreground
/// colour like Fill Path (selection, locks, anti-aliasing, feather, blend mode, opacity). The
/// active layer must be a pixel layer, otherwise a new one is made above it in the same history step.
fn fill_pixels(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("preset").and_then(Value::as_str).ok_or_else(|| bad(FILL_PIXELS, "`preset` is required"))?;
    let group = p.get("group").and_then(Value::as_str);
    let shape = crate::shape_index::find_shape(s, name, group).ok_or_else(|| bad(FILL_PIXELS, format!("no shape \"{name}\" (see shape.presets.list)")))?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let bounds = d.doc.bounds();
    let (w, h) = (f64::from(d.doc.size.width), f64::from(d.doc.size.height));
    let keep = p.get("keepAspect").and_then(Value::as_bool).unwrap_or(true);
    let rect = match p.get("rect").and_then(Value::as_array) {
        Some(a) if a.len() == 4 => {
            let v: Vec<f64> = a.iter().map(|x| x.as_f64().filter(|f| f.is_finite()).unwrap_or(0.0)).collect();
            let (x, wd) = if v[2] < 0.0 { (v[0] + v[2], -v[2]) } else { (v[0], v[2]) };
            let (y, hd) = if v[3] < 0.0 { (v[1] + v[3], -v[3]) } else { (v[1], v[3]) };
            if wd < 1.0 || hd < 1.0 {
                return Err(bad(FILL_PIXELS, "the shape rect is empty"));
            }
            [x, y, wd, hd]
        }
        _ => {
            let side = w.min(h) * 0.5;
            [(w - side) / 2.0, (h - side) / 2.0, side, side]
        }
    };
    let path = shapes::fit(&shape.path, rect, keep);

    let fg = s.tools.foreground;
    let c = p.get("color").and_then(crate::vector_cmds::color).unwrap_or(photocraft_color::Color::rgba(fg[0], fg[1], fg[2], fg[3]));
    let rgb = c.to_rgb();
    let src = [rgb[0], rgb[1], rgb[2], c.alpha];
    let opacity = (num(p, "opacity").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mode = p.get("mode").and_then(Value::as_str).and_then(blend_from_str).unwrap_or(BlendMode::Normal);
    let feather = num(p, "feather").unwrap_or(0.0).max(0.0) as f32;
    let aa = p.get("antiAlias").and_then(Value::as_bool).unwrap_or(true);
    let target = crate::vector_cmds::layer_id(s, p)?;

    let r = vector::fill_rasterizer(&path, vector::DEFAULT_TOLERANCE);
    let feather_pad = (feather.ceil() as i32).saturating_mul(3).saturating_add(1);
    let area = r.pixel_bounds().map_or(bounds, |b| b.inflate(feather_pad).intersect(&bounds));
    if area.is_empty() {
        // Nothing lands on the canvas: no history step, no new layer.
        return Ok(json!({"layer": target.0, "empty": true}));
    }

    let mut painted = target;
    s.edit("Fill Shape", |doc, active| {
        let is_pixel = doc.layer_mut(target).is_some_and(|l| l.surface_mut().is_some());
        if !is_pixel {
            let name = doc.next_layer_name("Layer");
            let id = doc.insert_above(Some(target).filter(|t| doc.layer(*t).is_some()).or(*active), Layer::raster(name, doc.pixel_format()));
            *active = Some(id);
            painted = id;
        }
        let mut cov = r.render(area);
        if !aa {
            cov.iter_mut().for_each(|v| *v = if *v >= 0.5 { 1.0 } else { 0.0 });
        }
        if feather > 0.0 {
            cov = sel::feather(&cov, area.width() as usize, area.height() as usize, feather);
        }
        if let Some(selection) = doc.selection.as_ref() {
            let mask = sel::mask_from_surface(Some(selection), area);
            cov.iter_mut().zip(&mask).for_each(|(c, m)| *c *= m);
        }
        let locks = doc.effective_locks(painted);
        let l = doc.layer_mut(painted).ok_or(EngineError::NoLayer(painted))?;
        if locks.pixels || locks.all {
            return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
        }
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("Fill Shape needs a pixel layer".into()))?;
        crate::vector_cmds::paint_coverage(surf, area, &cov, src, opacity, mode, locks.transparency);
        Ok(())
    })?;
    let LayerId(id) = painted;
    Ok(json!({"layer": id, "empty": false, "preset": shape.name}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: FILL_PIXELS,
        label: "Fill Shape (Pixels)",
        menu: &[],
        shortcut: None,
        params: r##"{"preset":name,"group":name?,"rect":[x,y,w,h]?,"keepAspect":bool=true,"color":"#rrggbb"|[r,g,b,a]?=foreground,"opacity":0..100=100,"mode":blend="Normal","feather":px=0,"antiAlias":bool=true,"layer":id?} → {layer,empty,preset}. Paints on the pixel layer (a new one above a non-pixel layer); empty:true when nothing lands on the canvas (no history step)."##,
        enabled: has_layer,
        run: fill_pixels,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
