//! Photoshop custom shape files (`.csh`), the Shapes panel's Load / Replace / Save Shapes
//! (fork-local, see MY_CHANGES.md).
//!
//! Adobe publishes no layout for this format. This one was worked out from real files (Photoshop
//! shape libraries of 100-200 MB) and checked against them: a 4-byte type `cush`, a `u32` version
//! (2) and a `u32` count; then per shape a Unicode name (a `u32` UTF-16 code-unit count that
//! includes the trailing null, then the units), zero padding up to a multiple of 4 bytes from the
//! start of the file, a `u32` version (1), a `u32` payload size and the payload.
//!
//! The payload is a Pascal string (a length byte and that many characters: the shape's GUID), the
//! shape's bounds as four `i32`s (top, left, bottom, right, pixels) and then 26-byte path records
//! (as in a PSD vector mask, see [`crate::path`]; the coordinates are fractions of the bounds)
//! up to the end of the payload, which may leave a few spare bytes. All values are big-endian.
//! Some files carry data after the last shape; it is ignored.
//!
//! Reading is lazy and works on a file without loading it: [`index_reader`] reads only the names
//! and where each payload sits, and [`parse_payload`] decodes one payload when it is wanted.

use std::io::{Cursor, Read, Seek, SeekFrom};

use crate::error::{PsdError, Result};
use crate::io::WriteExt;
use crate::path::{PathData, PsdSubpath};

/// Most shapes read from one file.
pub const MAX_SHAPES: usize = 65_536;
/// Longest name read or written, in UTF-16 code units.
pub const MAX_NAME_UNITS: usize = 1024;
/// Largest payload read or written, in bytes (real shapes of a traced drawing take about 2 MB).
pub const MAX_SHAPE_BYTES: usize = 16 * 1024 * 1024;

const SIGNATURE: &[u8; 4] = b"cush";
const SHAPE_VERSION: u32 = 1;

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

fn eof(offset: u64, needed: usize) -> PsdError {
    PsdError::UnexpectedEof { offset: usize::try_from(offset).unwrap_or(usize::MAX), needed }
}

fn io_err(e: std::io::Error, offset: u64, needed: usize) -> PsdError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof { eof(offset, needed) } else { PsdError::invalid(format!("could not read the file: {e}")) }
}

/// A reader that knows where it is and how long the file is, so every read is checked first.
struct Source<'a, R: Read + Seek> {
    r: &'a mut R,
    pos: u64,
    len: u64,
}

impl<R: Read + Seek> Source<'_, R> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        if self.len.saturating_sub(self.pos) < N as u64 {
            return Err(eof(self.pos, N));
        }
        let mut b = [0u8; N];
        self.r.read_exact(&mut b).map_err(|e| io_err(e, self.pos, N))?;
        self.pos += N as u64;
        Ok(b)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take::<4>()?))
    }

    /// Move forward `n` bytes, which must be inside the file.
    fn skip(&mut self, n: u64) -> Result<()> {
        let to = self.pos.checked_add(n).filter(|to| *to <= self.len).ok_or_else(|| eof(self.pos, usize::try_from(n).unwrap_or(usize::MAX)))?;
        self.r.seek(SeekFrom::Start(to)).map_err(|e| io_err(e, self.pos, 0))?;
        self.pos = to;
        Ok(())
    }

    fn name(&mut self) -> Result<String> {
        let n = u64::from(self.u32()?);
        // Every unit is 2 bytes of the file.
        if n.saturating_mul(2) > self.len.saturating_sub(self.pos) {
            return Err(eof(self.pos, usize::try_from(n.saturating_mul(2)).unwrap_or(usize::MAX)));
        }
        let kept = usize::try_from(n).map_or(MAX_NAME_UNITS, |n| n.min(MAX_NAME_UNITS));
        let mut units = Vec::with_capacity(kept);
        for _ in 0..kept {
            units.push(u16::from_be_bytes(self.take::<2>()?));
        }
        // The rest of an overlong name is read past, not kept.
        self.skip((n - kept as u64) * 2)?;
        while units.last() == Some(&0) {
            units.pop();
        }
        Ok(String::from_utf16_lossy(&units))
    }
}

/// Read the shape names and payload positions of a `.csh` file held in memory, decoding no path.
pub fn index(data: &[u8]) -> Result<Vec<CshEntry>> {
    index_reader(&mut Cursor::new(data))
}

/// Read the shape names and payload positions of a `.csh` file without reading the payloads: a
/// file of 200 MB costs a few small reads and seeks. Bad signatures, unknown versions, truncation
/// and impossible counts or sizes are errors.
pub fn index_reader<R: Read + Seek>(r: &mut R) -> Result<Vec<CshEntry>> {
    let len = r.seek(SeekFrom::End(0)).map_err(|e| io_err(e, 0, 0))?;
    r.seek(SeekFrom::Start(0)).map_err(|e| io_err(e, 0, 0))?;
    let mut s = Source { r, pos: 0, len };
    let sig = s.take::<4>()?;
    if &sig != SIGNATURE {
        return Err(PsdError::InvalidSignature { expected: "cush", found: sig });
    }
    let version = s.u32()?;
    if version != 2 {
        return Err(PsdError::UnsupportedVersion(u16::try_from(version).unwrap_or(u16::MAX)));
    }
    let count = usize::try_from(s.u32()?).map_err(|_| PsdError::LimitExceeded("shape count"))?;
    if count > MAX_SHAPES {
        return Err(PsdError::LimitExceeded("too many shapes in a .csh file"));
    }
    // A shape takes at least a name count (4), the version (1 + 3 padding) and a size (4).
    if (count as u64).saturating_mul(12) > s.len.saturating_sub(s.pos) {
        return Err(eof(s.pos, count.saturating_mul(12)));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let name = s.name()?;
        s.skip((4 - s.pos % 4) % 4)?;
        let _version = s.u32()?;
        let size = usize::try_from(s.u32()?).map_err(|_| PsdError::LimitExceeded("shape size"))?;
        if size > MAX_SHAPE_BYTES {
            return Err(PsdError::LimitExceeded("a shape in a .csh file is too large"));
        }
        let offset = usize::try_from(s.pos).map_err(|_| PsdError::LimitExceeded("file position"))?;
        s.skip(size as u64)?;
        out.push(CshEntry { name, offset, len: size });
    }
    Ok(out)
}

/// Decode the shape an [`index`] entry points at, from the whole file's bytes.
pub fn parse_shape(data: &[u8], entry: &CshEntry) -> Result<CshShape> {
    let end = entry.offset.checked_add(entry.len).ok_or(PsdError::LimitExceeded("shape range"))?;
    parse_payload(data.get(entry.offset..end).ok_or_else(|| PsdError::invalid("shape payload outside the file"))?)
}

/// Decode one shape's payload (the `len` bytes at an entry's `offset`).
pub fn parse_payload(payload: &[u8]) -> Result<CshShape> {
    let (&guid_len, rest) = payload.split_first().ok_or_else(|| eof(0, 1))?;
    let rest = rest.get(usize::from(guid_len)..).ok_or_else(|| eof(1, usize::from(guid_len)))?;
    let (bounds, records) = rest.split_at_checked(16).ok_or_else(|| eof(1 + u64::from(guid_len), 16))?;
    let int = |i: usize| bounds.get(i * 4..i * 4 + 4).and_then(|b| b.try_into().ok()).map(i32::from_be_bytes).unwrap_or(0);
    let bounds = [int(0), int(1), int(2), int(3)];
    // A partial record at the end (spare bytes) is ignored.
    let subpaths = PathData::from_bytes(records)?.subpaths();
    if subpaths.is_empty() {
        return Err(PsdError::invalid("shape has no path"));
    }
    Ok(CshShape { bounds, subpaths })
}

/// A stable GUID-shaped id for a shape written by this tool.
fn guid(name: &str, n: usize) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ n as u64;
    for b in name.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    let g = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", (h >> 32) as u32, (h >> 16) as u16, h as u16, (g >> 48) as u16, g & 0xffff_ffff_ffff)
}

/// Write a `.csh` file. Fails on more than [`MAX_SHAPES`] shapes, a shape over [`MAX_SHAPE_BYTES`]
/// or a subpath of over 65 535 knots (the record's count is 16-bit).
pub fn write(shapes: &[(String, CshShape)]) -> Result<Vec<u8>> {
    if shapes.len() > MAX_SHAPES {
        return Err(PsdError::LimitExceeded("too many shapes for a .csh file"));
    }
    let mut out = Vec::with_capacity(12 + shapes.len() * 128);
    out.put(SIGNATURE);
    out.put_u32(2);
    out.put_u32(shapes.len() as u32);
    for (n, (name, shape)) in shapes.iter().enumerate() {
        if shape.subpaths.iter().any(|s| s.knots.len() > usize::from(u16::MAX)) {
            return Err(PsdError::LimitExceeded("too many knots in a subpath for a .csh file"));
        }
        let records = PathData::from_subpaths(&shape.subpaths, false).to_bytes();
        let id = guid(name, n);
        let mut len = 1 + id.len() + 16 + records.len();
        len += (4 - len % 4) % 4;
        if len > MAX_SHAPE_BYTES {
            return Err(PsdError::LimitExceeded("a shape is too large for a .csh file"));
        }
        let units = crate::io::utf16_units(name, MAX_NAME_UNITS);
        out.put_u32(units.len() as u32 + 1);
        for u in units {
            out.put_u16(u);
        }
        out.put_u16(0);
        while !out.len().is_multiple_of(4) {
            out.put_u8(0);
        }
        out.put_u32(SHAPE_VERSION);
        out.put_u32(len as u32);
        let body = out.len();
        out.put_u8(id.len() as u8);
        out.put(id.as_bytes());
        for b in shape.bounds {
            out.put_u32(b as u32);
        }
        out.put(&records);
        while out.len() - body < len {
            out.put_u8(0);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
