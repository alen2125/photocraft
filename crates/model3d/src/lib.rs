//! Reader for 3D character and prop files: glTF 2.0 (`.glb`, self-contained `.gltf`) and VRM
//! (0.x and 1.0, which are glTF with extensions). Produces plain data ([`ModelData`]): the node
//! hierarchy, skinned meshes, materials and decoded textures, the VRM humanoid bone map and the
//! VRM spring-bone (hair, skirt) chains. No GPU, no scene logic, no workspace dependencies.
//!
//! Files are hostile input: sizes and counts are capped, bad indices are errors, nothing here
//! panics.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod read;
#[cfg(any(test, feature = "testgen"))]
pub mod testgen;
mod vrm;

pub use read::load;

/// Largest model file accepted.
pub const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;
/// Most nodes, mesh vertices and triangles a model may hold.
pub const MAX_NODES: usize = 4096;
pub const MAX_VERTICES: usize = 4_000_000;
/// A texture edge above this is dropped (the material falls back to its flat colour).
pub const MAX_TEXTURE_EDGE: u32 = 4096;
/// Decoded texture bytes kept in total.
pub const MAX_TEXTURE_BYTES: usize = 768 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ModelError {
    #[error("file is {0} bytes, over the {MAX_FILE_BYTES}-byte limit")]
    TooLarge(usize),
    #[error("not a readable glTF/GLB/VRM file: {0}")]
    Parse(String),
    #[error("model has {0} nodes, over the limit of {MAX_NODES}")]
    TooManyNodes(usize),
    #[error("model has more than {MAX_VERTICES} vertices")]
    TooManyVertices,
    #[error("the model contains no triangle meshes")]
    NoMeshes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelFormat {
    Gltf,
    Vrm0,
    Vrm1,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelNode {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub translation: [f32; 3],
    /// Unit quaternion, `[x, y, z, w]`.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// One glTF primitive as a triangle mesh.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MeshData {
    pub name: String,
    /// The node that carries the mesh.
    pub node: usize,
    /// Index into [`ModelData::skins`] when the mesh is skinned.
    pub skin: Option<usize>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Per-vertex joint indices into the skin's joint list (empty when unskinned).
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkinData {
    /// Node index of each joint, in the order mesh vertices refer to them.
    pub joints: Vec<usize>,
    /// Column-major inverse bind matrix per joint.
    pub inverse_bind: Vec<[[f32; 4]; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    /// Pixels below the cutoff are discarded.
    Mask,
    /// Treated like `Mask` at 0.5 (the renderer does not sort).
    Blend,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialData {
    pub name: String,
    pub base_color: [f32; 4],
    /// Index into [`ModelData::textures`].
    pub base_color_texture: Option<usize>,
    pub emissive: [f32; 3],
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    /// The file asked for `KHR_materials_unlit` (VRM exporters set it on everything).
    pub unlit: bool,
}

/// Decoded sRGB RGBA8 image, row-major from the top-left.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A VRM spring-bone chain (hair, skirt, accessories): the roots whose descendants sway.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SpringGroup {
    pub comment: String,
    pub stiffness: f32,
    pub gravity_power: f32,
    pub gravity_dir: [f32; 3],
    pub drag_force: f32,
    pub hit_radius: f32,
    pub roots: Vec<usize>,
    pub collider_groups: Vec<usize>,
}

/// Spheres (offset in the node's space, radius) the spring bones must not enter.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ColliderGroup {
    pub node: usize,
    pub spheres: Vec<([f32; 3], f32)>,
}

/// Swing-twist limits of one joint node (the file's `PC_limits` extension; degrees).
#[derive(Clone, Debug, PartialEq)]
pub struct JointLimitData {
    pub node: usize,
    /// The joint frame as a quaternion `[x, y, z, w]` in the node's rest axes.
    pub frame: [f32; 4],
    pub twist: [f32; 2],
    pub swing: [f32; 2],
}

/// A joint some slider turns: its own axes (`frame`, a quaternion `[x, y, z, w]` in the file's
/// space) and the order its three turns are applied in.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlBoneData {
    pub node: usize,
    pub frame: [f32; 4],
    pub order: u8,
}

/// How far a slider at 1 turns one joint, in degrees about each of the joint's own axes.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlRot {
    pub node: usize,
    pub deg: [f32; 3],
}

/// A pose slider (the file's `PC_pose_controls` extension): joint turns, plus links that add the
/// slider's value to other sliders.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PoseControlData {
    pub name: String,
    pub label: String,
    pub group: String,
    pub min: f32,
    pub max: f32,
    pub rot: Vec<ControlRot>,
    pub links: Vec<(String, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelData {
    pub name: String,
    pub format: ModelFormat,
    pub nodes: Vec<ModelNode>,
    pub roots: Vec<usize>,
    pub meshes: Vec<MeshData>,
    pub skins: Vec<SkinData>,
    pub materials: Vec<MaterialData>,
    pub textures: Vec<Option<TextureData>>,
    /// VRM humanoid bone name (`hips`, `leftUpperArm`, …) to node index.
    pub humanoid: Vec<(String, usize)>,
    pub springs: Vec<SpringGroup>,
    pub collider_groups: Vec<ColliderGroup>,
    /// Nodes meant to be moved by hand (the file's `PC_controls` extension). The other joints
    /// (fingers, face, tongue…) are driven by sliders and poses; empty means every joint counts.
    pub controls: Vec<usize>,
    pub joint_limits: Vec<JointLimitData>,
    pub control_bones: Vec<ControlBoneData>,
    pub pose_controls: Vec<PoseControlData>,
}

impl ModelData {
    /// Node index of a humanoid bone, by its VRM name.
    pub fn humanoid_node(&self, bone: &str) -> Option<usize> {
        self.humanoid.iter().find(|(n, _)| n == bone).map(|(_, i)| *i)
    }

    /// Whether any skin uses node `i` as a joint.
    pub fn is_joint(&self, i: usize) -> bool {
        self.skins.iter().any(|s| s.joints.contains(&i))
    }
}

#[cfg(test)]
mod tests;
