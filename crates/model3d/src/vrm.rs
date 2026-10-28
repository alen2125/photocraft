//! VRM 0.x and 1.0 extensions: humanoid bone map and spring-bone chains.

use crate::{BodyData, ColliderData, ColliderGroup, ControlBoneData, ControlRot, DriveData, JointLimitData, ModelData, ModelFormat, PoseControlData, SpringGroup};
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
    if let Some(v) = doc.extension_value("PC_controls") {
        data.controls = v.get("nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|n| index(n, count)).collect();
    }
    if let Some(v) = doc.extension_value("PC_physics") {
        read_physics(v, data);
    }
    if let Some(v) = doc.extension_value("PC_pose_controls") {
        read_pose_controls(v, data);
    }
    if let Some(v) = doc.extension_value("PC_limits") {
        for j in v.get("joints").and_then(Value::as_array).into_iter().flatten().take(count) {
            let Some(node) = j.get("node").and_then(|n| index(n, count)) else { continue };
            let pair = |key: &str, fallback: [f32; 2]| -> [f32; 2] {
                match j.get(key).and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().map(|f| f as f32)).collect::<Vec<_>>()) {
                    Some(a) => match a.as_slice() {
                        [Some(x), Some(y)] if x.is_finite() && y.is_finite() => [*x, *y],
                        _ => fallback,
                    },
                    None => fallback,
                }
            };
            let frame = match j.get("frame").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().map(|f| f as f32)).collect::<Vec<_>>()) {
                Some(a) => match a.as_slice() {
                    [Some(x), Some(y), Some(z), Some(w)] if [x, y, z, w].iter().all(|c| c.is_finite()) => [*x, *y, *z, *w],
                    _ => [0.0, 0.0, 0.0, 1.0],
                },
                None => [0.0, 0.0, 0.0, 1.0],
            };
            data.joint_limits.push(JointLimitData { node, frame, twist: pair("twist", [-180.0, 180.0]), swing: pair("swing", [180.0, 180.0]) });
        }
    }
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

/// At most this many sliders, joint turns per slider and links per slider are read.
const MAX_CONTROLS: usize = 512;
const MAX_PER_CONTROL: usize = 256;

fn floats<const N: usize>(v: Option<&Value>) -> Option<[f32; N]> {
    let a = v?.as_array()?;
    if a.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        let f = x.as_f64()? as f32;
        if !f.is_finite() {
            return None;
        }
        *o = f;
    }
    Some(out)
}

/// At most this many bodies and colliders are read.
const MAX_BODIES: usize = 256;
const MAX_COLLIDERS: usize = 2048;

fn finite(v: Option<&Value>) -> Option<f32> {
    v.and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite())
}

fn drive(v: Option<&Value>, spring: &str, damper: &str, max: &str) -> Option<DriveData> {
    let v = v?;
    Some(DriveData { spring: finite(v.get(spring))?.max(0.0), damper: finite(v.get(damper))?.max(0.0), max_force: finite(v.get(max))?.max(0.0) })
}

fn read_physics(v: &Value, data: &mut ModelData) {
    let count = data.nodes.len();
    let p = &mut data.physics;
    p.step = finite(v.get("step")).filter(|s| (1e-4..=0.1).contains(s));
    p.iterations = v.get("iterations").and_then(Value::as_u64).filter(|i| (1..=64).contains(i)).map(|i| i as u32);
    p.friction = finite(v.get("friction")).map(|f| f.clamp(0.0, 5.0));
    p.bounciness = finite(v.get("bounciness")).map(|f| f.clamp(0.0, 1.0));
    p.max_velocity = finite(v.get("maxVelocity")).filter(|m| *m > 0.0);
    let hold = v.get("hold");
    p.hold_position = drive(hold, "posSpring", "posDamper", "posMax");
    p.hold_rotation = drive(hold, "rotSpring", "rotDamper", "rotMax");
    for b in v.get("bones").and_then(Value::as_array).into_iter().flatten().take(MAX_BODIES) {
        let (Some(node), Some(mass)) = (b.get("node").and_then(|n| index(n, count)), finite(b.get("mass")).filter(|m| *m > 0.0)) else { continue };
        let muscle = b.get("muscle").and_then(Value::as_array).and_then(|a| match a.as_slice() {
            [s, d, m] => Some(DriveData { spring: finite(Some(s))?.max(0.0), damper: finite(Some(d))?.max(0.0), max_force: finite(Some(m))?.max(0.0) }),
            _ => None,
        });
        p.bodies.push(BodyData { node, mass, drag: finite(b.get("drag")).unwrap_or(0.0).max(0.0), angular_drag: finite(b.get("angDrag")).unwrap_or(0.0).max(0.0), muscle });
    }
    for c in v.get("colliders").and_then(Value::as_array).into_iter().flatten().take(MAX_COLLIDERS) {
        let (Some(node), Some(a), Some(b), Some(radius)) = (c.get("node").and_then(|n| index(n, count)), floats::<3>(c.get("a")), floats::<3>(c.get("b")), finite(c.get("radius")).filter(|r| *r > 0.0)) else { continue };
        p.colliders.push(ColliderData { node, a, b, radius });
    }
}

fn read_pose_controls(v: &Value, data: &mut ModelData) {
    let count = data.nodes.len();
    for b in v.get("bones").and_then(Value::as_array).into_iter().flatten().take(count) {
        let (Some(node), Some(frame)) = (b.get("node").and_then(|n| index(n, count)), floats::<4>(b.get("frame"))) else { continue };
        let order = b.get("order").and_then(Value::as_u64).map_or(0, |o| o.min(5) as u8);
        data.control_bones.push(ControlBoneData { node, frame, order });
    }
    for c in v.get("controls").and_then(Value::as_array).into_iter().flatten().take(MAX_CONTROLS) {
        let Some(name) = c.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) else { continue };
        let text = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (Some(min), Some(max)) = (c.get("min").and_then(Value::as_f64).map(|f| f as f32), c.get("max").and_then(Value::as_f64).map(|f| f as f32)) else { continue };
        if !min.is_finite() || !max.is_finite() {
            continue;
        }
        let rot = c
            .get("rot")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_PER_CONTROL)
            .filter_map(|r| Some(ControlRot { node: index(r.get("node")?, count)?, deg: floats::<3>(r.get("deg"))? }))
            .collect();
        let links = c
            .get("links")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_PER_CONTROL)
            .filter_map(|l| {
                let to = l.get("to")?.as_str()?.to_string();
                let m = l.get("mult")?.as_f64()? as f32;
                m.is_finite().then_some((to, m))
            })
            .collect();
        data.pose_controls.push(PoseControlData { name: name.to_string(), label: if text("label").is_empty() { name.to_string() } else { text("label") }, group: text("group"), min, max, rot, links });
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
