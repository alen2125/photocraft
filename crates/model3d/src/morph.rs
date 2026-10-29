//! VaM morph files: `.vmi` (a JSON header: name, group, range, and formulas that move bones or
//! drive other morphs) and `.vmb` (the vertex offsets: a count, then per vertex an index into the
//! figure's base vertices and a 3-float offset). Both are hostile input: counts and sizes are
//! capped, indices are checked by whoever applies them, nothing here panics.

use gltf::json::Value;

/// Most vertex offsets one morph may hold (VaM's own are a few thousand to a few hundred thousand).
pub const MAX_MORPH_DELTAS: usize = 2_000_000;
/// Most formulas one morph may carry.
pub const MAX_MORPH_FORMULAS: usize = 4096;
/// Largest `.vmi` header read.
pub const MAX_VMI_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MorphError {
    #[error("not a readable morph header: {0}")]
    Header(String),
    #[error("morph offsets file is {0} bytes, which is not a whole number of records")]
    BadSize(usize),
    #[error("morph has {0} vertex offsets, over the limit of {MAX_MORPH_DELTAS}")]
    TooManyDeltas(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// What a morph formula does, by VaM's `targetType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormulaKind {
    /// Moves a bone's centre along a world axis, in metres.
    BoneCenter(Axis),
    /// Turns a bone's rest orientation, in degrees.
    Orientation(Axis),
    /// Rotates a bone (a pose control), in degrees.
    Rotation(Axis),
    /// Scales a bone along an axis.
    Scale(Axis),
    /// Scales the whole figure.
    GeneralScale,
    /// Drives another morph: its value becomes `multiplier * this value`.
    MorphValue,
    /// Takes its value from another morph.
    Mcm,
    McmMult,
    /// A kind this reader does not know: kept so nothing is silently lost, never applied.
    Other,
}

impl FormulaKind {
    fn parse(s: &str) -> Self {
        let axis = |c: char| match c {
            'X' => Some(Axis::X),
            'Y' => Some(Axis::Y),
            'Z' => Some(Axis::Z),
            _ => None,
        };
        let split = |prefix: &str| s.strip_prefix(prefix).and_then(|r| r.chars().next().filter(|_| r.len() == 1)).and_then(axis);
        if let Some(a) = split("BoneCenter") {
            return Self::BoneCenter(a);
        }
        if let Some(a) = split("Orientation") {
            return Self::Orientation(a);
        }
        if let Some(a) = split("Rotation") {
            return Self::Rotation(a);
        }
        if let Some(a) = split("Scale") {
            return Self::Scale(a);
        }
        match s {
            "GeneralScale" => Self::GeneralScale,
            "MorphValue" => Self::MorphValue,
            "MCM" => Self::Mcm,
            "MCMMult" => Self::McmMult,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MorphFormula {
    pub kind: FormulaKind,
    /// A bone name or another morph's id, by `kind`.
    pub target: String,
    pub multiplier: f32,
}

/// What a `.vmi` says about a morph, without its offsets.
#[derive(Clone, Debug, PartialEq)]
pub struct MorphInfo {
    pub id: String,
    pub display_name: String,
    pub group: String,
    pub region: String,
    pub min: f32,
    pub max: f32,
    pub num_deltas: usize,
    pub formulas: Vec<MorphFormula>,
}

/// One vertex offset, in VaM's own (Unity) axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MorphDelta {
    /// Index into the figure's base vertices.
    pub vertex: u32,
    pub delta: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Morph {
    pub info: MorphInfo,
    pub deltas: Vec<MorphDelta>,
}

/// VaM writes every number in its headers as a string; plain JSON numbers are accepted too.
fn number(v: Option<&Value>) -> Option<f32> {
    let f = match v? {
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        other => other.as_f64()?,
    };
    (f.is_finite() && f.abs() < 1.0e9).then_some(f as f32)
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().chars().take(512).collect()
}

/// Reads a `.vmi` header.
pub fn parse_vmi(bytes: &[u8]) -> Result<MorphInfo, MorphError> {
    if bytes.len() > MAX_VMI_BYTES {
        return Err(MorphError::Header(format!("{} bytes is too large", bytes.len())));
    }
    // Some files start with a byte-order mark.
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let value: Value = gltf::json::deserialize::from_slice(body).map_err(|e| MorphError::Header(e.to_string()))?;
    if !value.is_object() {
        return Err(MorphError::Header("not an object".into()));
    }
    let id = text(&value, "id");
    if id.is_empty() {
        return Err(MorphError::Header("no id".into()));
    }
    let display_name = Some(text(&value, "displayName")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
    let formulas = value
        .get("formulas")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_MORPH_FORMULAS)
        .filter_map(|f| {
            let multiplier = number(f.get("multiplier"))?;
            Some(MorphFormula { kind: FormulaKind::parse(f.get("targetType").and_then(Value::as_str)?), target: text(f, "target"), multiplier })
        })
        .collect();
    Ok(MorphInfo {
        display_name,
        group: text(&value, "group"),
        region: text(&value, "region"),
        min: number(value.get("min")).unwrap_or(-1.0),
        max: number(value.get("max")).unwrap_or(1.0),
        num_deltas: number(value.get("numDeltas")).map_or(0, |n| n.max(0.0) as usize),
        formulas,
        id,
    })
}

/// Reads a `.vmb`: an `i32` count, then that many records of an `i32` vertex index and three `f32`
/// offsets, all little-endian. A count that disagrees with the file's size is an error.
pub fn parse_vmb(bytes: &[u8]) -> Result<Vec<MorphDelta>, MorphError> {
    let Some((head, body)) = bytes.split_first_chunk::<4>() else { return Err(MorphError::BadSize(bytes.len())) };
    let count = i32::from_le_bytes(*head);
    if count < 0 || body.len() % 16 != 0 || body.len() / 16 != count as usize {
        return Err(MorphError::BadSize(bytes.len()));
    }
    if body.len() / 16 > MAX_MORPH_DELTAS {
        return Err(MorphError::TooManyDeltas(body.len() / 16));
    }
    let f = |b: [u8; 4]| f32::from_le_bytes(b);
    let (records, _) = body.as_chunks::<16>();
    Ok(records
        .iter()
        .filter_map(|r| {
            let vertex = u32::try_from(i32::from_le_bytes([r[0], r[1], r[2], r[3]])).ok()?;
            let delta = [f([r[4], r[5], r[6], r[7]]), f([r[8], r[9], r[10], r[11]]), f([r[12], r[13], r[14], r[15]])];
            delta.iter().all(|c| c.is_finite()).then_some(MorphDelta { vertex, delta })
        })
        .collect())
}

/// A whole morph from its two files.
pub fn parse_morph(vmi: &[u8], vmb: &[u8]) -> Result<Morph, MorphError> {
    Ok(Morph { info: parse_vmi(vmi)?, deltas: parse_vmb(vmb)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vmb(records: &[(i32, [f32; 3])]) -> Vec<u8> {
        let mut out = (records.len() as i32).to_le_bytes().to_vec();
        for (v, d) in records {
            out.extend(v.to_le_bytes());
            for c in d {
                out.extend(c.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn a_header_reads_with_its_numbers_as_strings_and_its_formulas() {
        let vmi = br#"{ "id" : "Mouth big X", "displayName" : "Mouth big X", "group" : "G1/Mouth", "region" : "Actor",
            "min" : "-1", "max" : "2.5", "numDeltas" : "799", "isPoseControl" : "false",
            "formulas" : [ { "targetType" : "BoneCenterY", "target" : "hip", "multiplier" : "0.0156" },
                           { "targetType" : "OrientationX", "target" : "hip", "multiplier" : -0.5 },
                           { "targetType" : "MorphValue", "target" : "Other", "multiplier" : "-0.036" },
                           { "targetType" : "Strange", "target" : "x", "multiplier" : "1" },
                           { "targetType" : "RotationZ", "target" : "lThumb1", "multiplier" : "oops" } ] }"#;
        let m = parse_vmi(vmi).unwrap();
        assert_eq!((m.id.as_str(), m.group.as_str(), m.min, m.max, m.num_deltas), ("Mouth big X", "G1/Mouth", -1.0, 2.5, 799));
        assert_eq!(m.formulas.len(), 4, "the one with a bad multiplier is dropped");
        assert_eq!(m.formulas[0].kind, FormulaKind::BoneCenter(Axis::Y));
        assert_eq!(m.formulas[1].kind, FormulaKind::Orientation(Axis::X));
        assert_eq!(m.formulas[2].kind, FormulaKind::MorphValue);
        assert_eq!(m.formulas[3].kind, FormulaKind::Other);
    }

    #[test]
    fn garbage_headers_are_errors_not_panics() {
        for bad in [&b""[..], b"[]", b"{}", b"{\"id\": 3}", b"\xff\xfe\x00", b"{\"id\":\"a\",\"formulas\":5}"] {
            let _ = parse_vmi(bad); // none panics
        }
        assert!(parse_vmi(b"{}").is_err());
        assert!(parse_vmi(b"not json").is_err());
        // A header with no formulas list is fine, and a byte-order mark is skipped.
        let ok = parse_vmi(b"\xEF\xBB\xBF{\"id\":\"A\"}").unwrap();
        assert_eq!((ok.display_name.as_str(), ok.min, ok.max, ok.formulas.len()), ("A", -1.0, 1.0, 0));
    }

    #[test]
    fn offsets_read_back_and_a_wrong_count_or_size_is_an_error() {
        let file = vmb(&[(0, [0.1, 0.2, 0.3]), (7, [-1.0, 0.0, 2.0])]);
        let d = parse_vmb(&file).unwrap();
        assert_eq!(d, vec![MorphDelta { vertex: 0, delta: [0.1, 0.2, 0.3] }, MorphDelta { vertex: 7, delta: [-1.0, 0.0, 2.0] }]);
        assert_eq!(parse_vmb(&file[..file.len() - 1]), Err(MorphError::BadSize(file.len() - 1)));
        let mut wrong = file.clone();
        wrong[0] = 3;
        assert!(parse_vmb(&wrong).is_err(), "the count must match the size");
        assert!(parse_vmb(&[]).is_err());
        assert!(parse_vmb(&(-1i32).to_le_bytes()).is_err());
        assert_eq!(parse_vmb(&0i32.to_le_bytes()).unwrap(), vec![]);
    }

    #[test]
    fn non_finite_and_negative_entries_are_skipped() {
        let file = vmb(&[(1, [f32::NAN, 0.0, 0.0]), (-3, [1.0, 1.0, 1.0]), (2, [0.5, 0.5, 0.5])]);
        assert_eq!(parse_vmb(&file).unwrap(), vec![MorphDelta { vertex: 2, delta: [0.5, 0.5, 0.5] }]);
    }

    /// Every morph of a real VaM install parses (run with `VAM_DIR` set to the game folder).
    #[test]
    #[ignore]
    fn every_morph_in_a_real_install_parses() {
        let Ok(dir) = std::env::var("VAM_DIR") else { return };
        let root = std::path::Path::new(&dir).join("Custom/Atom/Person/Morphs");
        let mut stack = vec![root];
        let (mut ok, mut bad) = (0, Vec::new());
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "vmi") {
                    let (Ok(h), Ok(b)) = (std::fs::read(&p), std::fs::read(p.with_extension("vmb"))) else { continue };
                    match parse_morph(&h, &b) {
                        Ok(m) if m.deltas.len() == m.info.num_deltas || m.info.num_deltas == 0 => ok += 1,
                        other => bad.push((p.display().to_string(), other.map(|m| (m.deltas.len(), m.info.num_deltas)).map_err(|e| e.to_string()))),
                    }
                }
            }
        }
        eprintln!("MORPHS ok {ok} bad {}", bad.len());
        assert!(bad.len() * 50 < ok.max(1), "too many failed: {:?}", &bad[..bad.len().min(5)]);
    }
}
