//! Lazy shape libraries (fork-local, see MY_CHANGES.md): a library such as an imported `.csh`
//! keeps only its names in memory; a shape's path is parsed when something asks for it (the
//! selected shape, a visible thumbnail, `shape.presets.place`) and kept in a small LRU cache.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use photocraft_doc::vector::Path;

use crate::Session;
use crate::presets::shapes::{CUSTOM_GROUP, ShapePreset};

/// Parsed paths kept at once.
pub const PATH_CACHE: usize = 256;

/// Where a lazy library's shapes come from: names are cheap, a path is parsed on demand.
pub trait ShapeSource: Send + Sync + std::fmt::Debug {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn name(&self, i: usize) -> Option<&str>;
    /// Parse shape `i`; `None` when its data is damaged.
    fn parse(&self, i: usize) -> Option<Path>;
    /// Where the library came from, kept in the preferences so it loads again next launch.
    fn origin(&self) -> Option<Origin<'_>> {
        None
    }
}

/// What a lazy library was read from.
#[derive(Clone, Copy, Debug)]
pub enum Origin<'a> {
    /// A file on disk, read a shape at a time.
    File(&'a std::path::Path),
    /// A file held in memory.
    Bytes(&'a [u8]),
}

#[derive(Clone, Debug)]
pub struct LazyGroup {
    pub name: String,
    pub source: Arc<dyn ShapeSource>,
}

/// Least-recently-used paths, keyed by (group, shape) index.
#[derive(Debug, Default)]
struct Lru {
    cap: usize,
    items: Vec<((usize, usize), Arc<Path>)>,
}

impl Lru {
    fn get(&mut self, key: (usize, usize)) -> Option<Arc<Path>> {
        let at = self.items.iter().position(|(k, _)| *k == key)?;
        let hit = self.items.remove(at);
        let path = hit.1.clone();
        self.items.push(hit);
        Some(path)
    }

    fn put(&mut self, key: (usize, usize), path: Arc<Path>) {
        self.items.retain(|(k, _)| *k != key);
        self.items.push((key, path));
        if self.items.len() > self.cap {
            self.items.remove(0);
        }
    }
}

/// The lazy libraries of a session. Cloning shares the cache.
#[derive(Clone, Debug)]
pub struct LazyShapes {
    groups: Vec<LazyGroup>,
    cache: Arc<Mutex<Lru>>,
    parses: Arc<AtomicUsize>,
}

impl Default for LazyShapes {
    fn default() -> Self {
        Self::with_capacity(PATH_CACHE)
    }
}

impl LazyShapes {
    pub fn with_capacity(cap: usize) -> Self {
        LazyShapes { groups: Vec::new(), cache: Arc::new(Mutex::new(Lru { cap: cap.max(1), items: Vec::new() })), parses: Arc::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Add a library, replacing a lazy group of the same name.
    pub fn add_group(&mut self, name: impl Into<String>, source: Arc<dyn ShapeSource>) {
        let name = name.into();
        self.groups.retain(|g| g.name != name);
        self.groups.push(LazyGroup { name, source });
        self.clear_cache();
    }

    pub fn clear(&mut self) {
        self.groups.clear();
        self.clear_cache();
    }

    fn clear_cache(&self) {
        if let Ok(mut c) = self.cache.lock() {
            c.items.clear();
        }
    }

    pub fn groups(&self) -> &[LazyGroup] {
        &self.groups
    }

    /// How many paths have been parsed so far (tests check the library was not parsed whole).
    pub fn parse_count(&self) -> usize {
        self.parses.load(Ordering::Relaxed)
    }

    /// The path of shape `name` (in lazy group `group`, else any), parsed now or taken from the cache.
    pub fn path(&self, group: Option<&str>, name: &str) -> Option<(String, Arc<Path>)> {
        self.groups.iter().enumerate().filter(|(_, g)| group.is_none_or(|n| g.name == n)).find_map(|(gi, g)| {
            let i = (0..g.source.len()).find(|&i| g.source.name(i) == Some(name))?;
            let path = self.path_at(gi, i)?;
            Some((g.name.clone(), path))
        })
    }

    fn path_at(&self, gi: usize, i: usize) -> Option<Arc<Path>> {
        if let Ok(mut c) = self.cache.lock()
            && let Some(p) = c.get((gi, i))
        {
            return Some(p);
        }
        let parsed = Arc::new(self.groups.get(gi)?.source.parse(i)?);
        self.parses.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut c) = self.cache.lock() {
            c.put((gi, i), parsed.clone());
        }
        Some(parsed)
    }
}

/// One group of the shapes index: names only, no paths.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexGroup {
    pub name: String,
    pub items: Vec<String>,
}

/// Every shape the panels show, by name: the presets, Define Custom Shape results and the lazy
/// libraries. Unlike `shapes::all_groups` it clones no paths and parses nothing.
pub fn index(s: &Session) -> Vec<IndexGroup> {
    let mut out: Vec<IndexGroup> =
        s.presets.shapes.iter().map(|g| IndexGroup { name: g.name.clone(), items: g.items.iter().map(|i| i.name.clone()).collect() }).collect();
    if !s.edit_state.custom_shapes.is_empty() {
        out.push(IndexGroup { name: CUSTOM_GROUP.into(), items: s.edit_state.custom_shapes.iter().map(|c| c.name.clone()).collect() });
    }
    for g in s.presets.lazy_shapes.groups() {
        out.push(IndexGroup { name: g.name.clone(), items: (0..g.source.len()).filter_map(|i| g.source.name(i).map(str::to_string)).collect() });
    }
    out
}

/// The shape called `name` (in `group`, else anywhere): presets first, then Define Custom Shape
/// results, then the lazy libraries. Only the one shape's path is cloned or parsed.
pub fn find_shape(s: &Session, name: &str, group: Option<&str>) -> Option<ShapePreset> {
    shape_path(s, name, group).map(|path| ShapePreset { name: name.to_string(), path: (*path).clone() })
}

/// [`find_shape`]'s path without copying it when it is cached (a traced drawing has tens of
/// thousands of knots, and the drag preview asks for it on every pointer move).
pub fn shape_path(s: &Session, name: &str, group: Option<&str>) -> Option<Arc<Path>> {
    let in_group = |g: &str| group.is_none_or(|n| n == g);
    let builtin = s.presets.shapes.iter().filter(|g| in_group(&g.name)).find_map(|g| g.items.iter().find(|i| i.name == name));
    if let Some(sh) = builtin {
        return Some(Arc::new(sh.path.clone()));
    }
    if in_group(CUSTOM_GROUP)
        && let Some(c) = s.edit_state.custom_shapes.iter().find(|c| c.name == name)
    {
        return Some(Arc::new(c.path.clone()));
    }
    s.presets.lazy_shapes.path(group, name).map(|(_, path)| path)
}

#[cfg(test)]
mod tests;
