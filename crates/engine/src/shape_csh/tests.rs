use serde_json::{Value, json};

use super::*;
use crate::shape_index::{find_shape, index};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s
}

fn saved(s: &mut Session, p: Value) -> Vec<u8> {
    let r = s.execute("shape.presets.cshSave", p).unwrap();
    photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()
}

fn load(s: &mut Session, bytes: &[u8], extra: Value) -> crate::Result<Value> {
    let mut p = json!({"data": photocraft_paint::tile::b64_encode(bytes)});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s.execute("shape.presets.cshLoad", p)
}

fn square_file(n: usize) -> Vec<u8> {
    let sub = |k: &mut Vec<photocraft_psd::path::PsdKnot>, x: f64, y: f64| {
        k.push(photocraft_psd::path::PsdKnot { linked: false, pre: (x, y), anchor: (x, y), post: (x, y) })
    };
    let shapes: Vec<(String, CshShape)> = (0..n)
        .map(|i| {
            let mut knots = Vec::new();
            for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                sub(&mut knots, x, y);
            }
            (
                format!("Sq{i}"),
                CshShape { bounds: [0, 0, 30, 20 + (i % 7) as i32], subpaths: vec![photocraft_psd::path::PsdSubpath { closed: true, operation: 1, knots }] },
            )
        })
        .collect();
    csh::write(&shapes).unwrap()
}

#[test]
fn saved_shapes_load_back_with_their_proportions() {
    let mut s = session();
    let bytes = saved(&mut s, json!({"names": ["Heart", "Star"]}));
    let mut t = session();
    let r = load(&mut t, &bytes, json!({"name": "Mine"})).unwrap();
    assert_eq!((r["group"].as_str(), r["count"].as_u64()), (Some("Mine"), Some(2)));
    let idx = index(&t);
    assert_eq!(idx.last().unwrap().items, ["Heart", "Star"]);
    for name in ["Heart", "Star"] {
        let want = find_shape(&s, name, None).unwrap().path.control_bounds().unwrap();
        let got = find_shape(&t, name, Some("Mine")).unwrap().path.control_bounds().unwrap();
        let (wa, ha) = (want.2 - want.0, want.3 - want.1);
        let (wb, hb) = (got.2 - got.0, got.3 - got.1);
        assert!((wa / ha - wb / hb).abs() < 0.03, "{name}: aspect {} vs {}", wa / ha, wb / hb);
    }
}

#[test]
fn a_library_of_5000_shapes_loads_without_decoding_any_path() {
    let mut s = session();
    let r = load(&mut s, &square_file(5000), json!({"name": "Big"})).unwrap();
    assert_eq!(r["count"], json!(5000));
    assert_eq!(s.presets.lazy_shapes.parse_count(), 0);
    assert_eq!(index(&s).last().unwrap().items.len(), 5000);
    assert!(find_shape(&s, "Sq4321", Some("Big")).is_some());
    assert_eq!(s.presets.lazy_shapes.parse_count(), 1);
}

#[test]
fn a_loaded_shape_places_and_fills_by_name() {
    let mut s = session();
    load(&mut s, &square_file(10), json!({"name": "Squares"})).unwrap();
    s.execute("shape.presets.place", json!({"preset": "Sq3", "group": "Squares", "rect": [10, 10, 40, 40]})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("shape.presets.fillPixels", json!({"preset": "Sq4", "rect": [10, 10, 40, 40], "color": "#ff0000"})).unwrap();
    let d = s.active().unwrap();
    let a = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(30, 30)[3];
    assert!(a > 0.99, "{a}");
}

#[test]
fn names_are_made_unique_and_append_or_replace_decide_what_stays() {
    let mut s = session();
    let dup = csh::write(&[
        ("A".into(), csh::parse_shape(&square_file(1), &csh::index(&square_file(1)).unwrap()[0]).unwrap()),
        ("A".into(), csh::parse_shape(&square_file(1), &csh::index(&square_file(1)).unwrap()[0]).unwrap()),
    ])
    .unwrap();
    load(&mut s, &dup, json!({"name": "Dups"})).unwrap();
    assert_eq!(index(&s).last().unwrap().items, ["A", "A 2"]);
    let r = load(&mut s, &square_file(2), json!({"name": "Dups"})).unwrap();
    assert_eq!(r["group"], json!("Dups 2"), "a taken group name gets a number");
    assert_eq!(s.presets.lazy_shapes.groups().len(), 2);
    load(&mut s, &square_file(2), json!({"name": "Only", "mode": "replace"})).unwrap();
    assert_eq!(s.presets.lazy_shapes.groups().len(), 1, "Replace drops the other loaded libraries");
    assert!(load(&mut s, &square_file(1), json!({"mode": "sideways"})).is_err());
}

#[test]
fn bad_files_and_bad_params_are_errors() {
    let mut s = session();
    assert!(load(&mut s, b"not a shape file", json!({})).is_err());
    assert!(load(&mut s, &square_file(2)[..30], json!({})).is_err(), "truncated");
    assert!(load(&mut s, &csh::write(&[]).unwrap(), json!({})).is_err(), "no shapes");
    assert!(s.execute("shape.presets.cshLoad", json!({})).is_err(), "no file");
    assert!(s.execute("shape.presets.cshSave", json!({"group": "Nope"})).is_err());
    assert!(s.execute("shape.presets.cshSave", json!({"names": [1]})).is_err());
    assert!(s.presets.lazy_shapes.is_empty());
}

#[test]
fn a_shape_with_a_damaged_payload_is_listed_but_gives_no_path() {
    let mut bytes = square_file(3);
    let entries = csh::index(&bytes).unwrap();
    // The payload's GUID length runs past its end.
    bytes[entries[1].offset] = 255;
    let mut s = session();
    load(&mut s, &bytes, json!({"name": "Hurt"})).unwrap();
    assert!(find_shape(&s, "Sq0", Some("Hurt")).is_some());
    assert!(find_shape(&s, "Sq1", Some("Hurt")).is_none());
    assert!(find_shape(&s, "Sq2", Some("Hurt")).is_some());
    assert!(s.execute("shape.presets.place", json!({"preset": "Sq1"})).is_err());
}

#[test]
fn libraries_survive_a_restart_without_being_decoded() {
    let mut s = session();
    load(&mut s, &square_file(50), json!({"name": "Kept"})).unwrap();
    let saved = s.presets.to_json(&s);
    let mut t = session();
    t.load_presets_json(saved);
    assert_eq!(index(&t).last().map(|g| (g.name.as_str(), g.items.len())), Some(("Kept", 50)));
    assert_eq!(t.presets.lazy_shapes.parse_count(), 0);
    // A damaged library entry is skipped and the rest load.
    let mut v = s.presets.to_json(&s);
    v["csh_libraries"] = json!([{"name": "Bad", "data": "!!!"}, {"name": "Short"}, v["csh_libraries"][0].clone()]);
    let mut u = session();
    u.load_presets_json(v);
    assert_eq!(u.presets.lazy_shapes.groups().len(), 1);
}

#[test]
fn an_empty_or_unmeasurable_path_has_no_csh_form() {
    assert!(shape_from_path(&Path::default()).is_none());
}

#[test]
fn replace_swaps_out_the_built_in_shapes_and_reset_brings_them_back() {
    let mut s = session();
    assert!(find_shape(&s, "Heart", None).is_some());
    let r = load(&mut s, &square_file(3), json!({"name": "Mine", "mode": "replace"})).unwrap();
    assert_eq!(r["first"], json!("Sq0"));
    assert!(find_shape(&s, "Heart", None).is_none(), "the built-in groups are gone");
    assert_eq!(index(&s).iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Mine"]);
    assert!(s.execute("shape.presets.place", json!({"preset": "Heart"})).is_err());
    s.execute("shape.presets.place", json!({"preset": "Sq1", "rect": [5, 5, 30, 30]})).unwrap();
    s.execute("shape.presets.reset", json!({"append": true})).unwrap();
    assert!(find_shape(&s, "Heart", None).is_some(), "reset restores the defaults next to the loaded library");
    // Appending keeps what was there.
    let mut t = session();
    load(&mut t, &square_file(3), json!({"name": "Extra"})).unwrap();
    assert!(find_shape(&t, "Heart", None).is_some());
}

fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-csh-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Lineart.csh");
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_file_loaded_by_path_stays_on_disk_and_the_preferences_keep_its_path() {
    let path = temp_file("path", &square_file(40));
    let mut s = session();
    let r = s.execute("shape.presets.cshLoad", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!((r["group"].as_str(), r["count"].as_u64()), (Some("Lineart"), Some(40)));
    let g = &s.presets.lazy_shapes.groups()[0];
    assert!(matches!(g.source.origin(), Some(crate::shape_index::Origin::File(_))), "indexed in place, not read whole");
    assert_eq!(s.presets.lazy_shapes.parse_count(), 0);
    // A shape is read from the file when asked for, with the same result as from memory.
    let from_file = find_shape(&s, "Sq7", Some("Lineart")).unwrap().path;
    let mut t = session();
    load(&mut t, &square_file(40), json!({"name": "Mem"})).unwrap();
    assert_eq!(find_shape(&t, "Sq7", Some("Mem")).unwrap().path, from_file);
    // The preferences hold the path, not the 40 shapes' bytes.
    let saved = s.presets.to_json(&s);
    assert_eq!(saved["csh_libraries"][0]["path"], json!(path.to_string_lossy()));
    assert!(saved["csh_libraries"][0].get("data").is_none());
    let mut u = session();
    u.load_presets_json(saved.clone());
    assert_eq!(index(&u).last().map(|g| (g.name.as_str(), g.items.len())), Some(("Lineart", 40)));
    assert!(find_shape(&u, "Sq39", None).is_some());
    // The file is gone: the library is skipped, nothing else is lost.
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    let mut v = session();
    v.load_presets_json(saved);
    assert!(v.presets.lazy_shapes.is_empty());
    assert!(find_shape(&v, "Heart", None).is_some());
}

#[test]
fn a_file_that_is_not_a_shape_file_or_is_cut_short_is_refused_with_a_message() {
    let bad_file = temp_file("bad", b"definitely not a csh file");
    let cut = temp_file("cut", &square_file(3)[..40]);
    let mut s = session();
    for p in [&bad_file, &cut] {
        let e = s.execute("shape.presets.cshLoad", json!({"path": p.to_string_lossy()})).unwrap_err().to_string();
        assert!(e.contains("not a custom shape file"), "{e}");
    }
    assert!(s.execute("shape.presets.cshLoad", json!({"path": "Z:/no/such/file.csh"})).is_err());
    assert!(s.presets.lazy_shapes.is_empty());
    for p in [bad_file, cut] {
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
}

#[test]
fn a_big_library_without_a_file_is_not_written_into_the_preferences() {
    // 15 000 small shapes come to over 2 MB: too big to keep as bytes, and there is no file to point at.
    let big = square_file(15_000);
    assert!(big.len() > super::MAX_PERSISTED_BYTES);
    let mut s = session();
    load(&mut s, &big, json!({"name": "Big"})).unwrap();
    assert_eq!(s.presets.to_json(&s)["csh_libraries"].as_array().map(Vec::len), Some(0));
    // A small one is kept as bytes.
    let mut t = session();
    load(&mut t, &square_file(50), json!({"name": "Small"})).unwrap();
    let saved = t.presets.to_json(&t);
    assert_eq!(saved["csh_libraries"].as_array().map(Vec::len), Some(1));
    assert!(saved["csh_libraries"][0].get("data").is_some());
}

/// Run with `CSH_SAMPLE=<a real .csh> cargo test -p photocraft-engine --lib real_library -- --ignored --nocapture`.
#[test]
#[ignore = "needs a real Photoshop .csh file in CSH_SAMPLE"]
fn a_real_library_loads_by_path_and_its_shapes_place_and_fill() {
    let Some(path) = std::env::var_os("CSH_SAMPLE") else { return };
    let mut s = session();
    let t = std::time::Instant::now();
    let r = s.execute("shape.presets.cshLoad", json!({"path": path.to_string_lossy(), "mode": "replace"})).unwrap();
    let loaded = t.elapsed();
    let count = r["count"].as_u64().unwrap();
    let first = r["first"].as_str().unwrap().to_string();
    assert_eq!(s.presets.lazy_shapes.parse_count(), 0);
    let t = std::time::Instant::now();
    let p = find_shape(&s, &first, None).unwrap().path;
    let knots: usize = p.subpaths.iter().map(|sp| sp.knots.len()).sum();
    let parsed = t.elapsed();
    let t = std::time::Instant::now();
    // A vector shape layer holds at most 4096 knots; a traced drawing has far more.
    let placed_vector = s.execute("shape.presets.place", json!({"preset": first, "rect": [10, 10, 80, 80]}));
    assert_eq!(placed_vector.is_ok(), knots <= 4096, "{knots} knots");
    let placed = t.elapsed();
    s.execute("layer.new.layer", json!({})).unwrap();
    let t = std::time::Instant::now();
    let r = s.execute("shape.presets.fillPixels", json!({"preset": first, "rect": [10, 10, 80, 80], "color": "#000000"})).unwrap();
    let filled = t.elapsed();
    assert_eq!(r["empty"], json!(false));
    println!("{count} shapes: indexed in {loaded:?}; first = {knots} knots, read in {parsed:?}; place {placed:?}; fill {filled:?}");
}
