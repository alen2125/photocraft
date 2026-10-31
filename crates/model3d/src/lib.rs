//! Reader for 3D character and prop files: glTF 2.0 (`.glb`, self-contained `.gltf`) and VRM
//! (0.x and 1.0, which are glTF with extensions). Produces plain data ([`ModelData`]): the node
//! hierarchy, skinned meshes, materials and decoded textures, the VRM humanoid bone map and the
//! VRM spring-bone (hair, skirt) chains. No GPU, no scene logic, no workspace dependencies.
//!
//! Files are hostile input: sizes and counts are capped, bad indices are errors, nothing here
//! panics.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod morph;
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
    /// A tangent-space normal map (index into [`ModelData::textures`]), green up.
    pub normal_texture: Option<usize>,
    /// How strongly the normal map bends the surface (1 = as drawn).
    pub normal_scale: f32,
    /// The same for the highlight (VaM has one number for the lit surface and one for the shine).
    pub spec_normal_scale: f32,
    /// A picture whose red channel scales the highlight (a specular or gloss map).
    pub specular_texture: Option<usize>,
    /// Highlight strength and sharpness; `None` leaves the renderer's own values.
    pub specular: Option<f32>,
    pub shininess: Option<f32>,
}

/// Decoded sRGB RGBA8 image, row-major from the top-left.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    /// Shared, so copying a model (a morph rebuilds it) does not copy its pictures.
    pub rgba: std::sync::Arc<Vec<u8>>,
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

/// How hard a drive pulls: spring (per unit of error), damper (per unit of speed), strongest pull.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveData {
    pub spring: f32,
    pub damper: f32,
    pub max_force: f32,
}

/// The physical makeup of one body part, as its model file gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyData {
    pub node: usize,
    pub mass: f32,
    pub drag: f32,
    pub angular_drag: f32,
    /// The muscle holding the joint to the body part above it at its rest pose, if it has one.
    pub muscle: Option<DriveData>,
}

/// A capsule on a body part: the end points (in the bone's own axes, from the bone's origin)
/// and the radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColliderData {
    pub node: usize,
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub radius: f32,
}

/// What the physics runs on (the file's `PC_physics` extension). `None` fields keep the editor's
/// own defaults.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PhysicsData {
    /// Seconds per physics step.
    pub step: Option<f32>,
    pub iterations: Option<u32>,
    pub friction: Option<f32>,
    pub bounciness: Option<f32>,
    /// A body part never moves faster than this (m/s).
    pub max_velocity: Option<f32>,
    /// The drive of a controller that is On (it holds the body part where it was, by spring).
    pub hold_position: Option<DriveData>,
    pub hold_rotation: Option<DriveData>,
    pub bodies: Vec<BodyData>,
    pub colliders: Vec<ColliderData>,
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
    pub physics: PhysicsData,
    /// For each vertex of the meshes (they share one vertex list), the base vertex of the figure it
    /// copies: a morph's offsets are given per base vertex, and a seam copy takes its twin's. Empty
    /// when the model has no morph data (the file's `PC_morph` extension).
    pub vertex_base: Vec<u32>,
    /// The base vertex where the genital graft starts: the genital morphs number their vertices
    /// from 0 within the graft, so a morph's index plus this is the figure's base vertex.
    pub graft_start: Option<u32>,
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
