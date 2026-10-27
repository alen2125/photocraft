//! VRM 0.x and 1.0 extensions: humanoid bone map and spring-bone chains.

use crate::{ColliderGroup, ModelData, ModelFormat, SpringGroup};
use gltf::json::Value;

fn num(v: &Value, key: &str) -> f32 {
    v.get(key).and_then(Value::as_f64).map_or(0.0, |f| f as f32)
}

fn vec3(v: Option<&Value>) -> [f32; 3] {
    let f = |k: &str| v.and_then(|o| o.get(k)).and_then(Value::as_f64).map_or(0.0, |f| f as f32);
    [f("x"), f("y"), f("z")]
}

fn index(v: &Value, count: usize) -> Option<usize> {
    v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < count)
}

pub(crate) fn read(doc: &gltf::Document, data: &mut ModelData) {
    let count = data.nodes.len();
    if let Some(v) = doc.extension_value("VRMC_vrm") {
        data.format = ModelFormat::Vrm1;
        if let Some(bones) = v.pointer("/humanoid/humanBones").and_then(Value::as_object) {
            for (name, b) in bones {
                if let Some(i) = b.get("node").and_then(|n| index(n, count)) {
                    data.humanoid.push((name.clone(), i));
                }
            }
        }
        read_springs_v1(doc, data);
    } else if let Some(v) = doc.extension_value("VRM") {
        data.format = ModelFormat::Vrm0;
        if let Some(bones) = v.pointer("/humanoid/humanBones").and_then(Value::as_array) {
            for b in bones {
                if let (Some(name), Some(i)) = (b.get("bone").and_then(Value::as_str), b.get("node").and_then(|n| index(n, count))) {
                    data.humanoid.push((name.to_string(), i));
                }
            }
        }
        read_springs_v0(v, data);
    }
}

fn read_springs_v0(v: &Value, data: &mut ModelData) {
    let count = data.nodes.len();
    let Some(sec) = v.get("secondaryAnimation") else { return };
    for g in sec.get("colliderGroups").and_then(Value::as_array).into_iter().flatten() {
        let Some(node) = g.get("node").and_then(|n| index(n, count)) else { continue };
        let spheres = g
            .get("colliders")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|c| (vec3(c.get("offset")), num(c, "radius")))
            .collect();
        data.collider_groups.push(ColliderGroup { node, spheres });
    }
    let groups = data.collider_groups.len();
    for g in sec.get("boneGroups").and_then(Value::as_array).into_iter().flatten() {
        data.springs.push(SpringGroup {
            comment: g.get("comment").and_then(Value::as_str).unwrap_or("").to_string(),
            // VRM 0 spells it "stiffiness".
            stiffness: g.get("stiffiness").map_or_else(|| num(g, "stiffness"), |s| s.as_f64().unwrap_or(0.0) as f32),
            gravity_power: num(g, "gravityPower"),
            gravity_dir: vec3(g.get("gravityDir")),
            drag_force: num(g, "dragForce"),
            hit_radius: num(g, "hitRadius"),
            roots: g.get("bones").and_then(Value::as_array).into_iter().flatten().filter_map(|b| index(b, count)).collect(),
            collider_groups: g.get("colliderGroups").and_then(Value::as_array).into_iter().flatten().filter_map(|c| index(c, groups)).collect(),
        });
    }
}

fn read_springs_v1(doc: &gltf::Document, data: &mut ModelData) {
    let count = data.nodes.len();
    let Some(sb) = doc.extension_value("VRMC_springBone") else { return };
    let colliders: Vec<(usize, ([f32; 3], f32))> = sb
        .get("colliders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let node = c.get("node").and_then(|n| index(n, count))?;
            let shape = c.get("shape")?;
            let s = shape.get("sphere").or_else(|| shape.get("capsule"))?;
            let offset = s.get("offset").map_or([0.0; 3], |o| {
                let a: Vec<f32> = o.as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
                [a.first().copied().unwrap_or(0.0), a.get(1).copied().unwrap_or(0.0), a.get(2).copied().unwrap_or(0.0)]
            });
            Some((node, (offset, num(s, "radius"))))
        })
        .collect();
    for g in sb.get("colliderGroups").and_then(Value::as_array).into_iter().flatten() {
        let members: Vec<&(usize, ([f32; 3], f32))> = g.get("colliders").and_then(Value::as_array).into_iter().flatten().filter_map(|i| index(i, colliders.len())).filter_map(|i| colliders.get(i)).collect();
        if let Some(first) = members.first() {
            data.collider_groups.push(ColliderGroup { node: first.0, spheres: members.iter().map(|m| m.1).collect() });
        }
    }
    let groups = data.collider_groups.len();
    for s in sb.get("springs").and_then(Value::as_array).into_iter().flatten() {
        let joints: Vec<&Value> = s.get("joints").and_then(Value::as_array).into_iter().flatten().collect();
        let Some(first) = joints.first() else { continue };
        data.springs.push(SpringGroup {
            comment: s.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            stiffness: num(first, "stiffness"),
            gravity_power: num(first, "gravityPower"),
            gravity_dir: first.get("gravityDir").map_or([0.0, -1.0, 0.0], |o| {
                let a: Vec<f32> = o.as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
                [a.first().copied().unwrap_or(0.0), a.get(1).copied().unwrap_or(-1.0), a.get(2).copied().unwrap_or(0.0)]
            }),
            drag_force: num(first, "dragForce"),
            hit_radius: num(first, "hitRadius"),
            roots: first.get("node").and_then(|n| index(n, count)).into_iter().collect(),
            collider_groups: s.get("colliderGroups").and_then(Value::as_array).into_iter().flatten().filter_map(|c| index(c, groups)).collect(),
        });
    }
}
