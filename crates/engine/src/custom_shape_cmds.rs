//! Custom Shape additions kept out of the shared preset files (fork-local, see MY_CHANGES.md):
//! `shape.presets.fillPixels` paints a preset shape straight onto a pixel layer.

use photocraft_algo::selection as sel;
use photocraft_color::BlendMode;
use photocraft_doc::vector::Path;
use photocraft_doc::{Layer, LayerId};
use photocraft_geom::{Point, Rect};
use photocraft_vector as vector;
use photocraft_vector::warp::{Leaf, clip_to_leaf, warp_path};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::presets::{bad, has_doc, has_layer, shapes};
use crate::{EngineError, Result, Session};

const FILL_PIXELS: &str = "shape.presets.fillPixels";
const PLACE_LEAF: &str = "shape.presets.placeLeaf";
const FILL_GRID: &str = "shape.presets.fillGrid";
/// Most cells a grid fill lays out and most names a `shapes` list may hold.
const MAX_CELLS: usize = 50_000;
const MAX_NAMES: usize = MAX_CELLS;
/// The most points a fill is drawn from: past it nearby points are merged, which the eye cannot see.
const MAX_FILL_POINTS: usize = 120_000;

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
    let path = photocraft_vector::thin::simplify_to(&target_path(p, &shape, (w, h), FILL_PIXELS)?, 0.1, 0.25, MAX_FILL_POINTS);

    let fg = s.tools.foreground;
    let c = p.get("color").and_then(crate::vector_cmds::color).unwrap_or(photocraft_color::Color::rgba(fg[0], fg[1], fg[2], fg[3]));
    let rgb = c.to_rgb();
    let src = [rgb[0], rgb[1], rgb[2], c.alpha];
    let opacity = (num(p, "opacity").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mode = p.get("mode").and_then(Value::as_str).and_then(blend_from_str).unwrap_or(BlendMode::Normal);
    let feather = num(p, "feather").unwrap_or(0.0).max(0.0) as f32;
    let aa = p.get("antiAlias").and_then(Value::as_bool).unwrap_or(true);
    let target = crate::vector_cmds::layer_id(s, p)?;
    let erase = p.get("erase").and_then(Value::as_bool).unwrap_or(false);

    let r = vector::fill_rasterizer(&path, vector::DEFAULT_TOLERANCE);
    let feather_pad = (feather.ceil() as i32).saturating_mul(3).saturating_add(1);
    let area = r.pixel_bounds().map_or(bounds, |b| b.inflate(feather_pad).intersect(&bounds));
    if area.is_empty() {
        // Nothing lands on the canvas: no history step, no new layer.
        return Ok(json!({"layer": target.0, "empty": true}));
    }

    let mut painted = target;
    let background = s.tools.background;
    s.edit(if erase { "Erase Shape" } else { "Fill Shape" }, |doc, active| {
        let is_pixel = doc.layer_mut(target).is_some_and(|l| l.surface_mut().is_some());
        if !is_pixel && erase {
            return Err(EngineError::Other("Erase Shape needs a pixel layer".into()));
        }
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
        let is_background = doc.is_background(painted);
        let l = doc.layer_mut(painted).ok_or(EngineError::NoLayer(painted))?;
        if locks.pixels || locks.all {
            return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
        }
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("Fill Shape needs a pixel layer".into()))?;
        if !erase {
            crate::vector_cmds::paint_coverage(surf, area, &cov, src, opacity, mode, locks.transparency);
        } else if is_background {
            // Nothing is transparent on the Background: erasing shows the background colour.
            let bg = [background[0], background[1], background[2], 1.0];
            crate::vector_cmds::paint_coverage(surf, area, &cov, bg, opacity, BlendMode::Normal, false);
        } else if !locks.transparency {
            erase_coverage(surf, area, &cov, opacity);
        }
        Ok(())
    })?;
    let LayerId(id) = painted;
    Ok(json!({"layer": id, "empty": false, "preset": name}))
}

/// Lowers the alpha of every pixel under `cov` (0..1 per pixel of `area`) by `cov * opacity`; a
/// pixel left with no alpha is cleared.
fn erase_coverage(surf: &mut photocraft_raster::Surface, area: photocraft_geom::Rect, cov: &[f32], opacity: f32) {
    let fmt = surf.format();
    let ch = fmt.channels();
    let mut vals = surf.read_region(area);
    for (px, c) in vals.chunks_exact_mut(ch).zip(cov) {
        let k = (c * opacity).clamp(0.0, 1.0);
        if k <= 0.0 {
            continue;
        }
        let mut rgba = photocraft_raster::to_rgba(&fmt, px);
        rgba[3] *= 1.0 - k;
        if rgba[3] <= 1e-6 {
            rgba = [0.0; 4];
        }
        photocraft_raster::from_rgba_into(&fmt, rgba, px);
    }
    surf.write_region(area, &vals);
    surf.prune();
}

/// A colour param (`"#rrggbb"` or `[r,g,b,a]`) as straight RGBA, `default` when absent or invalid.
fn rgba_param(p: &Value, key: &str, default: [f32; 4]) -> [f32; 4] {
    p.get(key).and_then(crate::vector_cmds::color).map_or(default, |c| {
        let rgb = c.to_rgb();
        [rgb[0], rgb[1], rgb[2], c.alpha]
    })
}

/// `shape.presets.fillGrid`: covers the selection with shapes laid in rows, left to right and top
/// to bottom, each fitted into a square cell of `cell` px (names taken in turn from `shapes`).
/// The shapes are painted with the foreground colour and the rest of the selection with the
/// background colour, as pixels, in one history step.
fn fill_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let names: Vec<&str> = p.get("shapes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if names.is_empty() || names.len() > MAX_NAMES {
        return Err(bad(FILL_GRID, format!("`shapes` must list 1 to {MAX_NAMES} shape names")));
    }
    let cell = num(p, "cell").filter(|c| c.is_finite() && (2.0..=10_000.0).contains(c)).ok_or_else(|| bad(FILL_GRID, "`cell` must be 2 to 10000 px"))?;
    let group = p.get("group").and_then(Value::as_str);
    let mut shapes_by_name: std::collections::HashMap<&str, std::sync::Arc<Path>> = std::collections::HashMap::new();
    for n in &names {
        if !shapes_by_name.contains_key(n) {
            let shape = crate::shape_index::shape_path(s, n, group).ok_or_else(|| bad(FILL_GRID, format!("no shape \"{n}\" (see shape.presets.list)")))?;
            shapes_by_name.insert(n, shape);
        }
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let bounds = d.doc.bounds();
    let Some(selection) = d.doc.selection.as_ref() else { return Err(bad(FILL_GRID, "there is no selection to fill")) };
    let area = selection.content_bounds().intersect(&bounds);
    if area.is_empty() {
        return Ok(json!({"layer": null, "empty": true, "cells": 0}));
    }
    let mask = sel::mask_from_surface(Some(selection), area);
    let (aw, ah) = (area.width() as usize, area.height() as usize);
    let (cols, rows) = ((aw as f64 / cell).ceil() as usize, (ah as f64 / cell).ceil() as usize);
    if cols.saturating_mul(rows) > MAX_CELLS {
        return Err(bad(FILL_GRID, format!("{cols} x {rows} cells is too many (at most {MAX_CELLS}): use a bigger `cell`")));
    }
    let aa = p.get("antiAlias").and_then(Value::as_bool).unwrap_or(true);
    let mut cov = vec![0.0f32; mask.len()];
    let mut placed = 0usize;
    for row in 0..rows {
        for col in 0..cols {
            let (x, y) = (f64::from(area.x0) + col as f64 * cell, f64::from(area.y0) + row as f64 * cell);
            let region = Rect::new(x.floor() as i32, y.floor() as i32, (x + cell).ceil() as i32, (y + cell).ceil() as i32).intersect(&area);
            if region.is_empty() {
                continue;
            }
            let rw = region.width() as usize;
            let touches = (region.y0..region.y1).any(|py| {
                let start = (py - area.y0) as usize * aw + (region.x0 - area.x0) as usize;
                mask.get(start..start + rw).is_some_and(|m| m.iter().any(|v| *v > 0.0))
            });
            if !touches {
                continue;
            }
            let name = names.get((row * cols + col) % names.len()).copied().unwrap_or_default();
            let Some(shape) = shapes_by_name.get(name) else { continue };
            let fitted = shapes::fit(shape, [x, y, cell, cell], true);
            let path = vector::thin::simplify_to(&fitted, 0.1, 0.25, MAX_FILL_POINTS);
            let r = vector::fill_rasterizer(&path, vector::DEFAULT_TOLERANCE);
            let Some(sub) = r.pixel_bounds().map(|b| b.intersect(&region)).filter(|b| !b.is_empty()) else { continue };
            let part = r.render(sub);
            let sw = sub.width() as usize;
            for (i, v) in part.iter().enumerate() {
                let idx = (sub.y0 - area.y0) as usize * aw + (sub.x0 - area.x0) as usize + (i / sw) * aw + i % sw;
                if let Some(c) = cov.get_mut(idx) {
                    *c = c.max(*v);
                }
            }
            placed += 1;
        }
    }
    if !aa {
        cov.iter_mut().for_each(|v| *v = if *v >= 0.5 { 1.0 } else { 0.0 });
    }
    let shape_cov: Vec<f32> = cov.iter().zip(&mask).map(|(c, m)| c * m).collect();
    let (fg, bg) = (rgba_param(p, "color", s.tools.foreground), rgba_param(p, "background", s.tools.background));
    let fill_background = p.get("fillBackground").and_then(Value::as_bool).unwrap_or(true);
    let opacity = (num(p, "opacity").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mode = p.get("mode").and_then(Value::as_str).and_then(blend_from_str).unwrap_or(BlendMode::Normal);
    let target = crate::vector_cmds::layer_id(s, p)?;

    let mut painted = target;
    s.edit("Fill Shapes", |doc, active| {
        let is_pixel = doc.layer_mut(target).is_some_and(|l| l.surface_mut().is_some());
        if !is_pixel {
            let name = doc.next_layer_name("Layer");
            let id = doc.insert_above(Some(target).filter(|t| doc.layer(*t).is_some()).or(*active), Layer::raster(name, doc.pixel_format()));
            *active = Some(id);
            painted = id;
        }
        let locks = doc.effective_locks(painted);
        let l = doc.layer_mut(painted).ok_or(EngineError::NoLayer(painted))?;
        if locks.pixels || locks.all {
            return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
        }
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("Fill Shapes needs a pixel layer".into()))?;
        if fill_background {
            crate::vector_cmds::paint_coverage(surf, area, &mask, bg, opacity, mode, locks.transparency);
        }
        crate::vector_cmds::paint_coverage(surf, area, &shape_cov, fg, opacity, mode, locks.transparency);
        Ok(())
    })?;
    let LayerId(id) = painted;
    Ok(json!({"layer": id, "empty": false, "cells": placed}))
}

/// A leaf from a `leaf` param: `{"a":[x,y],"b":[x,y],"width":0..1000=0.5,"fullness":0.1..3=1,"clip":bool=false}`,
/// and whether it is a zipper (`clip`): the shape cut off by the leaf instead of bent onto it.
fn leaf_param(p: &Value, cmd: &str) -> Result<Option<(Leaf, bool)>> {
    let Some(l) = p.get("leaf").filter(|l| !l.is_null()) else { return Ok(None) };
    let pt = |k: &str| -> Result<Point> {
        let a = l.get(k).and_then(Value::as_array).filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, format!("`leaf.{k}` must be [x, y]")))?;
        match (a[0].as_f64(), a[1].as_f64()) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(Point::new(x, y)),
            _ => Err(bad(cmd, format!("`leaf.{k}` must be two finite numbers"))),
        }
    };
    let leaf = Leaf { a: pt("a")?, b: pt("b")?, width: num(l, "width").unwrap_or(0.5), fullness: num(l, "fullness").unwrap_or(1.0) };
    Ok(Some((leaf, l.get("clip").and_then(Value::as_bool).unwrap_or(false))))
}

/// The shape's path where it goes: bent onto the `leaf` param, else fitted into `rect`
/// (`keepAspect`; by default the middle of the canvas).
fn target_path(p: &Value, shape: &Path, canvas: (f64, f64), cmd: &str) -> Result<Path> {
    if let Some((leaf, clip)) = leaf_param(p, cmd)? {
        let done = if clip { clip_to_leaf(shape, &leaf) } else { warp_path(shape, &leaf) };
        return done.ok_or_else(|| bad(cmd, "the leaf has no length (A and B coincide) or the shape is empty"));
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
        params: r##"{"preset":name,"group":name?,"rect":[x,y,w,h]?,"keepAspect":bool=true,"color":"#rrggbb"|[r,g,b,a]?=foreground,"opacity":0..100=100,"mode":blend="Normal","feather":px=0,"antiAlias":bool=true,"leaf":{"a":[x,y],"b":[x,y],"width":0..1000=0.5,"fullness":0.1..3=1,"clip":bool=false}? (bends the shape onto a leaf instead of fitting `rect`; with `clip` the shape is laid undeformed in the leaf's box and cut off by the leaf),"erase":bool=false (lowers the layer's alpha by the shape instead of painting; needs a pixel layer),"layer":id?} → {layer,empty,preset}. Paints on the pixel layer (a new one above a non-pixel layer); empty:true when nothing lands on the canvas (no history step)."##,
        enabled: has_layer,
        run: fill_pixels,
        journal: true,
    }];
    v.push(CommandSpec {
        id: PLACE_LEAF,
        label: "Place Shape on Leaf",
        menu: &[],
        shortcut: None,
        params: r##"{"preset":name,"group":name?,"leaf":{"a":[x,y],"b":[x,y],"width":0..1000=0.5,"fullness":0.1..3=1},"fill":..,"stroke":..,"name":str?} → the new shape layer (as shape.create). The shape's box is bent onto the leaf between A and B."##,
        enabled: has_doc,
        run: place_leaf,
        journal: true,
    });
    v.push(CommandSpec {
        id: FILL_GRID,
        label: "Fill Selection with Shapes",
        menu: &[],
        shortcut: None,
        params: r##"{"shapes":[name,..] (taken in turn, cell by cell),"cell":2..10000 px (square cells in rows, left to right, top to bottom, from the selection's top left),"group":name?,"color":"#rrggbb"|[r,g,b,a]?=foreground,"background":"#rrggbb"|[r,g,b,a]?=background colour,"fillBackground":bool=true,"opacity":0..100=100,"mode":blend="Normal","antiAlias":bool=true,"layer":id?} → {layer,empty,cells}. Needs a selection. Each shape is fitted whole into its cell and painted with the foreground colour; the rest of the selection gets the background colour. One history step; a new pixel layer above a non-pixel layer."##,
        enabled: has_layer,
        run: fill_grid,
        journal: true,
    });
    v.extend(crate::shape_csh::specs());
    v
}

#[cfg(test)]
mod tests;
