//! A zip archive read from disk without loading it whole: VaM's `.var` packages run to gigabytes,
//! and an index of what is inside needs only the central directory at the end. Entries are
//! inflated one at a time, with their size capped. ZIP64 and encrypted archives are refused.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::VamError;

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
/// Most entries one archive may list.
const MAX_ENTRIES: usize = 200_000;
/// Largest central directory read.
const MAX_DIRECTORY: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ZipEntry {
    pub name: String,
    method: u16,
    crc: u32,
    compressed: u64,
    pub size: u64,
    local_offset: u64,
}

#[derive(Debug)]
pub struct ZipFile {
    path: PathBuf,
    pub entries: Vec<ZipEntry>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o.checked_add(2)?).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn bad(path: &Path, what: &str) -> VamError {
    VamError::Zip(format!("{}: {what}", path.display()))
}

/// The CRC-32 zip uses (IEEE, reflected).
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    !data.iter().fold(!0u32, |c, &b| table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8))
}

impl ZipFile {
    /// Reads the central directory of the archive at `path`.
    pub fn open(path: &Path) -> Result<Self, VamError> {
        let mut f = File::open(path).map_err(|e| VamError::Io(path.display().to_string(), e.to_string()))?;
        let len = f.metadata().map_err(|e| VamError::Io(path.display().to_string(), e.to_string()))?.len();
        if len < 22 {
            return Err(bad(path, "too short for a zip archive"));
        }
        // The end record sits in the last 22 + 65535 bytes (a comment may follow it).
        let tail_len = len.min(22 + 65535);
        f.seek(SeekFrom::Start(len - tail_len)).map_err(|e| VamError::Io(path.display().to_string(), e.to_string()))?;
        let mut tail = vec![0u8; tail_len as usize];
        f.read_exact(&mut tail).map_err(|e| VamError::Io(path.display().to_string(), e.to_string()))?;
        let eocd = (0..=tail.len() - 22)
            .rev()
            .find(|&i| u32_at(&tail, i) == Some(EOCD_SIG) && u16_at(&tail, i + 20).is_some_and(|c| i + 22 + usize::from(c) == tail.len()))
            .ok_or_else(|| bad(path, "no end-of-directory record"))?;
        let count = usize::from(u16_at(&tail, eocd + 10).unwrap_or(0));
        let size = u64::from(u32_at(&tail, eocd + 12).unwrap_or(0));
        let offset = u64::from(u32_at(&tail, eocd + 16).unwrap_or(0));
        if count == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
            return Err(bad(path, "ZIP64 archives are not supported"));
        }
        if size > MAX_DIRECTORY || offset.checked_add(size).is_none_or(|end| end > len) || count > MAX_ENTRIES {
            return Err(bad(path, "central directory out of range"));
        }
        f.seek(SeekFrom::Start(offset)).map_err(|e| VamError::Io(path.display().to_string(), e.to_string()))?;
        let mut dir = vec![0u8; size as usize];
        f.read_exact(&mut dir).map_err(|e| VamError::Io(path.display().to_string(), e.to_string()))?;
        let mut entries = Vec::with_capacity(count);
        let mut p = 0usize;
        for _ in 0..count {
            if u32_at(&dir, p) != Some(CENTRAL_SIG) {
                return Err(bad(path, "bad central directory entry"));
            }
            let get16 = |o: usize| u16_at(&dir, p + o).ok_or_else(|| bad(path, "truncated directory"));
            let get32 = |o: usize| u32_at(&dir, p + o).ok_or_else(|| bad(path, "truncated directory"));
            let flags = get16(8)?;
            let method = get16(10)?;
            let (crc, compressed, size_u, local) = (get32(16)?, get32(20)?, get32(24)?, get32(42)?);
            let (nlen, elen, clen) = (usize::from(get16(28)?), usize::from(get16(30)?), usize::from(get16(32)?));
            let name_at = p + 46;
            let name = dir.get(name_at..name_at + nlen).ok_or_else(|| bad(path, "truncated name"))?;
            // Sizes and the offset that did not fit in 32 bits are in the ZIP64 extra field, in
            // this order, only for the fields that read all ones.
            let (mut compressed, mut size_u, mut local) = (u64::from(compressed), u64::from(size_u), u64::from(local));
            let extra = dir.get(name_at + nlen..name_at + nlen + elen).unwrap_or(&[]);
            let mut q = 0usize;
            while q + 4 <= extra.len() {
                let (id, len) = (u16_at(extra, q).unwrap_or(0), usize::from(u16_at(extra, q + 2).unwrap_or(0)));
                if id == 1 {
                    let mut at = q + 4;
                    let mut next = |wanted: bool, field: &mut u64| {
                        if wanted && let Some(b) = extra.get(at..at + 8) {
                            *field = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                            at += 8;
                        }
                    };
                    let (need_size, need_comp, need_local) = (size_u == 0xFFFF_FFFF, compressed == 0xFFFF_FFFF, local == 0xFFFF_FFFF);
                    next(need_size, &mut size_u);
                    next(need_comp, &mut compressed);
                    next(need_local, &mut local);
                }
                q += 4 + len;
            }
            if flags & 1 == 0 && !name.ends_with(b"/") {
                entries.push(ZipEntry { name: String::from_utf8_lossy(name).into_owned(), method, crc, compressed, size: size_u, local_offset: local });
            }
            p = name_at + nlen + elen + clen;
        }
        Ok(ZipFile { path: path.to_path_buf(), entries })
    }

    pub fn find(&self, name: &str) -> Option<&ZipEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// The entry's bytes, at most `max` of them, checked against the archive's CRC.
    pub fn read(&self, e: &ZipEntry, max: usize) -> Result<Vec<u8>, VamError> {
        if e.size > max as u64 {
            return Err(bad(&self.path, &format!("`{}` is {} bytes, over the limit of {max}", e.name, e.size)));
        }
        let io = |err: std::io::Error| VamError::Io(self.path.display().to_string(), err.to_string());
        let mut f = File::open(&self.path).map_err(io)?;
        f.seek(SeekFrom::Start(e.local_offset)).map_err(io)?;
        let mut head = [0u8; 30];
        f.read_exact(&mut head).map_err(io)?;
        if u32_at(&head, 0) != Some(LOCAL_SIG) {
            return Err(bad(&self.path, "bad local header"));
        }
        let skip = u64::from(u16_at(&head, 26).unwrap_or(0)) + u64::from(u16_at(&head, 28).unwrap_or(0));
        f.seek(SeekFrom::Current(skip as i64)).map_err(io)?;
        let mut raw = Vec::new();
        f.take(e.compressed.min(max as u64 * 2 + 1024)).read_to_end(&mut raw).map_err(io)?;
        let data = match e.method {
            0 => raw,
            8 => {
                let mut out = Vec::with_capacity((e.size as usize).min(raw.len().saturating_mul(1032)).min(64 << 20));
                flate2::read::DeflateDecoder::new(raw.as_slice()).take(max as u64 + 1).read_to_end(&mut out).map_err(io)?;
                out
            }
            m => return Err(bad(&self.path, &format!("compression method {m} is not supported"))),
        };
        if data.len() as u64 != e.size || crc32(&data) != e.crc {
            return Err(bad(&self.path, &format!("`{}` is damaged", e.name)));
        }
        Ok(data)
    }

    pub fn read_by_name(&self, name: &str, max: usize) -> Result<Vec<u8>, VamError> {
        let e = self.find(name).ok_or_else(|| bad(&self.path, &format!("no entry `{name}`")))?;
        self.read(e, max)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A zip with the given entries, stored (method 0) or deflated (method 8).
    pub(crate) fn make_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        use std::io::Write;
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in files {
            let (method, body) = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                (8u16, e.finish().unwrap())
            } else {
                (0u16, data.to_vec())
            };
            let crc = crc32(data);
            let offset = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20, 0, 0, 0]);
            out.extend(method.to_le_bytes());
            out.extend([0u8; 4]);
            out.extend(crc.to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend([0, 0]);
            out.extend(name.as_bytes());
            out.extend(&body);
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend(method.to_le_bytes());
            central.extend([0u8; 4]);
            central.extend(crc.to_le_bytes());
            central.extend((body.len() as u32).to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_offset.to_le_bytes());
        out.extend([0, 0]);
        out
    }

    pub(crate) fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("photocraft-vam-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn stored_and_deflated_entries_read_back_and_directories_are_skipped() {
        let body = b"hello hello hello hello hello".repeat(20);
        let zip = make_zip(&[("a/b.txt", b"plain", false), ("c.bin", &body, true), ("d/", b"", false)]);
        let p = temp_file("t1.var", &zip);
        let z = ZipFile::open(&p).unwrap();
        assert_eq!(z.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["a/b.txt", "c.bin"]);
        assert_eq!(z.read_by_name("a/b.txt", 100).unwrap(), b"plain");
        assert_eq!(z.read_by_name("c.bin", 10_000).unwrap(), body);
        assert!(z.read_by_name("c.bin", 10).is_err(), "over the size limit");
        assert!(z.read_by_name("nope", 10).is_err());
    }

    #[test]
    fn damaged_or_foreign_files_are_errors_not_panics() {
        let zip = make_zip(&[("x", b"data", false)]);
        for (i, name) in ["short.var", "junk.var", "cut.var", "flip.var"].iter().enumerate() {
            let bytes: Vec<u8> = match i {
                0 => vec![1, 2, 3],
                1 => vec![7; 5000],
                2 => zip[..zip.len() - 10].to_vec(),
                _ => {
                    let mut z = zip.clone();
                    z[35] ^= 0xFF; // a byte of the stored data
                    z
                }
            };
            let p = temp_file(name, &bytes);
            if let Ok(z) = ZipFile::open(&p) {
                let _ = z.read_by_name("x", 100); // a failed CRC is an error
            }
        }
        let p = temp_file("flip.var", &{
            let mut z = zip.clone();
            z[31] ^= 0xFF;
            z
        });
        let z = ZipFile::open(&p).unwrap();
        assert!(z.read_by_name("x", 100).is_err(), "the CRC catches a changed byte");
        assert!(ZipFile::open(Path::new("no-such-file.var")).is_err());
    }
}
