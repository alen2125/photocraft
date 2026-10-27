//! Triangle meshes for the parametric primitives.

use crate::scene::Primitive;

/// Indexed triangle list with per-vertex normals (flat shading is done by duplicating vertices).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

const MIN_SIZE: f32 = 1e-4;
const MAX_SIZE: f32 = 1e6;

fn size(v: f32) -> f32 {
    if v.is_finite() { v.abs().clamp(MIN_SIZE, MAX_SIZE) } else { 1.0 }
}

impl Mesh {
    fn quad(&mut self, p: [[f32; 3]; 4], n: [f32; 3]) {
        let base = self.positions.len() as u32;
        for v in p {
            self.positions.push(v);
            self.normals.push(n);
        }
        self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Ring of `segments` vertices at height `y`, radius `r`, with the given normal slope `ny`.
    fn ring(&mut self, segments: u32, r: f32, y: f32, ny: f32) {
        for i in 0..=segments {
            let a = i as f32 / segments as f32 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            let len = (1.0 + ny * ny).sqrt();
            self.positions.push([c * r, y, s * r]);
            self.normals.push([c / len, ny / len, s / len]);
        }
    }

    /// Joins two rings of `segments + 1` vertices that start at `a` and `b`.
    fn bridge(&mut self, a: u32, b: u32, segments: u32) {
        for i in 0..segments {
            self.indices.extend([a + i, b + i, a + i + 1, a + i + 1, b + i, b + i + 1]);
        }
    }

    fn cap(&mut self, segments: u32, r: f32, y: f32, up: bool) {
        let centre = self.positions.len() as u32;
        let ny = if up { 1.0 } else { -1.0 };
        self.positions.push([0.0, y, 0.0]);
        self.normals.push([0.0, ny, 0.0]);
        let ring = centre + 1;
        for i in 0..=segments {
            let a = i as f32 / segments as f32 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            self.positions.push([c * r, y, s * r]);
            self.normals.push([0.0, ny, 0.0]);
        }
        for i in 0..segments {
            if up {
                self.indices.extend([centre, ring + i + 1, ring + i]);
            } else {
                self.indices.extend([centre, ring + i, ring + i + 1]);
            }
        }
    }
}

impl Primitive {
    /// The mesh for this primitive, centred on the origin. Sizes and segment counts are clamped.
    pub fn mesh(&self) -> Mesh {
        let mut m = Mesh::default();
        match *self {
            Primitive::Cube { size: s } => {
                let (x, y, z) = (size(s[0]) / 2.0, size(s[1]) / 2.0, size(s[2]) / 2.0);
                m.quad([[x, -y, -z], [x, y, -z], [x, y, z], [x, -y, z]], [1.0, 0.0, 0.0]);
                m.quad([[-x, -y, z], [-x, y, z], [-x, y, -z], [-x, -y, -z]], [-1.0, 0.0, 0.0]);
                m.quad([[-x, y, -z], [-x, y, z], [x, y, z], [x, y, -z]], [0.0, 1.0, 0.0]);
                m.quad([[-x, -y, z], [-x, -y, -z], [x, -y, -z], [x, -y, z]], [0.0, -1.0, 0.0]);
                m.quad([[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]], [0.0, 0.0, 1.0]);
                m.quad([[x, -y, -z], [-x, -y, -z], [-x, y, -z], [x, y, -z]], [0.0, 0.0, -1.0]);
            }
            Primitive::Sphere { radius, segments } => {
                let r = size(radius);
                let seg = segments.clamp(4, 128);
                let rings = (seg / 2).max(2);
                for j in 0..=rings {
                    let v = j as f32 / rings as f32 * std::f32::consts::PI;
                    let (sv, cv) = v.sin_cos();
                    for i in 0..=seg {
                        let u = i as f32 / seg as f32 * std::f32::consts::TAU;
                        let (su, cu) = u.sin_cos();
                        let n = [sv * cu, cv, sv * su];
                        m.positions.push([n[0] * r, n[1] * r, n[2] * r]);
                        m.normals.push(n);
                    }
                }
                let stride = seg + 1;
                for j in 0..rings {
                    for i in 0..seg {
                        let a = j * stride + i;
                        let b = a + stride;
                        m.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
                    }
                }
            }
            Primitive::Cylinder { radius, height, segments } => {
                let (r, h) = (size(radius), size(height) / 2.0);
                let seg = segments.clamp(3, 128);
                m.ring(seg, r, h, 0.0);
                m.ring(seg, r, -h, 0.0);
                m.bridge(0, seg + 1, seg);
                m.cap(seg, r, h, true);
                m.cap(seg, r, -h, false);
            }
            Primitive::Cone { radius, height, segments } => {
                let (r, h) = (size(radius), size(height));
                let seg = segments.clamp(3, 128);
                // Side normals lean up by r/h.
                let slope = r / h;
                m.ring(seg, 0.0, h / 2.0, slope);
                m.ring(seg, r, -h / 2.0, slope);
                m.bridge(0, seg + 1, seg);
                m.cap(seg, r, -h / 2.0, false);
            }
            Primitive::Plane { size: s } => {
                let (x, z) = (size(s[0]) / 2.0, size(s[1]) / 2.0);
                m.quad([[-x, 0.0, z], [x, 0.0, z], [x, 0.0, -z], [-x, 0.0, -z]], [0.0, 1.0, 0.0]);
            }
        }
        m
    }
}
