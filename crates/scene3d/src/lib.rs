//! 3D scene model for the pose editor: nodes (primitives, lights, empties) in a hierarchy, a
//! camera, and the `.pcscene` file format. Pure data and math: no GPU, no egui, no file I/O, so
//! it builds for wasm32 and the headless CLI. Rendering lives in `photocraft-render3d`.
//!
//! Every number that comes from a file or a command is treated as hostile: sizes are clamped,
//! hierarchies are depth-bounded and cycle-checked, and nothing here panics.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod gizmo;
mod hold;
mod ik;
mod joint;
mod link;
mod mesh;
mod model;
mod scene;

pub use glam;
pub use mesh::Mesh;
pub use gizmo::{Delta, Drag, Gizmo, GizmoMode, Handle, Ray, closest_on_line, ray_plane};
pub use ik::{IkResult, rotate_world, set_world_rotation, solve_ik, translate_to};
pub use hold::enforce_holds;
pub use joint::JointLimit;
pub use link::{Link, LinkSpace};
pub use photocraft_model3d as model3d;
pub use scene::{
    Asset, Camera, FORMAT_VERSION, Light, LightKind, MAX_DEPTH, MAX_FILE_BYTES, MAX_NODES, Material, ModelCache, ModelMesh, Node, NodeId, NodeKind, Pose,
    PoseBone, Primitive, RotationLimits, Scene, SceneError, Transform,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_models;

#[cfg(test)]
mod tests_real;

#[cfg(test)]
mod tests_gizmo;

#[cfg(test)]
mod tests_links;

#[cfg(test)]
mod tests_joint;

#[cfg(test)]
mod tests_vam;

#[cfg(test)]
mod tests_hold;
