//! Custom shape files (`.csh`) for the Shapes panel (fork-local, see MY_CHANGES.md): Load /
//! Replace / Save Shapes. A loaded file stays a lazy library (`shape_index`): only its names are
//! read up front, and a shape's path is decoded when something asks for it. The raw file bytes are
//! kept so the library survives a restart without being decoded either.

use std::collections::HashMap;
use std::sync::Arc;

use photocraft_doc::vector::Path;
use photocraft_psd::csh::{self, CshEntry, CshShape};
use photocraft_psd::path::PathData;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::presets::{always, bad, str_param};
use crate::shape_index::ShapeSource;
use crate::{EngineError, Result, Session};

/// Longest library name kept, in characters.
const MAX_GROUP_NAME: usize = 255;
/// The group a file without a usable name loads into.
const DEFAULT_GROUP: &str = "Imported Shapes";

/// A `.csh` file's shapes, decoded one at a time.
pub struct CshSource {
    data: Arc<[u8]>,
    entries: Vec<CshEntry>,
    /// The entries' names, made unique within the file (shapes are addressed by name).
    names: Vec<String>,
}

impl std::fmt::Debug for CshSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CshSource").field("bytes", &self.data.len()).field("shapes", &self.entries.len()).finish()
    }
}

impl CshSource {
    /// Read the names of a `.csh` file; decodes no path.
    pub fn new(data: Vec<u8>) -> Result<Self> {
        let entries = csh::index(&data).map_err(|e| EngineError::Other(format!("not a custom shape file: {e}")))?;
        let mut seen: HashMap<String, usize> = HashMap::new();
        let names = entries
            .iter()
            .map(|e| {
                let base = if e.name.trim().is_empty() { "Shape".to_string() } else { e.name.clone() };
                let n = seen.entry(base.clone()).or_insert(0);
                *n += 1;
                if *n == 1 { base } else { format!("{base} {n}") }
            })
            .collect();
        Ok(CshSource { data: data.into(), entries, names })
    }
}

impl ShapeSource for CshSource {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn name(&self, i: usize) -> Option<&str> {
        self.names.get(i).map(String::as_str)
    }

    fn parse(&self, i: usize) -> Option<Path> {
        let shape = csh::parse_shape(&self.data, self.entries.get(i)?).ok()?;
        Some(path_from_shape(&shape))
    }

    fn raw(&self) -> Option<&[u8]> {
        Some(&self.data)
    }
}

/// A decoded shape as a path in its own pixel space (fractions scaled by its bounds).
pub fn path_from_shape(shape: &CshShape) -> Path {
    let (w, h) = shape.size();
    let records = PathData::from_subpaths(&shape.subpaths, false);
    photocraft_io::vector_map::path_from_records(&records, w.round().clamp(1.0, f64::from(u32::MAX)) as u32, h.round().clamp(1.0, f64::from(u32::MAX)) as u32)
}

/// A path as a `.csh` shape: bounds from its control box, coordinates as fractions of them.
/// `None` for an empty path or one with no finite extent.
pub fn shape_from_path(path: &Path) -> Option<CshShape> {
    let (x0, y0, x1, y1) = path.control_bounds().filter(|b| [b.0, b.1, b.2, b.3].iter().all(|v| v.is_finite()))?;
    // Fractions are relative to whole-pixel bounds, so the box is rounded up and moved to the origin.
    let (w, h) = ((x1 - x0).ceil().clamp(1.0, 1.0e6), (y1 - y0).ceil().clamp(1.0, 1.0e6));
    let moved = path.transform(&photocraft_geom::Affine { m: [1.0, 0.0, 0.0, 1.0, -x0, -y0] });
    let records = photocraft_io::vector_map::path_to_records(&moved, w as u32, h as u32);
    Some(CshShape { bounds: [0, 0, h as i32, w as i32], subpaths: records.subpaths() })
}

/// Add a decoded-on-demand library to the session; the name it got.
fn add_library(s: &mut Session, name: &str, source: CshSource) -> String {
    let taken = |n: &str| crate::shape_index::index(s).iter().any(|g| g.name == n);
    let base: String = name.chars().take(MAX_GROUP_NAME).collect();
    let base = if base.trim().is_empty() { DEFAULT_GROUP.to_string() } else { base };
    let group = if taken(&base) { (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.clone()) } else { base };
    s.presets.lazy_shapes.add_group(group.clone(), Arc::new(source));
    group
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.presets.cshLoad";
    let replace = match str_param(p, "mode") {
        None | Some("append") => false,
        Some("replace") => true,
        Some(m) => return Err(bad(CMD, format!("unknown mode `{m}` (append|replace)"))),
    };
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, CMD, ".csh file")?;
    let source = CshSource::new(bytes)?;
    if source.is_empty() {
        return Err(bad(CMD, "the file holds no shapes"));
    }
    let count = source.len();
    if replace {
        s.presets.lazy_shapes.clear();
    }
    let name = str_param(p, "name").map(str::to_string).filter(|n| !n.is_empty()).unwrap_or(stem);
    let group = add_library(s, &name, source);
    s.presets_changed();
    Ok(json!({"group": group, "count": count}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.presets.cshSave";
    let group = str_param(p, "group");
    let wanted: Option<Vec<&str>> = match p.get("names") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(a.iter().map(|n| n.as_str().ok_or_else(|| bad(CMD, "`names` must be strings"))).collect::<Result<_>>()?),
        Some(_) => return Err(bad(CMD, "`names` must be a list of shape names")),
    };
    let mut shapes: Vec<(String, CshShape)> = Vec::new();
    for g in crate::shape_index::index(s).iter().filter(|g| group.is_none_or(|n| g.name == n)) {
        for name in g.items.iter().filter(|n| wanted.as_ref().is_none_or(|w| w.contains(&n.as_str()))) {
            if let Some(shape) = crate::shape_index::find_shape(s, name, Some(&g.name)).and_then(|sh| shape_from_path(&sh.path)) {
                shapes.push((name.clone(), shape));
            }
        }
    }
    if shapes.is_empty() {
        return Err(bad(CMD, "there are no shapes to save"));
    }
    let bytes = csh::write(&shapes).map_err(|e| EngineError::Other(format!("could not write the shapes: {e}")))?;
    let mut out = json!({"count": shapes.len(), "bytes": bytes.len()});
    match str_param(p, "path") {
        Some(path) => {
            crate::file_cmds::write_file(path, &bytes)?;
            out["path"] = json!(path);
        }
        None => out["data"] = json!(photocraft_paint::tile::b64_encode(&bytes)),
    }
    Ok(out)
}

/// The libraries to keep in the preferences: (name, base64 file bytes).
pub(crate) fn to_json(s: &Session) -> Value {
    let libs: Vec<Value> =
        s.presets.lazy_shapes.groups().iter().filter_map(|g| g.source.raw().map(|raw| json!({"name": g.name, "data": photocraft_paint::tile::b64_encode(raw)}))).collect();
    Value::Array(libs)
}

/// Restore libraries saved by [`to_json`]; a damaged one is skipped, the rest load.
pub(crate) fn from_json(s: &mut Session, v: &Value) {
    s.presets.lazy_shapes.clear();
    for lib in v.as_array().into_iter().flatten().take(64) {
        let (Some(name), Some(data)) = (lib.get("name").and_then(Value::as_str), lib.get("data").and_then(Value::as_str)) else { continue };
        let Some(bytes) = photocraft_paint::tile::b64_decode(data) else { continue };
        if let Ok(source) = CshSource::new(bytes) {
            add_library(s, name, source);
        }
    }
}

/// Command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "shape.presets.cshLoad",
            label: "Load Shapes…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".csh file"?,"data":base64 bytes?,"mode":"append"|"replace"="append","name":group name?=file name} → {group,count}. Only the names are read; a shape's path is decoded when it is used. Replace drops the other loaded libraries."##,
            enabled: always,
            run: load,
            journal: false,
        },
        CommandSpec {
            id: "shape.presets.cshSave",
            label: "Save Shapes…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":str?,"group":name?,"names":[str]?} → {count,bytes,path|data (base64)}. Saves the shapes of `group` (all groups when omitted), or only `names`."##,
            enabled: always,
            run: save,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
