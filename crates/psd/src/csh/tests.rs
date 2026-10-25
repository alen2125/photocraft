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

/// A file header with one shape whose payload is `size` bytes long (not included).
fn header_with_size(size: u32) -> Vec<u8> {
    let mut v = b"cush".to_vec();
    v.put_u32(2);
    v.put_u32(1);
    v.put_u32(1);
    v.put_u16(0);
    v.put_u16(0);
    v.put_u32(1);
    v.put_u32(size);
    v
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
fn the_layout_matches_the_real_files() {
    // One shape named "Ab", as Photoshop lays it out: the name's count includes the null, padding
    // brings the version to a multiple of 4, then the version (1), the size and the payload
    // (a 36-character GUID, the bounds, the records and spare bytes up to a multiple of 4).
    let bytes = write(&[("Ab".to_string(), CshShape { bounds: [-7, -2, 1502, 1128], subpaths: vec![square(1)] })]).unwrap();
    assert_eq!(&bytes[..12], b"cush\0\0\0\x02\0\0\0\x01");
    assert_eq!(&bytes[12..16], &[0, 0, 0, 3], "name: 2 units and the null");
    assert_eq!(&bytes[16..22], &[0, b'A', 0, b'b', 0, 0]);
    assert_eq!(&bytes[22..24], &[0, 0], "padding to a multiple of 4 from the file start");
    assert_eq!(&bytes[24..28], &[0, 0, 0, 1], "shape version");
    let size = u32::from_be_bytes(bytes[28..32].try_into().unwrap()) as usize;
    assert_eq!(32 + size, bytes.len());
    assert_eq!(size % 4, 0);
    assert_eq!(bytes[32], 36, "Pascal GUID");
    assert_eq!(bytes[33 + 8], b'-');
    assert_eq!(&bytes[69..85], &[0xff, 0xff, 0xff, 0xf9, 0xff, 0xff, 0xff, 0xfe, 0, 0, 5, 0xde, 0, 0, 4, 0x68], "top, left, bottom, right");
    assert_eq!(&bytes[85..87], &[0, 6], "the first record is the fill rule");
}

#[test]
fn reads_a_file_through_a_reader_without_the_payloads() {
    // A reader that counts the bytes read: indexing must read far less than the file holds.
    struct Counting<R>(R, std::cell::Cell<usize>);
    impl<R: Read> Read for Counting<R> {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.read(b)?;
            self.1.set(self.1.get() + n);
            Ok(n)
        }
    }
    impl<R: Seek> Seek for Counting<R> {
        fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(p)
        }
    }
    let big = CshShape {
        bounds: [0, 0, 100, 100],
        subpaths: vec![PsdSubpath {
            closed: true,
            operation: 1,
            knots: vec![PsdKnot { linked: false, pre: (0.5, 0.5), anchor: (0.5, 0.5), post: (0.5, 0.5) }; 20_000],
        }],
    };
    let lib: Vec<_> = (0..10).map(|i| (format!("Big {i}"), big.clone())).collect();
    let bytes = write(&lib).unwrap();
    assert!(bytes.len() > 5_000_000);
    let mut r = Counting(Cursor::new(bytes.clone()), std::cell::Cell::new(0));
    let idx = index_reader(&mut r).unwrap();
    assert_eq!(idx.len(), 10);
    assert!(r.1.get() < 1000, "indexing read {} bytes of {}", r.1.get(), bytes.len());
    assert_eq!(parse_shape(&bytes, &idx[9]).unwrap().subpaths[0].knots.len(), 20_000);
}

#[test]
fn indexing_reads_no_path_and_a_damaged_payload_only_fails_its_own_shape() {
    let mut bytes = write(&library(3)).unwrap();
    let idx = index(&bytes).unwrap();
    // Make the second payload's GUID length longer than the payload: the index still lists all three.
    bytes[idx[1].offset] = 255;
    let idx2 = index(&bytes).unwrap();
    assert_eq!(idx2.len(), 3);
    assert!(parse_shape(&bytes, &idx2[0]).is_ok());
    assert!(parse_shape(&bytes, &idx2[1]).is_err());
    assert!(parse_shape(&bytes, &idx2[2]).is_ok());
}

#[test]
fn data_after_the_last_shape_and_spare_bytes_in_a_payload_are_ignored() {
    let mut bytes = write(&library(2)).unwrap();
    bytes.extend_from_slice(&[7; 5401]);
    let idx = index(&bytes).unwrap();
    assert_eq!(idx.len(), 2);
    assert!(parse_shape(&bytes, &idx[1]).is_ok());
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
    assert!(index(&header_with_size(0x7fff_ffff)).is_err(), "a size past the end of the file");
    assert!(matches!(index(&header_with_size(MAX_SHAPE_BYTES as u32 + 1)), Err(PsdError::LimitExceeded(_))), "over the size cap");
    // A name that claims more units than the file has.
    let mut v = b"cush".to_vec();
    v.put_u32(2);
    v.put_u32(1);
    v.put_u32(0xffff_ffff);
    assert!(index(&v).is_err());
}

#[test]
fn payloads_that_are_not_paths_are_errors() {
    assert!(parse_payload(&[]).is_err());
    assert!(parse_payload(&[200, 1, 2, 3]).is_err(), "a GUID longer than the payload");
    assert!(parse_payload(&[0, 1, 2, 3]).is_err(), "no bounds");
    assert!(parse_payload(&[0; 17]).is_err(), "bounds but no path");
    let bytes = write(&library(1)).unwrap();
    let e = index(&bytes).unwrap().remove(0);
    assert!(parse_shape(&bytes[..e.offset + 2], &e).is_err());
    assert!(parse_shape(&bytes, &CshEntry { name: String::new(), offset: usize::MAX, len: 10 }).is_err());
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
fn a_name_longer_than_the_cap_is_cut_and_the_rest_read_past() {
    // Written by hand: 1030 units, only the first 1024 kept.
    let mut v = b"cush".to_vec();
    v.put_u32(2);
    v.put_u32(1);
    v.put_u32(1030);
    for _ in 0..1030 {
        v.put_u16(u16::from(b'x'));
    }
    while !v.len().is_multiple_of(4) {
        v.put_u8(0);
    }
    v.put_u32(1);
    v.put_u32(0);
    let idx = index(&v).unwrap();
    assert_eq!(idx[0].name.len(), MAX_NAME_UNITS);
    assert_eq!(idx[0].len, 0);
}

#[test]
fn a_shape_over_the_size_cap_cannot_be_written() {
    let knot = PsdKnot { linked: false, pre: (0.0, 0.0), anchor: (0.0, 0.0), post: (0.0, 0.0) };
    let huge = CshShape { bounds: [0, 0, 1, 1], subpaths: vec![PsdSubpath { closed: true, operation: 1, knots: vec![knot; 60_000] }; 12] };
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

/// Run with `CSH_SAMPLE=<a real .csh> cargo test -p photocraft-psd --lib real_file -- --ignored`.
#[test]
#[ignore = "needs a real Photoshop .csh file in CSH_SAMPLE"]
fn a_real_file_indexes_and_its_shapes_decode() {
    let Some(path) = std::env::var_os("CSH_SAMPLE") else { return };
    let mut f = std::fs::File::open(path).unwrap();
    let idx = index_reader(&mut f).unwrap();
    assert!(!idx.is_empty());
    for e in [idx.first().unwrap(), idx.last().unwrap()] {
        let mut payload = vec![0u8; e.len];
        f.seek(SeekFrom::Start(e.offset as u64)).unwrap();
        f.read_exact(&mut payload).unwrap();
        let s = parse_payload(&payload).unwrap();
        assert!(!s.subpaths.is_empty() && s.size().0 > 1.0 && s.size().1 > 1.0, "{:?}", s.bounds);
    }
}
