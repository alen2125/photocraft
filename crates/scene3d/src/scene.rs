//! Scene graph types and the `.pcscene` (JSON) file format.

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// `.pcscene` format version. A newer file is refused; older ones load through `serde(default)`.
pub const FORMAT_VERSION: u32 = 1;
/// Most nodes a scene may hold (bounds work and memory for hostile files).
pub const MAX_NODES: usize = 4096;
/// Deepest parent chain walked when resolving world transforms.
pub const MAX_DEPTH: usize = 64;
/// Largest `.pcscene` text accepted.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

pub type NodeId = u32;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SceneError {
    #[error("scene has too many nodes (limit {MAX_NODES})")]
    TooManyNodes,
    #[error("no node with id {0}")]
    NoSuchNode(NodeId),
    #[error("that parent would make the hierarchy loop")]
    Cycle,
    #[error("{0}")]
    BadLink(String),
    #[error("duplicate node id {0}")]
    DuplicateId(NodeId),
    #[error("node {0} names a parent that does not exist")]
    MissingParent(NodeId),
    #[error("scene file is {0} bytes, over the {MAX_FILE_BYTES}-byte limit")]
    TooLarge(usize),
    #[error("scene file is version {0}, newer than this build understands ({FORMAT_VERSION})")]
    TooNew(u32),
    #[error("invalid scene file: {0}")]
    Parse(String),
}

/// Local transform: translation, XYZ Euler rotation in degrees, scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub translation: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self { translation: [0.0; 3], rotation: [0.0; 3], scale: [1.0; 3] }
    }
}

fn finite3(v: [f32; 3], fallback: f32) -> Vec3 {
    let f = |x: f32| if x.is_finite() { x } else { fallback };
    Vec3::new(f(v[0]), f(v[1]), f(v[2]))
}

impl Transform {
    pub fn at(x: f32, y: f32, z: f32) -> Self {
        Self { translation: [x, y, z], ..Self::default() }
    }

    /// Local matrix; non-finite components fall back to the identity value.
    pub fn matrix(&self) -> Mat4 {
        let t = finite3(self.translation, 0.0);
        let r = finite3(self.rotation, 0.0);
        let s = finite3(self.scale, 1.0);
        let q = Quat::from_euler(EulerRot::XYZ, r.x.to_radians(), r.y.to_radians(), r.z.to_radians());
        Mat4::from_scale_rotation_translation(s, q, t)
    }
}

/// Surface look. Colours are sRGB 0–1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Material {
    pub color: [f32; 3],
    pub emissive: [f32; 3],
    /// Blinn–Phong exponent, 1–512.
    pub shininess: f32,
    /// Specular strength, 0–1.
    pub specular: f32,
    /// Ignore lights and show `color` as is (what VRM/MToon models ask for).
    pub unlit: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self { color: [0.8, 0.8, 0.8], emissive: [0.0; 3], shininess: 32.0, specular: 0.3, unlit: false }
    }
}

/// A simple parametric solid. Sizes are clamped when the mesh is built, so any value is safe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "camelCase")]
pub enum Primitive {
    Cube { size: [f32; 3] },
    Sphere { radius: f32, segments: u32 },
    Cylinder { radius: f32, height: f32, segments: u32 },
    Cone { radius: f32, height: f32, segments: u32 },
    /// A flat sheet in the XZ plane facing +Y.
    Plane { size: [f32; 2] },
}

impl Primitive {
    pub fn cube() -> Self {
        Primitive::Cube { size: [1.0; 3] }
    }
    pub fn sphere() -> Self {
        Primitive::Sphere { radius: 0.5, segments: 32 }
    }
    pub fn cylinder() -> Self {
        Primitive::Cylinder { radius: 0.5, height: 1.0, segments: 32 }
    }
    pub fn cone() -> Self {
        Primitive::Cone { radius: 0.5, height: 1.0, segments: 32 }
    }
    pub fn plane() -> Self {
        Primitive::Plane { size: [4.0, 4.0] }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Primitive::Cube { .. } => "Cube",
            Primitive::Sphere { .. } => "Sphere",
            Primitive::Cylinder { .. } => "Cylinder",
            Primitive::Cone { .. } => "Cone",
            Primitive::Plane { .. } => "Plane",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LightKind {
    /// Shines along the node's local −Z axis.
    Directional,
    Point,
    /// Cone along the node's local −Z axis; `angle` is the full cone angle in degrees.
    Spot { angle: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Light {
    #[serde(flatten)]
    pub kind: LightKind,
    pub color: [f32; 3],
    pub intensity: f32,
    /// Reach of point and spot lights in scene units.
    pub range: f32,
}

impl Default for Light {
    fn default() -> Self {
        Self { kind: LightKind::Point, color: [1.0; 3], intensity: 1.0, range: 20.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NodeKind {
    Empty,
    Primitive { primitive: Primitive, #[serde(default)] material: Material },
    Light(Light),
    /// A joint of an imported skeleton: posed by rotating it.
    Bone,
    /// One mesh of an imported model (see [`ModelMesh`]).
    Mesh(ModelMesh),
}

/// A mesh from an imported model. Skinned meshes follow their `joints`; the node's own
/// transform is ignored for them (glTF rule), unskinned ones sit where the node is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelMesh {
    pub asset: u32,
    /// Index into the model's `meshes`.
    pub mesh: u32,
    /// Scene node of each skin joint, in the skin's order (empty when unskinned).
    pub joints: Vec<NodeId>,
}

/// Local Euler (XYZ, degrees) range a joint may be posed within.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RotationLimits {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Default for RotationLimits {
    fn default() -> Self {
        Self { min: [-180.0; 3], max: [180.0; 3] }
    }
}

impl RotationLimits {
    /// `rotation` clamped into the range; non-finite values become 0.
    pub fn clamp(&self, rotation: [f32; 3]) -> [f32; 3] {
        let mut out = rotation;
        for (i, v) in out.iter_mut().enumerate() {
            let (lo, hi) = (self.min[i], self.max[i]);
            let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
            *v = if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
        }
        out
    }
}

/// An imported file the scene refers to. Its geometry is not stored in the scene file: the
/// editor reloads it from `path`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Asset {
    pub id: u32,
    pub name: String,
    pub path: Option<String>,
}

/// A joint's pose inside a [`Pose`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PoseBone {
    pub node: NodeId,
    pub rotation: [f32; 3],
    pub translation: [f32; 3],
}

impl Default for PoseBone {
    fn default() -> Self {
        Self { node: 0, rotation: [0.0; 3], translation: [0.0; 3] }
    }
}

/// A named snapshot of every bone's rotation and position.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Pose {
    pub name: String,
    pub bones: Vec<PoseBone>,
    /// The pose sliders' values when it was saved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<crate::pose_control::ControlValue>,
}

/// Models loaded in memory, by asset id. Not part of the scene file.
#[derive(Clone, Default)]
pub struct ModelCache(pub(crate) std::collections::HashMap<u32, std::sync::Arc<photocraft_model3d::ModelData>>);

impl PartialEq for ModelCache {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().all(|(k, v)| other.0.get(k).is_some_and(|o| std::sync::Arc::ptr_eq(v, o)))
    }
}

impl std::fmt::Debug for ModelCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ModelCache({} models)", self.0.len())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub parent: Option<NodeId>,
    pub visible: bool,
    pub transform: Transform,
    pub kind: NodeKind,
    /// The transform the node was imported with; "Reset pose" returns to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest: Option<Transform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<RotationLimits>,
    /// Ties the node to another at a distance and angle (see [`crate::Link`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<crate::Link>,
    /// Pinned solids stay where they are when the physics runs; unpinned ones fall and collide.
    #[serde(skip_serializing_if = "is_true")]
    pub pinned: bool,
    /// For a bone of a VRM humanoid: its standard name (`leftUpperArm`, `hips`, …). The physics
    /// uses it to size and group the bone's collision capsule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub humanoid: Option<String>,
    /// A joint that is not a hand control (fingers, face, tongue…): no dot on the canvas and no
    /// row in the list. It is still posed by sliders and saved poses, and still simulated.
    #[serde(skip_serializing_if = "is_false")]
    pub minor: bool,
    /// Anatomical swing-twist limits (from the model file); honoured by posing and IK.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joint: Option<crate::JointLimit>,
    /// A bone frozen at this world position: moving the bones above it turns the limb to keep it
    /// there (see [`crate::enforce_holds`]).
    #[serde(skip_serializing_if = "no_anchor")]
    pub hold: Option<[f32; 3]>,
    /// A bone whose orientation is frozen: this world rotation (`[x, y, z, w]`) is kept while the
    /// bones above it move or turn. Independent of [`Node::hold`], like VaM's separate position and
    /// rotation controls.
    #[serde(skip_serializing_if = "no_anchor")]
    pub hold_rot: Option<[f32; 4]>,
    /// For a node of an imported model: the model's asset id and the node's index in the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<(u32, u32)>,
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// An anchor that is missing or not made of finite numbers is not saved (JSON has no NaN, and a
/// scene with `null` in an anchor would not open).
fn no_anchor<const N: usize>(a: &Option<[f32; N]>) -> bool {
    a.is_none_or(|v| !v.iter().all(|c| c.is_finite()))
}

impl Default for Node {
    fn default() -> Self {
        Self { id: 0, name: String::new(), parent: None, visible: true, transform: Transform::default(), kind: NodeKind::Empty, rest: None, limits: None, link: None, pinned: true, humanoid: None, minor: false, joint: None, hold: None, hold_rot: None, source: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Camera {
    pub position: [f32; 3],
    pub target: [f32; 3],
    /// Vertical field of view in degrees.
    pub fov: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { position: [3.0, 2.0, 5.0], target: [0.0, 0.5, 0.0], fov: 45.0, near: 0.05, far: 200.0 }
    }
}

impl Camera {
    /// Camera orbiting `target`: `yaw`/`pitch` in degrees, `distance` in scene units.
    pub fn orbit(target: [f32; 3], yaw: f32, pitch: f32, distance: f32) -> [f32; 3] {
        let (yaw, pitch) = (yaw.to_radians(), pitch.clamp(-89.0, 89.0).to_radians());
        let d = if distance.is_finite() { distance.max(0.01) } else { 1.0 };
        let t = finite3(target, 0.0);
        let p = t + Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()) * d;
        p.to_array()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Scene {
    pub version: u32,
    pub nodes: Vec<Node>,
    pub camera: Camera,
    /// Sky colour behind the scene (sRGB 0–1).
    pub background: [f32; 3],
    /// Flat ambient light (sRGB 0–1).
    pub ambient: [f32; 3],
    pub assets: Vec<Asset>,
    pub poses: Vec<Pose>,
    /// Values of the pose sliders that are not at 0.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<crate::pose_control::ControlValue>,
    next_id: NodeId,
    #[serde(skip)]
    pub(crate) models: ModelCache,
}

impl Default for Scene {
    fn default() -> Self {
        Self { version: FORMAT_VERSION, nodes: Vec::new(), camera: Camera::default(), background: [0.18, 0.19, 0.22], ambient: [0.18, 0.18, 0.2], assets: Vec::new(), poses: Vec::new(), controls: Vec::new(), next_id: 1, models: ModelCache::default() }
    }
}

impl Scene {
    /// An empty scene with one sun light, so a first primitive is visibly lit.
    pub fn new() -> Self {
        let mut s = Self::default();
        let sun = Node {
            name: "Sun".into(),
            transform: Transform { translation: [0.0, 5.0, 0.0], rotation: [-50.0, 30.0, 0.0], scale: [1.0; 3] },
            kind: NodeKind::Light(Light { kind: LightKind::Directional, intensity: 1.1, ..Light::default() }),
            ..Node::default()
        };
        // A fresh scene is far below the node limit.
        let _ = s.add(sun);
        s
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Adds `node` under a fresh id (its own `id` is ignored) and returns that id.
    pub fn add(&mut self, mut node: Node) -> Result<NodeId, SceneError> {
        if self.nodes.len() >= MAX_NODES {
            return Err(SceneError::TooManyNodes);
        }
        if let Some(p) = node.parent
            && self.node(p).is_none()
        {
            return Err(SceneError::NoSuchNode(p));
        }
        let id = self.next_id.max(self.nodes.iter().map(|n| n.id).max().unwrap_or(0).saturating_add(1));
        self.next_id = id.saturating_add(1);
        node.id = id;
        if node.name.is_empty() {
            node.name = match &node.kind {
                NodeKind::Empty => "Empty".into(),
                NodeKind::Primitive { primitive, .. } => primitive.label().into(),
                NodeKind::Light(_) => "Light".into(),
                NodeKind::Bone => "Bone".into(),
                NodeKind::Mesh(_) => "Mesh".into(),
            };
        }
        self.nodes.push(node);
        Ok(id)
    }

    pub fn add_primitive(&mut self, primitive: Primitive, transform: Transform) -> Result<NodeId, SceneError> {
        self.add(Node { transform, kind: NodeKind::Primitive { primitive, material: Material::default() }, ..Node::default() })
    }

    /// Removes a node and everything below it. Returns how many nodes went.
    pub fn remove(&mut self, id: NodeId) -> Result<usize, SceneError> {
        if self.node(id).is_none() {
            return Err(SceneError::NoSuchNode(id));
        }
        let mut doomed = vec![id];
        let mut i = 0;
        while let Some(&cur) = doomed.get(i) {
            i += 1;
            for n in &self.nodes {
                if n.parent == Some(cur) && !doomed.contains(&n.id) {
                    doomed.push(n.id);
                }
            }
        }
        let before = self.nodes.len();
        self.nodes.retain(|n| !doomed.contains(&n.id));
        self.drop_dangling_links();
        Ok(before - self.nodes.len())
    }

    /// Re-parents `id` (None = root). The node keeps its local transform.
    pub fn set_parent(&mut self, id: NodeId, parent: Option<NodeId>) -> Result<(), SceneError> {
        if self.node(id).is_none() {
            return Err(SceneError::NoSuchNode(id));
        }
        if let Some(p) = parent {
            if self.node(p).is_none() {
                return Err(SceneError::NoSuchNode(p));
            }
            // Walk up from the new parent: meeting `id` means a loop.
            let mut cur = Some(p);
            for _ in 0..=MAX_DEPTH + self.nodes.len() {
                match cur {
                    Some(c) if c == id => return Err(SceneError::Cycle),
                    Some(c) => cur = self.node(c).and_then(|n| n.parent),
                    None => break,
                }
            }
        }
        if let Some(n) = self.node_mut(id) {
            n.parent = parent;
        }
        Ok(())
    }

    /// Model-to-world matrix, or `None` for an unknown id or a chain deeper than [`MAX_DEPTH`].
    pub fn world_matrix(&self, id: NodeId) -> Option<Mat4> {
        crate::link::resolve(&|i| self.node(i), id, &mut std::collections::HashMap::new(), 0)
    }

    /// Whether the node and all of its ancestors are visible.
    pub fn is_visible(&self, id: NodeId) -> bool {
        let mut cur = Some(id);
        for _ in 0..=MAX_DEPTH {
            let Some(c) = cur else { return true };
            let Some(n) = self.node(c) else { return false };
            if !n.visible {
                return false;
            }
            cur = n.parent;
        }
        false
    }

    /// Checks ids and parents; a loaded scene must pass this before anything walks it.
    pub fn validate(&self) -> Result<(), SceneError> {
        if self.nodes.len() > MAX_NODES {
            return Err(SceneError::TooManyNodes);
        }
        let mut seen = std::collections::HashSet::new();
        for n in &self.nodes {
            if !seen.insert(n.id) {
                return Err(SceneError::DuplicateId(n.id));
            }
        }
        for n in &self.nodes {
            if let Some(p) = n.parent
                && !seen.contains(&p)
            {
                return Err(SceneError::MissingParent(n.id));
            }
        }
        for n in &self.nodes {
            let mut cur = n.parent;
            for step in 0..=self.nodes.len() {
                match cur {
                    None => break,
                    Some(c) if c == n.id || step == self.nodes.len() => return Err(SceneError::Cycle),
                    Some(c) => cur = self.node(c).and_then(|x| x.parent),
                }
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, SceneError> {
        serde_json::to_string_pretty(self).map_err(|e| SceneError::Parse(e.to_string()))
    }

    pub fn from_json(text: &str) -> Result<Scene, SceneError> {
        if text.len() > MAX_FILE_BYTES {
            return Err(SceneError::TooLarge(text.len()));
        }
        let mut s: Scene = serde_json::from_str(text).map_err(|e| SceneError::Parse(e.to_string()))?;
        if s.version > FORMAT_VERSION {
            return Err(SceneError::TooNew(s.version));
        }
        s.validate()?;
        s.drop_dangling_links();
        s.next_id = s.next_id.max(s.nodes.iter().map(|n| n.id).max().unwrap_or(0).saturating_add(1));
        Ok(s)
    }
}
