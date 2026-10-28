"""Export a VaM base figure (female or male) from the game's own asset bundles to a skinned .vrm (VRM 0.x)."""
import json, os, struct, sys
import UnityPy

SA = r"F:\Virt-a-Mate_1.22.0.3\VaM_1.22.0.3\VaM_Data\StreamingAssets"
OUT = sys.argv[2] if len(sys.argv) > 2 else "."
who = sys.argv[1]  # female | male
bundle, person = ("f_1", "a_per") if who == "female" else ("m_1", "a_per")
pos_key = "_worldPosition" if who == "female" else "_maleWorldPosition"

# ---- mesh + skin
env = UnityPy.load(os.path.join(SA, bundle))
mesh = skin = None
for o in env.objects:
    if o.type.name != "MonoBehaviour":
        continue
    cls = o.read(check_read=False).m_Script.read().m_ClassName
    if cls == "DAZMergedMesh":
        mesh = o.read_typetree()
    elif cls == "DAZMergedSkinV2":
        skin = o.read_typetree()
assert mesh and skin
n_base = mesh["_numBaseVertices"]
uv_pos = mesh["_UVVertices"]
uv_uv = mesh["_OrigUV"]
extra = {m["tovert"]: m["fromvert"] for m in mesh["_baseVerticesToUVVertices"]}
def base_of(j):
    return j if j < n_base else extra[j]

# ---- skeleton (DAZBone components from the person atom)
penv = UnityPy.load(os.path.join(SA, person))
bones = {}
for o in penv.objects:
    if o.type.name != "MonoBehaviour":
        continue
    try:
        cls = o.read(check_read=False).m_Script.read().m_ClassName
    except BaseException:
        continue
    if cls == "DAZBone":
        t = o.read_typetree()
        bones[o.path_id] = t
print("DAZBone components:", len(bones))
by_id = {}
for pid, t in bones.items():
    by_id.setdefault(t["_id"], []).append(pid)
dups = {k: len(v) for k, v in by_id.items() if len(v) > 1}
print("distinct bone ids:", len(by_id), "duplicated ids:", len(dups))

def pick(bid):
    # the bone of this id (several atoms carry the same rig; they hold the same data)
    return bones[by_id[bid][0]]

skin_names = []
for n in skin["nodes"]:
    if n["name"] not in skin_names:
        skin_names.append(n["name"])
missing = [n for n in skin_names if n not in by_id]
print("skin bones:", len(skin_names), "without a DAZBone:", missing)

# bones of the skeleton = skin bones + their ancestors
need = []
def add(bid):
    if bid in need or bid not in by_id:
        return
    t = pick(bid)
    par = bones.get(t["parentBone"]["m_PathID"])
    if par:
        add(par["_id"])
    need.append(bid)
for n in skin_names:
    add(n)
print("skeleton bones:", len(need))

# right-handed, character faces -Z (VRM 0.x): mirror the Unity z axis
def cv(p):
    return (p["x"], p["y"], -p["z"])
world = {b: cv(pick(b)[pos_key]) for b in need}
parent = {}
for b in need:
    par = bones.get(pick(b)["parentBone"]["m_PathID"])
    parent[b] = par["_id"] if par and par["_id"] in world else None

# ---- weights per base vertex
w = {}
for n in skin["nodes"]:
    src = [(g["vertex"], g["weight"]) for g in n["generalWeights"]] or [(g["vertex"], (g["xweight"] + g["yweight"] + g["zweight"]) / 3) for g in n["weights"]]
    for vtx, wt in src:
        d = w.setdefault(vtx, {})
        d[n["name"]] = d.get(n["name"], 0.0) + wt
    for f in n["fullyWeightedVertices"]:
        vtx = f if isinstance(f, int) else f.get("vertex", f.get("index", -1))
        w.setdefault(vtx, {})[n["name"]] = 1.0
joint_index = {b: i for i, b in enumerate(need)}
root = next(b for b in need if parent[b] is None)

# ---- primitives by material
mat_names = mesh["_materialNames"]
by_mat = {}
for poly in mesh["_UVPolyList"]:
    by_mat.setdefault(poly["materialNum"], []).append(poly["vertices"])

blob = bytearray()
views, accessors = [], []
def add_view(data, target=None):
    while len(blob) % 4:
        blob.append(0)
    views.append({"buffer": 0, "byteOffset": len(blob), "byteLength": len(data), **({"target": target} if target else {})})
    blob.extend(data)
    return len(views) - 1
def add_acc(view, ctype, count, typ, **kw):
    accessors.append({"bufferView": view, "componentType": ctype, "count": count, "type": typ, **kw})
    return len(accessors) - 1

nv = len(uv_pos)
pos_bytes = b"".join(struct.pack("<3f", *cv(p)) for p in uv_pos)
uv_bytes = b"".join(struct.pack("<2f", u["x"], 1.0 - u["y"]) for u in uv_uv)
xs = [cv(p) for p in uv_pos]
mn = [min(c[i] for c in xs) for i in range(3)]
mx = [max(c[i] for c in xs) for i in range(3)]
a_pos = add_acc(add_view(pos_bytes, 34962), 5126, nv, "VEC3", min=mn, max=mx)
a_uv = add_acc(add_view(uv_bytes, 34962), 5126, nv, "VEC2")

# smooth normals: area-weighted face normals, welded over the UV seams by base vertex
acc = {}
for polys in by_mat.values():
    for v in polys:
        for a, b, c in ([(v[0], v[1], v[2])] if len(v) == 3 else [(v[0], v[1], v[2]), (v[0], v[2], v[3])]):
            pa, pb, pc = xs[a], xs[b], xs[c]
            e1 = [pb[i] - pa[i] for i in range(3)]
            e2 = [pc[i] - pa[i] for i in range(3)]
            n = (e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0])
            for k in (a, b, c):
                t = acc.setdefault(base_of(k), [0.0, 0.0, 0.0])
                for i in range(3):
                    t[i] += -n[i]  # mirrored axis: the outward side is the opposite one
nbytes = bytearray()
for j in range(nv):
    t = acc.get(base_of(j), [0.0, 1.0, 0.0])
    l = (t[0] ** 2 + t[1] ** 2 + t[2] ** 2) ** 0.5 or 1.0
    nbytes += struct.pack("<3f", t[0] / l, t[1] / l, t[2] / l)
a_n = add_acc(add_view(bytes(nbytes), 34962), 5126, nv, "VEC3")

jb, wb = bytearray(), bytearray()
unweighted = 0
for j in range(nv):
    d = w.get(base_of(j), {})
    items = sorted(((joint_index[k], v) for k, v in d.items() if k in joint_index and v > 0), key=lambda x: -x[1])[:4]
    s = sum(v for _, v in items)
    if s <= 0:
        items, s = [(joint_index[root], 1.0)], 1.0
        unweighted += 1
    items += [(0, 0.0)] * (4 - len(items))
    jb += struct.pack("<4H", *[i for i, _ in items])
    wb += struct.pack("<4f", *[v / s for _, v in items])
a_j = add_acc(add_view(bytes(jb), 34962), 5123, nv, "VEC4")
a_w = add_acc(add_view(bytes(wb), 34962), 5126, nv, "VEC4")
print("vertices", nv, "unweighted (bound to root):", unweighted)

prims, materials = [], []
for m, polys in sorted(by_mat.items()):
    if mat_names[m] in ("Hidden",):
        continue
    idx = []
    for v in polys:
        tris = [(v[0], v[1], v[2])] if len(v) == 3 else [(v[0], v[1], v[2]), (v[0], v[2], v[3])]
        for a, b, c in tris:
            idx += [a, c, b]  # mirrored axis flips the winding
    iv = add_view(struct.pack(f"<{len(idx)}I", *idx), 34963)
    materials.append({"name": mat_names[m], "pbrMetallicRoughness": {"baseColorFactor": [0.85, 0.7, 0.62, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.8}, "doubleSided": False})
    prims.append({"attributes": {"POSITION": a_pos, "NORMAL": a_n, "TEXCOORD_0": a_uv, "JOINTS_0": a_j, "WEIGHTS_0": a_w}, "indices": add_acc(iv, 5125, len(idx), "SCALAR"), "material": len(materials) - 1})

# joints and inverse bind matrices
ibm = bytearray()
for b in need:
    x, y, z = world[b]
    ibm += struct.pack("<16f", 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -x, -y, -z, 1)
a_ibm = add_acc(add_view(bytes(ibm)), 5126, len(need), "MAT4")

nodes = [{"name": "Armature_" + who, "children": []}]
bone_node = {b: i + 1 for i, b in enumerate(need)}
for b in need:
    p = parent[b]
    px = world[p] if p else (0, 0, 0)
    nodes.append({"name": b, "translation": [world[b][i] - px[i] for i in range(3)], "children": []})
for b in need:
    p = parent[b]
    (nodes[bone_node[p]] if p else nodes[0])["children"].append(bone_node[b])
nodes.append({"name": who, "mesh": 0, "skin": 0})
mesh_node = len(nodes) - 1

# VRM 0.x humanoid
H = {"hip": "hips", "abdomen": "spine", "abdomen2": "chest", "chest": "upperChest", "neck": "neck", "head": "head", "lowerJaw": "jaw"}
for s, side in (("l", "left"), ("r", "right")):
    H.update({f"{s}Collar": f"{side}Shoulder", f"{s}Shldr": f"{side}UpperArm", f"{s}ForeArm": f"{side}LowerArm", f"{s}Hand": f"{side}Hand",
              f"{s}Thigh": f"{side}UpperLeg", f"{s}Shin": f"{side}LowerLeg", f"{s}Foot": f"{side}Foot", f"{s}Toe": f"{side}Toes", f"{s}Eye": f"{side}Eye"})
    for f, fn in (("Thumb", "Thumb"), ("Index", "Index"), ("Mid", "Middle"), ("Ring", "Ring"), ("Pinky", "Little")):
        for k, part in ((1, "Proximal"), (2, "Intermediate"), (3, "Distal")):
            if f == "Thumb":
                part = ("Proximal", "Intermediate", "Distal")[k - 1]
            H[f"{s}{f}{k}"] = f"{side}{fn}{part}"
human = [{"bone": v, "node": bone_node[k], "useDefaultValues": True} for k, v in H.items() if k in bone_node]
print("humanoid bones mapped:", len(human), "of", len(H))

gltf = {
    "asset": {"version": "2.0", "generator": "vam-base-export"},
    "extensionsUsed": ["VRM"],
    "extensions": {"VRM": {"exporterVersion": "vam-base-export", "specVersion": "0.0", "meta": {"title": "VaM base " + who, "version": "1"}, "humanoid": {"humanBones": human}}},
    "scene": 0, "scenes": [{"nodes": [0, mesh_node]}],
    "nodes": nodes, "meshes": [{"name": who, "primitives": prims}],
    "skins": [{"joints": [bone_node[b] for b in need], "inverseBindMatrices": a_ibm, "skeleton": 0}],
    "materials": materials, "accessors": accessors, "bufferViews": views, "buffers": [{"byteLength": len(blob)}],
}
js = json.dumps(gltf, separators=(",", ":")).encode()
js += b" " * (-len(js) % 4)
bin_ = bytes(blob) + b"\0" * (-len(blob) % 4)
glb = struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(js) + 8 + len(bin_)) + struct.pack("<I4s", len(js), b"JSON") + js + struct.pack("<I4s", len(bin_), b"BIN\0") + bin_
os.makedirs(OUT, exist_ok=True)
path = os.path.join(OUT, f"vam_base_{who}.vrm")
open(path, "wb").write(glb)
print("wrote", path, len(glb) // 1024, "KB; materials", len(materials), "joints", len(need))
