//! Strand hair of a VaM hair item (`.vab`): a `RuntimeHairGeometryCreator` section of the game's
//! binary store. It holds the guide strands of the style: for each scalp vertex a list of points,
//! in the figure's axes. The rest of the section (connections, rigidities) is for the game's
//! physics and is not read. Files are hostile input: every count is capped.

use crate::VamError;

/// Most strands and most points in all one item may have.
const MAX_STRANDS: usize = 200_000;
const MAX_POINTS: usize = 8_000_000;

/// The guide strands of a hair item.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HairStrands {
    /// The name of the scalp the roots sit on (`LeytonScalp`).
    pub scalp: String,
    /// Points per strand as the style was made (21 or 50).
    pub segments: usize,
    pub segment_length: f32,
    /// The points of each strand, root first, in the game's axes. Strands with no points are left out.
    pub strands: Vec<Vec<[f32; 3]>>,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], VamError> {
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| VamError::Morph("the hair file is cut short".into()))?;
        let s = self.b.get(self.p..end).ok_or_else(|| VamError::Morph("the hair file is cut short".into()))?;
        self.p = end;
        Ok(s)
    }

    fn i32(&mut self) -> Result<i32, VamError> {
        let s = self.take(4)?;
        Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn f32(&mut self) -> Result<f32, VamError> {
        let s = self.take(4)?;
        let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        Ok(if v.is_finite() { v } else { 0.0 })
    }

    fn string(&mut self) -> Result<String, VamError> {
        let (mut len, mut shift) = (0usize, 0u32);
        loop {
            let c = self.take(1)?[0];
            len |= usize::from(c & 0x7f) << shift;
            shift += 7;
            if c & 0x80 == 0 {
                break;
            }
            if shift > 28 {
                return Err(VamError::Morph("bad string length in the hair file".into()));
            }
        }
        if len > 4096 {
            return Err(VamError::Morph("a string in the hair file is too long".into()));
        }
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
}

/// Reads the strands of a hair `.vab`.
pub fn parse_hair(bytes: &[u8]) -> Result<HairStrands, VamError> {
    let mut r = Reader { b: bytes, p: 0 };
    if r.string()? != "DynamicStore" {
        return Err(VamError::Morph("not a VaM hair file (no DynamicStore header)".into()));
    }
    let _version = r.string()?;
    let _sections = r.take(1)?;
    if r.string()? != "RuntimeHairGeometryCreator" {
        return Err(VamError::Morph("the file has no hair strands".into()));
    }
    let version = r.string()?;
    if version != "1.0" && version != "1.1" {
        return Err(VamError::Morph(format!("hair data of version {version} is not known")));
    }
    let scalp = r.string()?;
    let segments = usize::try_from(r.i32()?).ok().filter(|&s| (2..=1000).contains(&s)).ok_or_else(|| VamError::Morph("the hair has a strange number of points per strand".into()))?;
    let segment_length = r.f32()?;
    // The mask of scalp vertices: a name, a count and one byte each.
    let _mask_name = r.string()?;
    let mask = usize::try_from(r.i32()?).ok().filter(|&m| m <= 4_000_000).ok_or_else(|| VamError::Morph("bad hair mask".into()))?;
    r.take(mask)?;
    let n = usize::try_from(r.i32()?).ok().filter(|&n| n <= MAX_STRANDS).ok_or_else(|| VamError::Morph("the hair claims too many strands".into()))?;
    let mut strands = Vec::with_capacity(n.min(4096));
    let mut total = 0usize;
    for _ in 0..n {
        let _scalp_vertex = r.i32()?;
        let count = usize::try_from(r.i32()?).ok().filter(|&c| c <= 10_000).ok_or_else(|| VamError::Morph("a strand claims too many points".into()))?;
        total += count;
        if total > MAX_POINTS {
            return Err(VamError::Morph("the hair has too many points".into()));
        }
        if count == 0 {
            continue;
        }
        let mut points = Vec::with_capacity(count);
        for _ in 0..count {
            points.push([r.f32()?, r.f32()?, r.f32()?]);
        }
        strands.push(points);
    }
    Ok(HairStrands { scalp, segments, segment_length, strands })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dazmesh::tests::put_string;

    /// Three scalp vertices, the middle one without hair; two strands of three points.
    pub(crate) fn hair_file() -> Vec<u8> {
        hair_file_with(3)
    }

    /// The same with another number written as the strand count.
    fn hair_file_with(claimed: i32) -> Vec<u8> {
        let mut o = Vec::new();
        put_string(&mut o, "DynamicStore");
        put_string(&mut o, "1.0");
        o.push(1);
        put_string(&mut o, "RuntimeHairGeometryCreator");
        put_string(&mut o, "1.1");
        put_string(&mut o, "TestScalp");
        o.extend(3i32.to_le_bytes());
        o.extend(0.02f32.to_le_bytes());
        put_string(&mut o, "");
        o.extend(3i32.to_le_bytes());
        o.extend([1u8, 0, 1]);
        o.extend(claimed.to_le_bytes());
        for (idx, pts) in [(0i32, vec![[0.0f32, 1.7, -0.1], [0.0, 1.69, -0.12], [0.0, 1.66, -0.13]]), (1, vec![]), (2, vec![[0.05, 1.7, -0.1], [0.05, 1.68, -0.11], [0.05, 1.65, -0.12]])] {
            o.extend(idx.to_le_bytes());
            o.extend((pts.len() as i32).to_le_bytes());
            for p in pts {
                for c in p {
                    o.extend(c.to_le_bytes());
                }
            }
        }
        o
    }

    #[test]
    fn strands_are_read_and_empty_ones_left_out() {
        let h = parse_hair(&hair_file()).unwrap();
        assert_eq!((h.scalp.as_str(), h.segments, h.strands.len()), ("TestScalp", 3, 2));
        assert_eq!(h.strands[1][0], [0.05, 1.7, -0.1]);
        assert!((h.segment_length - 0.02).abs() < 1e-6);
    }

    #[test]
    fn damaged_hair_is_an_error_not_a_panic() {
        let good = hair_file();
        assert!(parse_hair(&[]).is_err());
        assert!(parse_hair(b"\x05hello").is_err());
        for cut in [5, 20, 60, 90, good.len() - 4] {
            assert!(parse_hair(&good[..cut]).is_err(), "cut at {cut}");
        }
        assert!(parse_hair(&hair_file_with(i32::MAX)).is_err(), "an enormous strand count");
        assert!(parse_hair(&hair_file_with(-5)).is_err());
        // A dazmesh item is not hair.
        assert!(parse_hair(&crate::dazmesh::tests::quad_item()).is_err());
    }
}
