use super::*;
use crate::path::PsdKnot;

fn square(op: i16) -> PsdSubpath {
    let k = |x: f64, y: f64| PsdKnot { linked: false, pre: (x, y), anchor: (x, y), post: (x, y) };
    PsdSubpath { closed: true, operation: op, knots: vec![k(0.0, 0.0), k(1.0, 0.0), k(1.0, 1.0), k(0.0, 1.0)] }
}

fn shape(w: i32, h: i32) -> CshShape {
    CshShape { bounds: [0, 0, h, w], subpaths: vec![square(1)] }
}

fn library(n: usize) -> Vec<(String, CshShape)> {
    (0..n).map(|i| (format!("Shape {i}"), shape(10 + i as i32, 20))).collect()
}

#[test]
fn round_trips_names_bounds_and_paths() {
    let lib = library(3);
    let bytes = write(&lib).unwrap();
    assert_eq!(&bytes[..4], b"cush");
    let idx = index(&bytes).unwrap();
    assert_eq!(idx.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Shape 0", "Shape 1", "Shape 2"]);
    for (e, (_, want)) in idx.iter().zip(&lib) {
        let got = parse_shape(&bytes, e).unwrap();
        assert_eq!(got.bounds, want.bounds);
        assert_eq!(got.subpaths.len(), 1);
        assert_eq!(got.subpaths[0].knots.len(), 4);
        for (a, b) in got.subpaths[0].knots.iter().zip(&want.subpaths[0].knots) {
            assert!((a.anchor.0 - b.anchor.0).abs() < 1e-6 && (a.anchor.1 - b.anchor.1).abs() < 1e-6);
        }
    }
}

#[test]
fn indexing_reads_no_path_and_a_damaged_payload_only_fails_its_own_shape() {
    let mut bytes = write(&library(3)).unwrap();
    let idx = index(&bytes).unwrap();
    // Corrupt the second payload's version: the index still lists all three.
    bytes[idx[1].offset..idx[1].offset + 4].copy_from_slice(&99u32.to_be_bytes());
    let idx2 = index(&bytes).unwrap();
    assert_eq!(idx2.len(), 3);
    assert!(parse_shape(&bytes, &idx2[0]).is_ok());
    assert!(parse_shape(&bytes, &idx2[1]).is_err());
    assert!(parse_shape(&bytes, &idx2[2]).is_ok());
}

#[test]
fn a_24_byte_payload_header_is_read_too() {
    let mut bytes = b"cush".to_vec();
    bytes.put_u32(2);
    bytes.put_u32(1);
    bytes.put_u32(2);
    bytes.put_u16(u16::from(b'A'));
    bytes.put_u16(0);
    let records = PathData::from_subpaths(&[square(1)], false).to_bytes();
    bytes.put_u32(24 + records.len() as u32);
    bytes.put_u32(3);
    for b in [0u32, 0, 8, 8] {
        bytes.put_u32(b);
    }
    bytes.put_u32(0);
    bytes.put(&records);
    while !bytes.len().is_multiple_of(4) {
        bytes.put_u8(0);
    }
    let idx = index(&bytes).unwrap();
    assert_eq!(idx[0].name, "A");
    assert_eq!(parse_shape(&bytes, &idx[0]).unwrap().subpaths[0].knots.len(), 4);
}

#[test]
fn truncated_files_are_errors_never_panics() {
    let bytes = write(&library(2)).unwrap();
    for cut in 0..bytes.len() {
        assert!(index(&bytes[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn hostile_headers_are_errors() {
    assert!(index(b"").is_err());
    assert!(index(b"cusX\0\0\0\x02\0\0\0\0").is_err(), "wrong signature");
    assert!(index(b"cush\0\0\0\x07\0\0\0\0").is_err(), "unknown version");
    assert!(index(b"cush\0\0\0\x02\xff\xff\xff\xff").is_err(), "absurd count");
    // A payload size past the end of the file.
    let mut v = b"cush".to_vec();
    v.put_u32(2);
    v.put_u32(1);
    v.put_u32(1);
    v.put_u16(0);
    v.put_u16(0);
    v.put_u32(0x7fff_ffff);
    assert!(index(&v).is_err());
    // A payload over the cap, even with the bytes present.
    let mut v = b"cush".to_vec();
    v.put_u32(2);
    v.put_u32(1);
    v.put_u32(1);
    v.put_u16(0);
    v.put_u16(0);
    v.put_u32(MAX_SHAPE_BYTES as u32 + 1);
    assert!(matches!(index(&v), Err(PsdError::LimitExceeded(_))));
}

#[test]
fn entries_pointing_outside_the_data_or_at_garbage_are_errors() {
    let bytes = write(&library(1)).unwrap();
    let e = index(&bytes).unwrap().remove(0);
    assert!(parse_shape(&bytes[..e.offset + 2], &e).is_err());
    assert!(parse_shape(&bytes, &CshEntry { name: String::new(), offset: usize::MAX, len: 10 }).is_err());
    assert!(parse_shape(&bytes, &CshEntry { name: String::new(), offset: e.offset, len: 4 }).is_err());
    let empty = write(&[("E".into(), CshShape { bounds: [0, 0, 1, 1], subpaths: vec![] })]).unwrap();
    let e = index(&empty).unwrap().remove(0);
    assert!(parse_shape(&empty, &e).is_err(), "no path");
}

#[test]
fn names_are_capped_without_splitting_a_surrogate_pair_and_keep_the_stream_in_sync() {
    let long = format!("{}\u{1F600}", "a".repeat(MAX_NAME_UNITS - 1));
    let lib = vec![(long, shape(5, 5)), ("Next".to_string(), shape(6, 6))];
    let bytes = write(&lib).unwrap();
    let idx = index(&bytes).unwrap();
    assert_eq!(idx[0].name, "a".repeat(MAX_NAME_UNITS - 1));
    assert_eq!(idx[1].name, "Next");
    assert_eq!(parse_shape(&bytes, &idx[1]).unwrap().bounds, [0, 0, 6, 6]);
}

#[test]
fn a_shape_over_the_size_cap_cannot_be_written() {
    let knot = PsdKnot { linked: false, pre: (0.0, 0.0), anchor: (0.0, 0.0), post: (0.0, 0.0) };
    let huge = CshShape { bounds: [0, 0, 1, 1], subpaths: vec![PsdSubpath { closed: true, operation: 1, knots: vec![knot; 60_000] }; 3] };
    assert!(write(&[("Huge".into(), huge)]).is_err());
    let long = CshShape { bounds: [0, 0, 1, 1], subpaths: vec![PsdSubpath { closed: true, operation: 1, knots: vec![knot; 70_000] }] };
    assert!(write(&[("Long".into(), long)]).is_err(), "a 16-bit knot count would truncate it");
}

#[test]
fn bounds_with_no_area_still_give_finite_sizes() {
    let s = CshShape { bounds: [5, 5, 5, 5], subpaths: vec![] };
    assert_eq!(s.size(), (1.0, 1.0));
    let s = CshShape { bounds: [i32::MIN, i32::MIN, i32::MAX, i32::MAX], subpaths: vec![] };
    assert!(s.size().0.is_finite() && s.size().1.is_finite());
}
