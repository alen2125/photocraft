//! Custom shape files (`.csh`) for the Shapes panel (fork-local, see MY_CHANGES.md): Load /
//! Replace / Save Shapes. A loaded file stays a lazy library (`shape_index`): only its names are
//! read up front, and a shape's path is decoded when something asks for it. Photoshop's shape
//! libraries run to 200 MB, so a file loaded by path is never read whole: its payloads are read
//! from disk one at a time, and the preferences keep the file's path, not its bytes.

use std::collections::HashMap;
use std::sync::Arc;

use photocraft_doc::vector::Path;
use photocraft_psd::csh::{self, CshEntry, CshShape};
use photocraft_psd::path::PathData;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::presets::{always, bad, str_param};
use crate::shape_index::{Origin, ShapeSource};
use crate::{EngineError, Result, Session};

/// Longest library name kept, in characters.
const MAX_GROUP_NAME: usize = 255;
/// The group a file without a usable name loads into.
const DEFAULT_GROUP: &str = "Imported Shapes";
/// Libraries held in memory (loaded from `data`) are kept in the preferences only up to this size.
const MAX_PERSISTED_BYTES: usize = 2 << 20;
/// Libraries restored from the preferences at start.
const MAX_RESTORED: usize = 64;

/// Where a library's payloads are read from.
enum Backing {
    Bytes(Arc<[u8]>),
    #[cfg(not(target_arch = "wasm32"))]
    File(std::path::PathBuf),
}

/// A `.csh` file's shapes, decoded one at a time.
pub struct CshSource {
    backing: Backing,
    entries: Vec<CshEntry>,
    /// The entries' names, made unique within the file (shapes are addressed by name).
    names: Vec<String>,
}

impl std::fmt::Debug for CshSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let from = match &self.backing {
            Backing::Bytes(b) => format!("{} bytes in memory", b.len()),
            #[cfg(not(target_arch = "wasm32"))]
            Backing::File(p) => p.display().to_string(),
        };
        f.debug_struct("CshSource").field("from", &from).field("shapes", &self.entries.len()).finish()
    }
}

fn unique_names(entries: &[CshEntry]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    entries
        .iter()
        .map(|e| {
            let base = if e.name.trim().is_empty() { "Shape".to_string() } else { e.name.clone() };
            let n = seen.entry(base.clone()).or_insert(0);
            *n += 1;
            if *n == 1 { base } else { format!("{base} {n}") }
        })
        .collect()
}

fn not_a_shape_file(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("not a custom shape file: {e}"))
}

impl CshSource {
    /// Read the names of a `.csh` file held in memory; decodes no path.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let entries = csh::index(&data).map_err(not_a_shape_file)?;
        let names = unique_names(&entries);
        Ok(CshSource { backing: Backing::Bytes(data.into()), entries, names })
    }

    /// Read the names of a `.csh` file on disk without reading its shapes; they are read from the
    /// file when asked for.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let mut f = std::fs::File::open(path).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
        let entries = csh::index_reader(&mut f).map_err(not_a_shape_file)?;
        let names = unique_names(&entries);
        Ok(CshSource { backing: Backing::File(path.to_path_buf()), entries, names })
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
        let entry = self.entries.get(i)?;
        let shape = match &self.backing {
            Backing::Bytes(data) => csh::parse_shape(data, entry).ok()?,
            #[cfg(not(target_arch = "wasm32"))]
            Backing::File(path) => {
                use std::io::{Read, Seek, SeekFrom};
                let mut f = std::fs::File::open(path).ok()?;
                f.seek(SeekFrom::Start(u64::try_from(entry.offset).ok()?)).ok()?;
                let mut payload = vec![0u8; entry.len];
                f.read_exact(&mut payload).ok()?;
                csh::parse_payload(&payload).ok()?
            }
        };
        Some(path_from_shape(&shape))
    }

    fn origin(&self) -> Option<Origin<'_>> {
        Some(match &self.backing {
            Backing::Bytes(b) => Origin::Bytes(b),
            #[cfg(not(target_arch = "wasm32"))]
            Backing::File(p) => Origin::File(p),
        })
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

/// The library to load: a file on disk by `path` (indexed in place, never read whole), else the
/// base64 `data`; with the name its file had.
fn source_of(p: &Value, cmd: &str) -> Result<(CshSource, String)> {
    #[cfg(not(target_arch = "wasm32"))]
    if p.get("data").is_none()
        && let Some(path) = str_param(p, "path")
    {
        let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        return Ok((CshSource::from_file(std::path::Path::new(path))?, stem));
    }
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, cmd, ".csh file")?;
    Ok((CshSource::from_bytes(bytes)?, stem))
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.presets.cshLoad";
    let replace = match str_param(p, "mode") {
        None | Some("append") => false,
        Some("replace") => true,
        Some(m) => return Err(bad(CMD, format!("unknown mode `{m}` (append|replace)"))),
    };
    let (source, stem) = source_of(p, CMD)?;
    if source.is_empty() {
        return Err(bad(CMD, "the file holds no shapes"));
    }
    let count = source.len();
    if replace {
        // Replace swaps the whole library, the built-in groups too (`shape.presets.reset` brings them back).
        s.presets.lazy_shapes.clear();
        s.presets.shapes.clear();
    }
    let name = str_param(p, "name").map(str::to_string).filter(|n| !n.is_empty()).unwrap_or(stem);
    let first = source.name(0).map(str::to_string);
    let group = add_library(s, &name, source);
    s.presets_changed();
    Ok(json!({"group": group, "count": count, "first": first}))
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
            if let Some(shape) = crate::shape_index::shape_path(s, name, Some(&g.name)).and_then(|path| shape_from_path(&path)) {
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

/// The libraries to keep in the preferences: a file by its path, a library held in memory by its
/// bytes (when small). A big one that came without a file cannot be kept.
pub(crate) fn to_json(s: &Session) -> Value {
    let libs = s.presets.lazy_shapes.groups().iter().filter_map(|g| match g.source.origin()? {
        Origin::File(path) => Some(json!({"name": g.name, "path": path.to_string_lossy()})),
        Origin::Bytes(b) if b.len() <= MAX_PERSISTED_BYTES => Some(json!({"name": g.name, "data": photocraft_paint::tile::b64_encode(b)})),
        Origin::Bytes(_) => None,
    });
    Value::Array(libs.collect())
}

/// Restore libraries saved by [`to_json`]; a damaged one, or a file that is gone, is skipped.
pub(crate) fn from_json(s: &mut Session, v: &Value) {
    s.presets.lazy_shapes.clear();
    for lib in v.as_array().into_iter().flatten().take(MAX_RESTORED) {
        let Some(name) = lib.get("name").and_then(Value::as_str) else { continue };
        let source = match (lib.get("path").and_then(Value::as_str), lib.get("data").and_then(Value::as_str)) {
            #[cfg(not(target_arch = "wasm32"))]
            (Some(path), _) => CshSource::from_file(std::path::Path::new(path)),
            (_, Some(data)) => match photocraft_paint::tile::b64_decode(data) {
                Some(bytes) => CshSource::from_bytes(bytes),
                None => continue,
            },
            _ => continue,
        };
        if let Ok(source) = source {
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
            params: r##"{"path":".csh file"?,"data":base64 bytes?,"mode":"append"|"replace"="append","name":group name?=file name} → {group,count,first}. A file given by `path` is indexed in place (its names only, never read whole); a shape's path is decoded when it is used. Replace drops the other loaded libraries and the built-in groups (shape.presets.reset restores them)."##,
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
