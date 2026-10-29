//! Imported models in the scene: instantiating a model's skeleton as nodes, world matrices for the
//! whole scene at once, and named poses.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{EulerRot, Mat4, Quat, Vec3};
use photocraft_model3d::{ModelData, ModelFormat};

use crate::scene::{Asset, ModelMesh, Node, NodeId, NodeKind, Pose, PoseBone, Scene, SceneError, Transform, MAX_NODES};

fn node_transform(n: &photocraft_model3d::ModelNode) -> Transform {
    let q = Quat::from_xyzw(n.rotation[0], n.rotation[1], n.rotation[2], n.rotation[3]);
    let q = if q.is_finite() && q.length_squared() > 1e-12 { q.normalize() } else { Quat::IDENTITY };
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    let fin = |v: [f32; 3], d: f32| [0, 1, 2].map(|i| if v[i].is_finite() { v[i] } else { d });
    Transform { translation: fin(n.translation, 0.0), rotation: [x.to_degrees(), y.to_degrees(), z.to_degrees()], scale: fin(n.scale, 1.0) }
}

impl Scene {
    /// The loaded model for an asset, if it is in memory.
    pub fn model(&self, asset: u32) -> Option<&Arc<ModelData>> {
        self.models.0.get(&asset)
    }

    /// Assets some node uses whose model is not loaded (a scene just read from a file).
    pub fn missing_assets(&self) -> Vec<&Asset> {
        self.assets.iter().filter(|a| !self.models.0.contains_key(&a.id)).collect()
    }

    /// Supplies the model for an asset named in the scene.
    pub fn attach_model(&mut self, asset: u32, model: Arc<ModelData>) -> Result<(), SceneError> {
        if !self.assets.iter().any(|a| a.id == asset) {
            return Err(SceneError::Parse(format!("scene has no asset {asset}")));
        }
        self.models.0.insert(asset, model);
        // A model brought in fresh is the unmorphed one: morphs are applied to it again.
        self.reset_morph_base(asset);
        self.relink_joints(asset);
        if self.morphs.iter().any(|m| m.asset == asset) {
            self.apply_morphs(asset)?;
        }
        Ok(())
    }

    /// A scene saved before nodes remembered where in the model they came from has bones without
    /// that link (and without the model's joint limits and controls list). The skinned meshes
    /// know their joints in the skin's order, so the link is rebuilt from them; joints that
    /// already have one are left as they are.
    fn relink_joints(&mut self, asset: u32) {
        let Some(model) = self.models.0.get(&asset).cloned() else { return };
        let mut pairs: Vec<(NodeId, usize)> = Vec::new();
        for n in &self.nodes {
            let NodeKind::Mesh(m) = &n.kind else { continue };
            if m.asset != asset {
                continue;
            }
            let Some(skin) = model.meshes.get(m.mesh as usize).and_then(|mesh| mesh.skin).and_then(|s| model.skins.get(s)) else { continue };
            pairs.extend(m.joints.iter().copied().zip(skin.joints.iter().copied()));
        }
        for (id, idx) in pairs {
            let Some(n) = self.node_mut(id) else { continue };
            if n.source.is_some() {
                continue;
            }
            n.source = Some((asset, idx as u32));
            if !model.controls.is_empty() && !model.controls.contains(&idx) {
                n.minor = true;
            }
            if n.joint.is_none() {
                n.joint = model.joint_limits.iter().find(|j| j.node == idx).map(|j| crate::JointLimit { frame: j.frame, twist: j.twist, swing: j.swing, enabled: true });
            }
        }
    }

    /// Adds a model as a tree of nodes under a new root and returns the root's id. Joints become
    /// [`NodeKind::Bone`] nodes, so posing is rotating nodes; each mesh becomes a
    /// [`NodeKind::Mesh`] node. Nothing is added when the scene has no room for all of it.
    pub fn add_model(&mut self, model: Arc<ModelData>, path: Option<String>) -> Result<NodeId, SceneError> {
        let needed = model.nodes.len() + model.meshes.len() + 1;
        if self.nodes.len() + needed > MAX_NODES {
            return Err(SceneError::TooManyNodes);
        }
        let asset = self.assets.iter().map(|a| a.id).max().map_or(1, |m| m.saturating_add(1));
        let stem = std::path::Path::new(&model.name).file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("Model").to_string();

        let mut root_t = Transform::default();
        if model.format == ModelFormat::Vrm0 {
            // VRM 0.x characters face -Z; turn them to face the default camera.
            root_t.rotation = [0.0, 180.0, 0.0];
        }
        let root = self.add(Node { name: stem, transform: root_t, ..Node::default() })?;

        // Parents before children; a node in a glTF child loop is never reached and is skipped.
        let mut ids: Vec<Option<NodeId>> = vec![None; model.nodes.len()];
        let mut stack: Vec<usize> = model.nodes.iter().enumerate().filter(|(_, n)| n.parent.is_none()).map(|(i, _)| i).rev().collect();
        while let Some(i) = stack.pop() {
            let Some(n) = model.nodes.get(i) else { continue };
            if ids.get(i).copied().flatten().is_some() {
                continue;
            }
            let parent = n.parent.and_then(|p| ids.get(p).copied().flatten()).unwrap_or(root);
            let t = node_transform(n);
            let kind = if model.is_joint(i) { NodeKind::Bone } else { NodeKind::Empty };
            let humanoid = model.humanoid.iter().find(|(_, node)| *node == i).map(|(name, _)| name.clone());
            let minor = matches!(kind, NodeKind::Bone) && !model.controls.is_empty() && !model.controls.contains(&i);
            let joint = model.joint_limits.iter().find(|j| j.node == i).map(|j| crate::JointLimit { frame: j.frame, twist: j.twist, swing: j.swing, enabled: true });
            let id = self.add(Node { name: n.name.clone(), parent: Some(parent), transform: t.clone(), rest: Some(t), kind, humanoid, minor, joint, source: Some((asset, i as u32)), ..Node::default() })?;
            if let Some(slot) = ids.get_mut(i) {
                *slot = Some(id);
            }
            stack.extend(n.children.iter().rev().copied());
        }

        for (mi, mesh) in model.meshes.iter().enumerate() {
            let (parent, joints) = match mesh.skin.and_then(|s| model.skins.get(s)) {
                Some(skin) => {
                    let joints: Option<Vec<NodeId>> = skin.joints.iter().map(|j| ids.get(*j).copied().flatten()).collect();
                    match joints {
                        Some(j) => (root, j),
                        None => continue,
                    }
                }
                None => (ids.get(mesh.node).copied().flatten().unwrap_or(root), Vec::new()),
            };
            self.add(Node {
                name: mesh.name.clone(),
                parent: Some(parent),
                kind: NodeKind::Mesh(ModelMesh { asset, mesh: mi as u32, joints }),
                ..Node::default()
            })?;
        }
        self.assets.push(Asset { id: asset, name: model.name.clone(), path });
        self.models.0.insert(asset, model);
        Ok(root)
    }

    /// World matrix of every node, computed once. Nodes in a chain deeper than [`MAX_DEPTH`] are left out.
    pub fn world_matrices(&self) -> HashMap<NodeId, Mat4> {
        let by_id: HashMap<NodeId, &Node> = self.nodes.iter().map(|n| (n.id, n)).collect();
        let find = |id: NodeId| by_id.get(&id).copied();
        let mut out: HashMap<NodeId, Mat4> = HashMap::with_capacity(self.nodes.len());
        for n in &self.nodes {
            // Parents and link targets first; a node too deep (or in a loop) is left out.
            let _ = crate::link::resolve(&find, n.id, &mut out, 0);
        }
        out
    }

    /// Saves every bone's current rotation and position as a pose and returns its index. A pose
    /// with the same name is replaced.
    pub fn capture_pose(&mut self, name: &str) -> usize {
        let bones = self
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Bone))
            .map(|n| PoseBone { node: n.id, rotation: n.transform.rotation, translation: n.transform.translation })
            .collect();
        let pose = Pose { name: name.to_string(), bones, controls: self.controls.clone() };
        match self.poses.iter().position(|p| p.name == name) {
            Some(i) => {
                self.poses[i] = pose;
                i
            }
            None => {
                self.poses.push(pose);
                self.poses.len() - 1
            }
        }
    }

    /// Puts the bones in a saved pose. Bones that no longer exist are skipped.
    pub fn apply_pose(&mut self, index: usize) -> Result<(), SceneError> {
        let pose = self.poses.get(index).cloned().ok_or_else(|| SceneError::Parse(format!("no pose {index}")))?;
        for b in &pose.bones {
            if let Some(n) = self.node_mut(b.node) {
                n.transform.rotation = b.rotation;
                n.transform.translation = b.translation;
            }
        }
        // The sliders show what the saved bones were set to.
        self.controls = pose.controls;
        Ok(())
    }

    /// Returns every imported node to the transform it was loaded with.
    pub fn reset_pose(&mut self) {
        self.controls.clear();
        for n in &mut self.nodes {
            if let Some(rest) = &n.rest {
                n.transform = rest.clone();
            }
        }
    }

    /// Positions of the world origin of every bone that is a hand control, for drawing the
    /// skeleton. A bone's parent is its nearest control ancestor, so lines skip the minor joints.
    pub fn bone_positions(&self) -> Vec<(NodeId, Option<NodeId>, Vec3)> {
        let world = self.world_matrices();
        let is_bone = |id: NodeId| self.node(id).filter(|n| matches!(n.kind, NodeKind::Bone));
        self.nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Bone) && !n.minor)
            .filter_map(|n| {
                let p = world.get(&n.id)?.transform_point3(Vec3::ZERO);
                // Walk up (bounded) to the nearest bone that is shown.
                let mut parent = n.parent;
                for _ in 0..crate::MAX_DEPTH {
                    match parent.and_then(is_bone) {
                        Some(pn) if pn.minor => parent = pn.parent,
                        Some(pn) => {
                            parent = Some(pn.id);
                            break;
                        }
                        None => {
                            parent = None;
                            break;
                        }
                    }
                }
                p.is_finite().then_some((n.id, parent, p))
            })
            .collect()
    }
}
