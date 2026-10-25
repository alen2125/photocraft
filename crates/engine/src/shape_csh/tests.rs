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
    bytes[entries[1].offset..entries[1].offset + 4].copy_from_slice(&9u32.to_be_bytes());
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
