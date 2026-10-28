//! Synthetic model files for tests (here and in crates that load models).

/// A one-triangle skinned model: joints `hips` (node 0) and `spine` (node 1, one unit up), the
/// mesh on node 2. `last_index` lets a test point an index past the vertices.
pub fn skinned_triangle_glb(vrm0: bool, last_index: u16) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in v {
            bin.extend(c.to_le_bytes());
        }
    }
    bin.extend([0u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]); // joints (u8 x4) x3: 0,0,1
    for w in [[1.0f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]] {
        for c in w {
            bin.extend(c.to_le_bytes());
        }
    }
    for i in [0u16, 1, last_index] {
        bin.extend(i.to_le_bytes());
    }
    bin.extend([0u8, 0]); // pad to 4
    for m in [[1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 1.0]] {
        for c in m {
            bin.extend(c.to_le_bytes());
        }
    }
    let ext = if vrm0 {
        r#","extensionsUsed":["VRM","PC_controls"],"extensions":{"PC_controls":{"nodes":[1,99,"x"]},"VRM":{"humanoid":{"humanBones":[{"bone":"hips","node":0},{"bone":"spine","node":1},{"bone":"bad","node":99}]},
        "secondaryAnimation":{"colliderGroups":[{"node":0,"colliders":[{"offset":{"x":0,"y":0.1,"z":0},"radius":0.05}]}],
        "boneGroups":[{"comment":"hair","stiffiness":0.5,"gravityPower":0.1,"dragForce":0.4,"hitRadius":0.02,"gravityDir":{"x":0,"y":-1,"z":0},"bones":[1,77],"colliderGroups":[0,5]}]}}}"#
    } else {
        ""
    };
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,2]}}],
        "nodes":[{{"name":"hips","children":[1]}},{{"name":"spine","translation":[0,1,0]}},{{"name":"body","mesh":0,"skin":0}}],
        "meshes":[{{"name":"Body","primitives":[{{"attributes":{{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2}},"indices":3,"material":0}}]}}],
        "materials":[{{"name":"skin","pbrMetallicRoughness":{{"baseColorFactor":[0.9,0.7,0.6,1]}},"alphaMode":"MASK","alphaCutoff":0.3}}],
        "skins":[{{"joints":[0,1],"inverseBindMatrices":4}}],
        "accessors":[
         {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
         {{"bufferView":1,"componentType":5121,"count":3,"type":"VEC4"}},
         {{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"}},
         {{"bufferView":3,"componentType":5123,"count":3,"type":"SCALAR"}},
         {{"bufferView":4,"componentType":5126,"count":2,"type":"MAT4"}}],
        "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":12}},{{"buffer":0,"byteOffset":48,"byteLength":48}},
         {{"buffer":0,"byteOffset":96,"byteLength":6}},{{"buffer":0,"byteOffset":104,"byteLength":128}}],
        "buffers":[{{"byteLength":{}}}]{ext}}}"#,
        bin.len()
    );
    let mut json = json.into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::new();
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(&bin);
    out
}
