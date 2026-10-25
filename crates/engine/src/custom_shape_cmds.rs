//! Custom Shape additions kept out of the shared preset files (fork-local, see MY_CHANGES.md):
//! `shape.presets.fillPixels` paints a preset shape straight onto a pixel layer.

use photocraft_algo::selection as sel;
use photocraft_color::BlendMode;
use photocraft_doc::vector::Path;
use photocraft_doc::{Layer, LayerId};
use photocraft_geom::Point;
use photocraft_vector as vector;
use photocraft_vector::warp::{Leaf, warp_path};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::presets::{bad, has_doc, has_layer, shapes};
use crate::{EngineError, Result, Session};

const FILL_PIXELS: &str = "shape.presets.fillPixels";
const PLACE_LEAF: &str = "shape.presets.placeLeaf";

fn num(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64)
}

/// `shape.presets.fillPixels`: fits the preset into `rect` and paints it with the foreground
/// colour like Fill Path (selection, locks, anti-aliasing, feather, blend mode, opacity). The
/// active layer must be a pixel layer, otherwise a new one is made above it in the same history step.
fn fill_pixels(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("preset").and_then(Value::as_str).ok_or_else(|| bad(FILL_PIXELS, "`preset` is required"))?;
    let group = p.get("group").and_then(Value::as_str);
    let shape = crate::shape_index::shape_path(s, name, group).ok_or_else(|| bad(FILL_PIXELS, format!("no shape \"{name}\" (see shape.presets.list)")))?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let bounds = d.doc.bounds();
    let (w, h) = (f64::from(d.doc.size.width), f64::from(d.doc.size.height));
    // A traced drawing has far more curves than pixels: draw it from points a quarter of a pixel apart.
    let path = photocraft_vector::thin::simplify(&target_path(p, &shape, (w, h), FILL_PIXELS)?, 0.1, 0.25);

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
    Ok(json!({"layer": id, "empty": false, "preset": name}))
}

/// A leaf from a `leaf` param: `{"a":[x,y],"b":[x,y],"width":0..4=0.5,"fullness":0.1..3=1}`.
fn leaf_param(p: &Value, cmd: &str) -> Result<Option<Leaf>> {
    let Some(l) = p.get("leaf").filter(|l| !l.is_null()) else { return Ok(None) };
    let pt = |k: &str| -> Result<Point> {
        let a = l.get(k).and_then(Value::as_array).filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, format!("`leaf.{k}` must be [x, y]")))?;
        match (a[0].as_f64(), a[1].as_f64()) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(Point::new(x, y)),
            _ => Err(bad(cmd, format!("`leaf.{k}` must be two finite numbers"))),
        }
    };
    Ok(Some(Leaf { a: pt("a")?, b: pt("b")?, width: num(l, "width").unwrap_or(0.5), fullness: num(l, "fullness").unwrap_or(1.0) }))
}

/// The shape's path where it goes: bent onto the `leaf` param, else fitted into `rect`
/// (`keepAspect`; by default the middle of the canvas).
fn target_path(p: &Value, shape: &Path, canvas: (f64, f64), cmd: &str) -> Result<Path> {
    if let Some(leaf) = leaf_param(p, cmd)? {
        return warp_path(shape, &leaf).ok_or_else(|| bad(cmd, "the leaf has no length (A and B coincide) or the shape is empty"));
    }
    let keep = p.get("keepAspect").and_then(Value::as_bool).unwrap_or(true);
    let rect = match p.get("rect").and_then(Value::as_array) {
        Some(a) if a.len() == 4 => {
            let v: Vec<f64> = a.iter().map(|x| x.as_f64().filter(|f| f.is_finite()).unwrap_or(0.0)).collect();
            let (x, wd) = if v[2] < 0.0 { (v[0] + v[2], -v[2]) } else { (v[0], v[2]) };
            let (y, hd) = if v[3] < 0.0 { (v[1] + v[3], -v[3]) } else { (v[1], v[3]) };
            if wd < 1.0 || hd < 1.0 {
                return Err(bad(cmd, "the shape rect is empty"));
            }
            [x, y, wd, hd]
        }
        _ => {
            let side = canvas.0.min(canvas.1) * 0.5;
            [(canvas.0 - side) / 2.0, (canvas.1 - side) / 2.0, side, side]
        }
    };
    Ok(shapes::fit(shape, rect, keep))
}

/// `shape.presets.placeLeaf`: a vector shape layer of the preset bent onto a leaf.
fn place_leaf(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("preset").and_then(Value::as_str).ok_or_else(|| bad(PLACE_LEAF, "`preset` is required"))?;
    let shape = crate::shape_index::find_shape(s, name, p.get("group").and_then(Value::as_str))
        .ok_or_else(|| bad(PLACE_LEAF, format!("no shape \"{name}\" (see shape.presets.list)")))?;
    if p.get("leaf").is_none_or(Value::is_null) {
        return Err(bad(PLACE_LEAF, "`leaf` is required"));
    }
    let path = target_path(p, &shape.path, (0.0, 0.0), PLACE_LEAF)?;
    let mut q = json!({"kind": "path", "path": crate::vector_cmds::path_json(&path), "name": p.get("name").and_then(Value::as_str).unwrap_or(&shape.name)});
    for k in ["fill", "stroke", "addTo", "op"] {
        if let Some(v) = p.get(k) {
            q[k] = v.clone();
        }
    }
    let mut r = crate::presets::call(s, "shape.create", q)?;
    r["preset"] = json!(shape.name);
    Ok(r)
}

pub fn specs() -> Vec<CommandSpec> {
    let mut v = vec![CommandSpec {
        id: FILL_PIXELS,
        label: "Fill Shape (Pixels)",
        menu: &[],
        shortcut: None,
        params: r##"{"preset":name,"group":name?,"rect":[x,y,w,h]?,"keepAspect":bool=true,"color":"#rrggbb"|[r,g,b,a]?=foreground,"opacity":0..100=100,"mode":blend="Normal","feather":px=0,"antiAlias":bool=true,"leaf":{"a":[x,y],"b":[x,y],"width":0..4=0.5,"fullness":0.1..3=1}? (bends the shape onto a leaf instead of fitting `rect`),"layer":id?} → {layer,empty,preset}. Paints on the pixel layer (a new one above a non-pixel layer); empty:true when nothing lands on the canvas (no history step)."##,
        enabled: has_layer,
        run: fill_pixels,
        journal: true,
    }];
    v.push(CommandSpec {
        id: PLACE_LEAF,
        label: "Place Shape on Leaf",
        menu: &[],
        shortcut: None,
        params: r##"{"preset":name,"group":name?,"leaf":{"a":[x,y],"b":[x,y],"width":0..4=0.5,"fullness":0.1..3=1},"fill":..,"stroke":..,"name":str?} → the new shape layer (as shape.create). The shape's box is bent onto the leaf between A and B."##,
        enabled: has_doc,
        run: place_leaf,
        journal: true,
    });
    v.extend(crate::shape_csh::specs());
    v
}

#[cfg(test)]
mod tests;
