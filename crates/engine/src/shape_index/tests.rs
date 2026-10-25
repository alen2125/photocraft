use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::presets::shapes::parse;

/// A library of `n` squares named "S0".."S{n-1}" whose paths exist only when parsed.
#[derive(Debug)]
struct Synthetic {
    names: Vec<String>,
}

impl Synthetic {
    fn new(n: usize) -> Arc<Self> {
        Arc::new(Synthetic { names: (0..n).map(|i| format!("S{i}")).collect() })
    }
}

impl ShapeSource for Synthetic {
    fn len(&self) -> usize {
        self.names.len()
    }
    fn name(&self, i: usize) -> Option<&str> {
        self.names.get(i).map(String::as_str)
    }
    fn parse(&self, i: usize) -> Option<Path> {
        self.names.get(i)?;
        parse("M 0 0 L 100 0 L 100 100 L 0 100 Z").ok()
    }
}

fn session_with(n: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.presets.lazy_shapes.add_group("Big", Synthetic::new(n));
    s
}

#[test]
fn a_5000_shape_library_opens_without_parsing_any_path() {
    let s = session_with(5000);
    let idx = index(&s);
    let big = idx.iter().find(|g| g.name == "Big").unwrap();
    assert_eq!(big.items.len(), 5000);
    assert_eq!(s.presets.lazy_shapes.parse_count(), 0, "listing the names parses nothing");
}

#[test]
fn only_the_asked_for_shape_is_parsed_and_it_is_cached() {
    let s = session_with(5000);
    assert!(find_shape(&s, "S4321", None).is_some());
    assert_eq!(s.presets.lazy_shapes.parse_count(), 1);
    assert!(find_shape(&s, "S4321", Some("Big")).is_some());
    assert_eq!(s.presets.lazy_shapes.parse_count(), 1, "the second ask is a cache hit");
    assert!(find_shape(&s, "S4321", Some("Other group")).is_none());
    assert!(find_shape(&s, "nope", None).is_none());
}

#[test]
fn the_cache_evicts_the_least_recently_used_path() {
    let mut lazy = LazyShapes::with_capacity(2);
    lazy.add_group("G", Synthetic::new(10));
    for n in ["S1", "S2", "S1", "S3"] {
        lazy.path(None, n).unwrap();
    }
    assert_eq!(lazy.parse_count(), 3, "S1, S2, S3 parsed once each so far");
    lazy.path(None, "S1").unwrap();
    assert_eq!(lazy.parse_count(), 3, "S1 was kept: S2 was the least recently used");
    lazy.path(None, "S2").unwrap();
    assert_eq!(lazy.parse_count(), 4, "S2 was evicted and parses again");
}

#[test]
fn place_and_fill_pixels_use_a_lazy_shape() {
    let mut s = session_with(100);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("shape.presets.fillPixels", json!({"preset": "S7", "rect": [10, 10, 30, 30], "color": "#ff0000"})).unwrap();
    assert_eq!(s.presets.lazy_shapes.parse_count(), 1);
    s.execute("shape.presets.place", json!({"preset": "S8", "rect": [10, 10, 30, 30]})).unwrap();
    assert_eq!(s.presets.lazy_shapes.parse_count(), 2);
    assert!(s.execute("shape.presets.place", json!({"preset": "S100"})).is_err());
}

#[test]
fn the_built_in_shapes_are_found_without_cloning_every_group() {
    let s = Session::new();
    let heart = find_shape(&s, "Heart", None).unwrap();
    assert!(!heart.path.is_empty());
    assert!(find_shape(&s, "Heart", Some("Nature")).is_none());
    let idx = index(&s);
    assert_eq!(idx.len(), crate::presets::shapes::all_groups(&s).len());
}

#[test]
fn a_source_whose_data_is_damaged_gives_no_shape() {
    #[derive(Debug)]
    struct Broken;
    impl ShapeSource for Broken {
        fn len(&self) -> usize {
            1
        }
        fn name(&self, i: usize) -> Option<&str> {
            (i == 0).then_some("Bad")
        }
        fn parse(&self, _: usize) -> Option<Path> {
            None
        }
    }
    let mut lazy = LazyShapes::default();
    lazy.add_group("G", Arc::new(Broken));
    assert!(lazy.path(None, "Bad").is_none());
    assert_eq!(lazy.parse_count(), 0);
}
