//! 3D pose editor commands: render a scene into a pixel layer, and save/open `.pcscene` files.
//!
//! The scene itself is not held by the session: the editor (or an agent) passes it as the `scene`
//! parameter (or a `path` to a `.pcscene` file), so these commands stay stateless and scriptable.
//! Rendering uses the software renderer (`photocraft-render3d`) and lands as one undo step.

use photocraft_color::PixelFormat;
use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use photocraft_render3d::{MAX_EDGE, RenderOptions};
use photocraft_scene3d::Scene;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, int};
use crate::file_cmds::{read_file, write_file};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// The scene from `scene` (inline JSON) or `path` (a `.pcscene` file).
fn scene_param(cmd: &str, p: &Value) -> Result<Scene> {
    let text = match (p.get("scene"), p.get("path").and_then(Value::as_str)) {
        (Some(v), _) if v.is_object() => serde_json::to_string(v).map_err(|e| bad(cmd, e.to_string()))?,
        (_, Some(path)) => {
            let bytes = read_file(path)?;
            String::from_utf8(bytes).map_err(|_| bad(cmd, format!("{path} is not a text scene file")))?
        }
        _ => return Err(bad(cmd, "give `scene` (a scene object) or `path` (a .pcscene file)")),
    };
    Scene::from_json(&text).map_err(|e| bad(cmd, e.to_string()))
}

fn dimension(cmd: &str, p: &Value, key: &str, default: i64) -> Result<u32> {
    let v = int(p, key).unwrap_or(default);
    u32::try_from(v).ok().filter(|v| (1..=MAX_EDGE).contains(v)).ok_or_else(|| bad(cmd, format!("`{key}` = {v} must be between 1 and {MAX_EDGE}")))
}

/// RGBA8 sRGB pixels as a surface, written in bands so the float staging buffer stays small.
fn surface_from_rgba(img: &photocraft_render3d::Image) -> Result<Surface> {
    const BAND: usize = 128;
    let mut surf = Surface::new(PixelFormat::RGBA8);
    let (w, h) = (img.width as usize, img.height as usize);
    let alloc = |e: photocraft_raster::AllocationError| EngineError::Other(format!("not enough memory for the render: {e:?}"));
    let mut y = 0;
    while y < h {
        let rows = BAND.min(h - y);
        let mut data = Vec::with_capacity(w * rows * 4);
        for px in img.rgba.as_chunks::<4>().0.iter().skip(y * w).take(w * rows) {
            data.extend(px.iter().map(|&c| f32::from(c) / 255.0));
        }
        let r = Rect::new(0, y as i32, w as i32, (y + rows) as i32);
        surf.try_write_region(r, &data).map_err(alloc)?;
        y += rows;
    }
    Ok(surf)
}

fn run_render(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.renderToLayer";
    let scene = scene_param(CMD, p)?;
    let doc = s.active().ok_or(EngineError::NoDocument)?;
    let fmt = doc.doc.pixel_format();
    let (dw, dh) = (i64::from(doc.doc.size.width), i64::from(doc.doc.size.height));
    let width = dimension(CMD, p, "width", dw)?;
    let height = dimension(CMD, p, "height", dh)?;
    let ssaa = int(p, "ssaa").unwrap_or(2).clamp(1, 4) as u32;
    let transparent = p.get("transparent").and_then(Value::as_bool).unwrap_or(true);
    let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or("3D Render").to_string();

    let img = photocraft_render3d::render(&scene, &RenderOptions { width, height, ssaa, transparent }).map_err(|e| bad(CMD, e.to_string()))?;
    let alloc = |e: photocraft_raster::AllocationError| EngineError::Other(format!("not enough memory for the render: {e:?}"));
    let surface = surface_from_rgba(&img)?.try_convert(fmt).map_err(alloc)?;
    let id = s.edit("3D Render", |doc, active| {
        let id = doc.insert_above(*active, Layer::new(name, LayerContent::Raster(surface)));
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "width": width, "height": height, "nodes": scene.nodes.len()}))
}

fn run_save(_: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.save";
    let path = p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| bad(CMD, "`path` is required"))?;
    let scene = scene_param(CMD, &json!({"scene": p.get("scene")}))?;
    let text = scene.to_json().map_err(|e| bad(CMD, e.to_string()))?;
    write_file(path, text.as_bytes())?;
    Ok(json!({"path": path, "nodes": scene.nodes.len(), "bytes": text.len()}))
}

fn run_open(_: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.open";
    p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| bad(CMD, "`path` is required"))?;
    let scene = scene_param(CMD, p)?;
    let value = serde_json::to_value(&scene).map_err(|e| bad(CMD, e.to_string()))?;
    Ok(json!({"scene": value, "nodes": scene.nodes.len()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "scene3d.renderToLayer",
            label: "Render 3D Scene to Layer",
            menu: &[],
            shortcut: None,
            params: r##"{"scene":object|"path":text (.pcscene),"width":1..16384=document width,"height":1..16384=document height,"ssaa":1..4=2,"transparent":bool=true,"name":text="3D Render"} → {layer,width,height,nodes}"##,
            enabled: has_doc,
            run: run_render,
            journal: true,
        },
        CommandSpec {
            id: "scene3d.save",
            label: "Save 3D Scene…",
            menu: &[],
            shortcut: None,
            params: r##"{"scene":object,"path":text} → {path,nodes,bytes}"##,
            enabled: always,
            run: run_save,
            journal: false,
        },
        CommandSpec {
            id: "scene3d.open",
            label: "Open 3D Scene…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":text} → {scene,nodes}"##,
            enabled: always,
            run: run_open,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
