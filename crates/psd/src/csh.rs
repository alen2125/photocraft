//! Photoshop custom shape files (`.csh`), the Shapes panel's Load / Replace / Save Shapes
//! (fork-local, see MY_CHANGES.md).
//!
//! Adobe publishes no layout for this format. This follows the structure described by public
//! reverse-engineering notes, and **has not been checked against files written by Photoshop**:
//! a 4-byte type `cush`, a `u32` version (2) and a `u32` count; then per shape a Unicode name (a
//! `u32` UTF-16 code-unit count that includes a trailing null, padded to 4 bytes), a `u32`
//! payload size and the payload, padded to 4 bytes. The payload is a `u32` version (3), the
//! shape's bounds as four `i32`s (top, left, bottom, right, pixels) and then 26-byte path records
//! (as in a PSD vector mask, see [`crate::path`]) whose coordinates are fractions of the bounds.
//! A payload header of 24 bytes (an extra `u32` of flags after the version) is accepted too.
//! All values are big-endian.
//!
//! Reading is lazy, for libraries of thousands of shapes: [`index`] reads only the names and where
//! each payload sits, and [`parse_shape`] decodes one payload when it is wanted.

use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt};
use crate::path::{PathData, PsdSubpath};

/// Most shapes read from one file.
pub const MAX_SHAPES: usize = 65_536;
/// Longest name read or written, in UTF-16 code units.
pub const MAX_NAME_UNITS: usize = 1024;
/// Largest payload read or written, in bytes (about 160 000 path records).
pub const MAX_SHAPE_BYTES: usize = 4 * 1024 * 1024;

const SIGNATURE: &[u8; 4] = b"cush";
const PAYLOAD_VERSION: u32 = 3;
/// Version, then top / left / bottom / right.
const HEADER: usize = 20;

/// Where one shape sits in a file: its name and its payload's byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CshEntry {
    /// The shape's name.
    pub name: String,
    /// Where its payload starts in the file.
    pub offset: usize,
    /// The payload's length in bytes.
    pub len: usize,
}

/// One decoded shape.
#[derive(Debug, Clone, PartialEq)]
pub struct CshShape {
    /// `[top, left, bottom, right]` in pixels; the path fractions are relative to it.
    pub bounds: [i32; 4],
    /// The path, as fractions of the bounds.
    pub subpaths: Vec<PsdSubpath>,
}

impl CshShape {
    /// Width and height of the bounds (1 when empty or inverted, so fractions stay finite).
    pub fn size(&self) -> (f64, f64) {
        let [top, left, bottom, right] = self.bounds;
        let span = |a: i32, b: i32| if b > a { f64::from(b) - f64::from(a) } else { 1.0 };
        (span(left, right), span(top, bottom))
    }
}

fn read_name(r: &mut Reader<'_>) -> Result<String> {
    let n = u64::from(r.u32()?);
    r.check_count(n, 2)?;
    let n = usize::try_from(n).map_err(|_| PsdError::LimitExceeded("shape name length"))?;
    let mut units = Vec::with_capacity(n.min(MAX_NAME_UNITS));
    for _ in 0..n {
        let u = r.u16()?;
        if units.len() < MAX_NAME_UNITS {
            units.push(u);
        }
    }
    while units.last() == Some(&0) {
        units.pop();
    }
    Ok(String::from_utf16_lossy(&units))
}

fn pad4(r: &mut Reader<'_>, from: usize) -> Result<()> {
    let used = r.pos().saturating_sub(from);
    r.skip((4 - used % 4) % 4)
}

/// Read the shape names and payload positions of a `.csh` file, without decoding any path.
/// Bad signatures, unknown versions, truncation and impossible counts or sizes are errors.
pub fn index(data: &[u8]) -> Result<Vec<CshEntry>> {
    let mut r = Reader::new(data);
    let sig = r.array::<4>()?;
    if &sig != SIGNATURE {
        return Err(PsdError::InvalidSignature { expected: "cush", found: sig });
    }
    let version = r.u32()?;
    if version != 2 {
        return Err(PsdError::UnsupportedVersion(u16::try_from(version).unwrap_or(u16::MAX)));
    }
    let count = u64::from(r.u32()?);
    // A shape takes at least a name count (4) and a payload size (4).
    r.check_count(count, 8)?;
    let count = usize::try_from(count).map_err(|_| PsdError::LimitExceeded("shape count"))?;
    if count > MAX_SHAPES {
        return Err(PsdError::LimitExceeded("too many shapes in a .csh file"));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let start = r.pos();
        let name = read_name(&mut r)?;
        pad4(&mut r, start)?;
        let len = usize::try_from(r.u32()?).map_err(|_| PsdError::LimitExceeded("shape size"))?;
        if len > MAX_SHAPE_BYTES {
            return Err(PsdError::LimitExceeded("a shape in a .csh file is too large"));
        }
        let offset = r.pos();
        r.skip(len)?;
        pad4(&mut r, offset)?;
        out.push(CshEntry { name, offset, len });
    }
    Ok(out)
}

/// Decode the shape an [`index`] entry points at. Errors for a range outside `data` or a payload
/// that is not a path.
pub fn parse_shape(data: &[u8], entry: &CshEntry) -> Result<CshShape> {
    let end = entry.offset.checked_add(entry.len).ok_or(PsdError::LimitExceeded("shape range"))?;
    let payload = data.get(entry.offset..end).ok_or_else(|| PsdError::invalid("shape payload outside the file"))?;
    let mut r = Reader::new(payload);
    let version = r.u32()?;
    if version != PAYLOAD_VERSION {
        return Err(PsdError::invalid(format!("unknown shape version {version}")));
    }
    let bounds = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
    let rest = r.peek_rest();
    // Records fill the rest of the payload; a 24-byte header leaves a multiple of 26 after 4 more bytes.
    let records = if rest.len().is_multiple_of(crate::path::RECORD_LEN) {
        rest
    } else {
        rest.get(4..).filter(|t| t.len().is_multiple_of(crate::path::RECORD_LEN)).unwrap_or(rest)
    };
    let subpaths = PathData::from_bytes(records)?.subpaths();
    if subpaths.is_empty() {
        return Err(PsdError::invalid("shape has no path"));
    }
    Ok(CshShape { bounds, subpaths })
}

/// Write a `.csh` file. Fails on more than [`MAX_SHAPES`] shapes or a shape over [`MAX_SHAPE_BYTES`].
pub fn write(shapes: &[(String, CshShape)]) -> Result<Vec<u8>> {
    if shapes.len() > MAX_SHAPES {
        return Err(PsdError::LimitExceeded("too many shapes for a .csh file"));
    }
    let mut out = Vec::with_capacity(12 + shapes.len() * 128);
    out.put(SIGNATURE);
    out.put_u32(2);
    out.put_u32(shapes.len() as u32);
    for (name, shape) in shapes {
        // A subpath's knot count is 16-bit; a longer one would be cut short silently.
        if shape.subpaths.iter().any(|s| s.knots.len() > usize::from(u16::MAX)) {
            return Err(PsdError::LimitExceeded("too many knots in a subpath for a .csh file"));
        }
        let records = PathData::from_subpaths(&shape.subpaths, false).to_bytes();
        let len = HEADER + records.len();
        if len > MAX_SHAPE_BYTES {
            return Err(PsdError::LimitExceeded("a shape is too large for a .csh file"));
        }
        let start = out.len();
        let units = crate::io::utf16_units(name, MAX_NAME_UNITS);
        out.put_u32(units.len() as u32 + 1);
        for u in units {
            out.put_u16(u);
        }
        out.put_u16(0);
        while !(out.len() - start).is_multiple_of(4) {
            out.put_u8(0);
        }
        out.put_u32(len as u32);
        let body = out.len();
        out.put_u32(PAYLOAD_VERSION);
        for b in shape.bounds {
            out.put_u32(b as u32);
        }
        out.put(&records);
        while !(out.len() - body).is_multiple_of(4) {
            out.put_u8(0);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
