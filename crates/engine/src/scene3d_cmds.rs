//! 3D pose editor commands: render a scene into a pixel layer, and save/open `.pcscene` files.
//!
//! The scene itself is not held by the session: the editor (or an agent) passes it as the `scene`
//! parameter (or a `path` to a `.pcscene` file), so these commands stay stateless and scriptable.
//! Rendering uses the software renderer (`photocraft-render3d`) and lands as one undo step.

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_color::PixelFormat;
use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use photocraft_render3d::{MAX_EDGE, RenderOptions};
use photocraft_scene3d::Scene;
use photocraft_scene3d::model3d::ModelData;
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

// A scene the editor handed over in memory, with its models as they are now (morphed, with the
// skin pictures of a look): a scene sent as JSON carries only the morph values, and the models
// would be loaded again from their files without them. Taken by the next `renderToLayer` that
// asks for it with `"prepared": true`, on the same thread (commands run where the editor calls them).
thread_local! {
    static PREPARED: std::cell::RefCell<Option<Scene>> = const { std::cell::RefCell::new(None) };
}

/// Leaves a scene for the next `scene3d.renderToLayer` call with `"prepared": true`.
pub fn prepare_scene(scene: Scene) {
    PREPARED.with(|p| *p.borrow_mut() = Some(scene));
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

/// The last few models loaded, keyed by a hash of the file's bytes, so rendering a scene again
/// (or several frames of one) does not decode the same textures every time.
static MODELS: Mutex<Vec<(blake3::Hash, Arc<ModelData>)>> = Mutex::new(Vec::new());
const MODEL_CACHE: usize = 2;

fn load_model(cmd: &str, path: &str) -> Result<Arc<ModelData>> {
    let bytes = read_file(path)?;
    let key = blake3::hash(&bytes);
    if let Some(hit) = MODELS.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|(k, _)| *k == key) {
        return Ok(hit.1.clone());
    }
    let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
    let model = Arc::new(photocraft_scene3d::model3d::load(name, &bytes).map_err(|e| bad(cmd, format!("{path}: {e}")))?);
    let mut cache = MODELS.lock().unwrap_or_else(PoisonError::into_inner);
    cache.insert(0, (key, model.clone()));
    cache.truncate(MODEL_CACHE);
    Ok(model)
}

/// Loads every model the scene names that is not in memory (a scene read from JSON or a file).
fn resolve_assets(cmd: &str, scene: &mut Scene) -> Result<()> {
    let missing: Vec<(u32, String, Option<String>)> = scene.missing_assets().iter().map(|a| (a.id, a.name.clone(), a.path.clone())).collect();
    for (id, name, path) in missing {
        let path = path.ok_or_else(|| bad(cmd, format!("model `{name}` has no file path; import it from a file so the scene can find it again")))?;
        let model = load_model(cmd, &path)?;
        scene.attach_model(id, model).map_err(|e| bad(cmd, e.to_string()))?;
    }
    Ok(())
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
    let prepared = if p.get("prepared").and_then(Value::as_bool) == Some(true) { PREPARED.with(|p| p.borrow_mut().take()) } else { None };
    let mut scene = match prepared {
        Some(scene) => scene,
        None => scene_param(CMD, p)?,
    };
    // Models the editor did not hold in memory still come from their files.
    resolve_assets(CMD, &mut scene)?;
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

fn run_add_model(_: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.addModel";
    let model_path = p.get("model").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| bad(CMD, "`model` (a .glb, .gltf or .vrm file) is required"))?;
    let mut scene = if p.get("scene").is_some() || p.get("path").is_some() { scene_param(CMD, p)? } else { Scene::new() };
    resolve_assets(CMD, &mut scene)?;
    let model = load_model(CMD, model_path)?;
    let (bones, meshes) = (model.nodes.iter().enumerate().filter(|(i, _)| model.is_joint(*i)).count(), model.meshes.len());
    let root = scene.add_model(model, Some(model_path.to_string())).map_err(|e| bad(CMD, e.to_string()))?;
    let value = serde_json::to_value(&scene).map_err(|e| bad(CMD, e.to_string()))?;
    Ok(json!({"scene": value, "root": root, "bones": bones, "meshes": meshes, "nodes": scene.nodes.len()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "scene3d.addModel",
            label: "Add 3D Model to Scene…",
            menu: &[],
            shortcut: None,
            params: r##"{"model":text (.glb|.gltf|.vrm),"scene":object|"path":text (.pcscene, optional: a new scene when omitted)} → {scene,root,bones,meshes,nodes}"##,
            enabled: always,
            run: run_add_model,
            journal: false,
        },
        CommandSpec {
            id: "scene3d.renderToLayer",
            label: "Render 3D Scene to Layer",
            menu: &[],
            shortcut: None,
            params: r##"{"scene":object|"path":text (.pcscene),"width":1..16384=document width,"height":1..16384=document height,"ssaa":1..4=2,"transparent":bool=true,"name":text="3D Render","prepared":bool=false (use the scene left by the editor)} → {layer,width,height,nodes}"##,
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

#[cfg(test)]
mod tests_models;
