//! The mesh of a VaM clothing or hair item (`.vab`): the game's own `DAZMesh` binary store, a
//! .NET `BinaryWriter` stream (7-bit length-prefixed strings, little-endian numbers). Files are
//! hostile input: every count is capped and read with bounds checks.

use crate::VamError;

/// Most vertices, polygons and materials one item may have.
const MAX_VERTICES: usize = 2_000_000;
const MAX_POLYS: usize = 4_000_000;
const MAX_MATERIALS: usize = 256;

/// One polygon (a triangle or a quad) with its material.
#[derive(Clone, Debug, PartialEq)]
pub struct Poly {
    pub material: u32,
    pub vertices: Vec<u32>,
}

/// How one vertex of an item is tied to the body it is worn on (the game's `DAZSkinWrapStore`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct WrapVert {
    /// The three body vertices of the nearest triangle, nearest first.
    pub vertices: [u32; 3],
    /// Distance along the triangle's normal, along its first tangent (towards the centre) and along
    /// the second tangent.
    pub normal: f32,
    pub tangent1: f32,
    pub tangent2: f32,
    /// The same for the vertex's own normal, which makes a thick item follow its thickness setting.
    pub normal_dot: f32,
    pub tangent1_dot: f32,
    pub tangent2_dot: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DazMesh {
    /// How each UV vertex is tied to the body; empty when the file has no wrap data.
    pub wrap: Vec<WrapVert>,
    /// The base vertices, in the game's axes (z toward the viewer's left hand).
    pub base: Vec<[f32; 3]>,
    pub materials: Vec<String>,
    /// Polygons by base vertex.
    pub base_polys: Vec<Poly>,
    /// The same polygons by UV vertex (a vertex split along a texture seam is two UV vertices).
    pub uv_polys: Vec<Poly>,
    /// A texture coordinate per UV vertex.
    pub uvs: Vec<[f32; 2]>,
    /// `(from, to)`: UV vertex `to` copies UV vertex `from`.
    pub copies: Vec<(u32, u32)>,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], VamError> {
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| VamError::Morph("the mesh file is cut short".into()))?;
        let s = self.b.get(self.p..end).ok_or_else(|| VamError::Morph("the mesh file is cut short".into()))?;
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

    fn count(&mut self, max: usize) -> Result<usize, VamError> {
        let n = self.i32()?;
        usize::try_from(n).ok().filter(|&n| n <= max).ok_or_else(|| VamError::Morph(format!("the mesh file claims {n} entries")))
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
                return Err(VamError::Morph("bad string length in the mesh file".into()));
            }
        }
        if len > 4096 {
            return Err(VamError::Morph("a string in the mesh file is too long".into()));
        }
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    fn polys(&mut self, n: usize, vertex_limit: usize) -> Result<Vec<Poly>, VamError> {
        let mut out = Vec::with_capacity(n.min(65_536));
        for _ in 0..n {
            let material = u32::try_from(self.i32()?).unwrap_or(0);
            let k = self.count(8)?;
            if k < 3 {
                return Err(VamError::Morph("a polygon with fewer than three corners".into()));
            }
            let mut vertices = Vec::with_capacity(k);
            for _ in 0..k {
                let v = usize::try_from(self.i32()?).ok().filter(|&v| v < vertex_limit).ok_or_else(|| VamError::Morph("a polygon names a missing vertex".into()))?;
                vertices.push(v as u32);
            }
            out.push(Poly { material, vertices });
        }
        Ok(out)
    }
}

/// The `DAZSkinWrap` section after the mesh, when there is one.
fn parse_wrap(r: &mut Reader, uv_count: usize) -> Result<Vec<WrapVert>, VamError> {
    if r.string()? != "DAZSkinWrap" || r.string()? != "1.0" {
        return Ok(Vec::new());
    }
    let _name = r.string()?;
    if r.string()? != "DAZSkinWrapStore" || r.string()? != "1.0" {
        return Ok(Vec::new());
    }
    let n = r.count(MAX_VERTICES)?;
    if n != uv_count {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(n.min(65_536));
    for _ in 0..n {
        let _triangle = r.i32()?;
        let mut vertices = [0u32; 3];
        for v in &mut vertices {
            *v = u32::try_from(r.i32()?).unwrap_or(0);
        }
        out.push(WrapVert { vertices, normal: r.f32()?, tangent1: r.f32()?, tangent2: r.f32()?, normal_dot: r.f32()?, tangent1_dot: r.f32()?, tangent2_dot: r.f32()? });
    }
    Ok(out)
}

/// Reads a `.vab` item mesh.
pub fn parse_dazmesh(bytes: &[u8]) -> Result<DazMesh, VamError> {
    let mut r = Reader { b: bytes, p: 0 };
    // The file starts with "DynamicStore", its version, then the DAZMesh section and its version.
    if r.string()? != "DynamicStore" {
        return Err(VamError::Morph("not a VaM item mesh (no DynamicStore header)".into()));
    }
    let _version = r.string()?;
    if r.string()? != "DAZMesh" {
        return Err(VamError::Morph("the item mesh has no DAZMesh section".into()));
    }
    if r.string()? != "1.0" {
        return Err(VamError::Morph("the item mesh is of a version this reader does not know".into()));
    }
    for _ in 0..4 {
        r.string()?; // node and geometry ids
    }
    let nb = r.count(MAX_VERTICES)?;
    let mut base = Vec::with_capacity(nb.min(65_536));
    for _ in 0..nb {
        base.push([r.f32()?, r.f32()?, r.f32()?]);
    }
    let nm = r.count(MAX_MATERIALS)?;
    let materials = (0..nm).map(|_| r.string()).collect::<Result<Vec<_>, _>>()?;
    let np = r.count(MAX_POLYS)?;
    let base_polys = r.polys(np, nb)?;
    // The UV polygons come before the UV vertex count: bound them by the most a file may have.
    let uv_polys_raw = r.polys(np, MAX_VERTICES)?;
    let nuv = r.count(MAX_VERTICES)?;
    if uv_polys_raw.iter().any(|p| p.vertices.iter().any(|&v| v as usize >= nuv)) {
        return Err(VamError::Morph("a UV polygon names a missing vertex".into()));
    }
    let mut uvs = Vec::with_capacity(nuv.min(65_536));
    for _ in 0..nuv {
        uvs.push([r.f32()?, r.f32()?]);
    }
    let nc = r.count(MAX_VERTICES)?;
    let mut copies = Vec::with_capacity(nc.min(65_536));
    for _ in 0..nc {
        let (from, to, _poly) = (r.i32()?, r.i32()?, r.i32()?);
        let (Ok(from), Ok(to)) = (u32::try_from(from), u32::try_from(to)) else { continue };
        if (from as usize) < nuv && (to as usize) < nuv {
            copies.push((from, to));
        }
    }
    let wrap = parse_wrap(&mut r, nuv).unwrap_or_default();
    Ok(DazMesh { wrap, base, materials, base_polys, uv_polys: uv_polys_raw, uvs, copies })
}

impl DazMesh {
    /// The base vertex each UV vertex stands for.
    pub fn uv_base(&self) -> Vec<u32> {
        let mut out = vec![u32::MAX; self.uvs.len()];
        for (i, slot) in out.iter_mut().enumerate().take(self.base.len()) {
            *slot = i as u32;
        }
        // A copy takes the base vertex of the one it copies (copies may chain, in file order).
        for &(from, to) in &self.copies {
            let b = out.get(from as usize).copied().unwrap_or(u32::MAX);
            if let Some(slot) = out.get_mut(to as usize) {
                *slot = b;
            }
        }
        out
    }

    /// The position of each UV vertex: the base vertices first, then each copy of one it duplicates.
    pub fn uv_positions(&self) -> Vec<[f32; 3]> {
        let mut out = vec![[0.0f32; 3]; self.uvs.len()];
        for (slot, p) in out.iter_mut().zip(&self.base) {
            *slot = *p;
        }
        for &(from, to) in &self.copies {
            let p = out.get(from as usize).copied().unwrap_or([0.0; 3]);
            if let Some(slot) = out.get_mut(to as usize) {
                *slot = p;
            }
        }
        out
    }

    /// The triangles of each material as UV-vertex indices, quads split in two.
    pub fn triangles_by_material(&self) -> Vec<Vec<u32>> {
        let mut out = vec![Vec::new(); self.materials.len().max(1)];
        for p in &self.uv_polys {
            let Some(list) = out.get_mut(p.material as usize) else { continue };
            let v = &p.vertices;
            list.extend([v[0], v[1], v[2]]);
            if let Some(&d) = v.get(3) {
                list.extend([v[0], v[2], d]);
            }
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn put_string(out: &mut Vec<u8>, s: &str) {
        out.push(s.len() as u8);
        out.extend(s.as_bytes());
    }

    /// A one-quad, two-material item: four base vertices, one seam copy (UV vertex 4 copies 0).
    pub(crate) fn quad_item() -> Vec<u8> {
        let mut o = Vec::new();
        for s in ["DynamicStore", "1.0", "DAZMesh", "1.0", "n", "sn", "g", "sg"] {
            put_string(&mut o, s);
        }
        o.extend(4i32.to_le_bytes());
        for v in [[0.0f32, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 2.0, 0.0]] {
            for c in v {
                o.extend(c.to_le_bytes());
            }
        }
        o.extend(2i32.to_le_bytes());
        put_string(&mut o, "Cloth");
        put_string(&mut o, "Trim");
        // Base polygons: a quad on material 1 ...
        o.extend(2i32.to_le_bytes());
        for (m, vs) in [(1i32, vec![0i32, 1, 2, 3]), (0, vec![0, 1, 2])] {
            o.extend(m.to_le_bytes());
            o.extend((vs.len() as i32).to_le_bytes());
            for v in vs {
                o.extend(v.to_le_bytes());
            }
        }
        // ... the same by UV vertex (the triangle uses the seam copy 4).
        for (m, vs) in [(1i32, vec![0i32, 1, 2, 3]), (0, vec![4, 1, 2])] {
            o.extend(m.to_le_bytes());
            o.extend((vs.len() as i32).to_le_bytes());
            for v in vs {
                o.extend(v.to_le_bytes());
            }
        }
        o.extend(5i32.to_le_bytes());
        for i in 0..5 {
            o.extend((i as f32 / 10.0).to_le_bytes());
            o.extend(0.5f32.to_le_bytes());
        }
        o.extend(1i32.to_le_bytes());
        for v in [0i32, 4, 0] {
            o.extend(v.to_le_bytes());
        }
        o
    }

    #[test]
    fn an_item_mesh_reads_with_its_seam_copies_and_splits_quads() {
        let m = parse_dazmesh(&quad_item()).unwrap();
        assert_eq!((m.base.len(), m.materials.clone(), m.uvs.len()), (4, vec!["Cloth".to_string(), "Trim".to_string()], 5));
        assert_eq!(m.uv_base(), vec![0, 1, 2, 3, 0]);
        assert_eq!(m.uv_positions()[4], m.uv_positions()[0], "a copy sits where its original does");
        let t = m.triangles_by_material();
        assert_eq!(t[0], vec![4, 1, 2]);
        assert_eq!(t[1], vec![0, 1, 2, 0, 2, 3], "a quad is two triangles");
    }

    #[test]
    fn damaged_item_meshes_are_errors_not_panics() {
        let good = quad_item();
        assert!(parse_dazmesh(&[]).is_err());
        assert!(parse_dazmesh(b"\x05hello").is_err());
        for cut in [10, 30, 60, 100, good.len() - 5] {
            assert!(parse_dazmesh(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut huge = good.clone();
        huge[39..43].copy_from_slice(&i32::MAX.to_le_bytes()); // vertex count
        assert!(parse_dazmesh(&huge).is_err());
        let mut bent = good;
        let n = bent.len();
        for b in &mut bent[n - 40..] {
            *b = 0xFF;
        }
        let _ = parse_dazmesh(&bent);
    }
}
