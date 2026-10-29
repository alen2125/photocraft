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
    # seam copies of a vertex (also some below n_base) map back to the vertex they duplicate
    return extra.get(j, j)

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

import collections
MAIN_OWNER = collections.Counter(t["dazBones"]["m_PathID"] for t in bones.values()).most_common(1)[0][0]

def pick(bid):
    # a_per holds the main rig (most bones, with parents) and a small helper rig; take the main one
    ids = by_id[bid]
    return bones[next((i for i in ids if bones[i]["dazBones"]["m_PathID"] == MAIN_OWNER), ids[0])]

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
primary = {}
general_idx = set()
for n in skin["nodes"]:
    src = [(g["vertex"], g["weight"]) for g in n["generalWeights"]] or [(g["vertex"], (g["xweight"] + g["yweight"] + g["zweight"]) / 3) for g in n["weights"]]
    for g in n["generalWeights"]:
        general_idx.add(g["vertex"])
    for vtx, wt in src:
        d = w.setdefault(vtx, {})
        d[n["name"]] = d.get(n["name"], 0.0) + wt
    # fullyWeightedVertices is each vertex's primary bone (every vertex is listed once); it is the
    # binding only for vertices that have no blended weights
    for f in n["fullyWeightedVertices"]:
        primary[f] = n["name"]
# Graft (genital) vertices have no blended weights in the file: the game moves them with the body
# seam. Give them the distance-weighted weights of the nearest body vertices instead, so they bend
# with the thighs and hips and never tear away from the body.
nv_all = len(uv_pos)
_cell = 0.03
_grid = {}
_body = [j for j in range(nv_all) if j in w and sum(w[j].values()) > 0]
for j in _body:
    p = uv_pos[j]
    _grid.setdefault((int(p["x"] // _cell), int(p["y"] // _cell), int(p["z"] // _cell)), []).append(j)
def _nearest(p, k=8):
    cx, cy, cz = int(p["x"] // _cell), int(p["y"] // _cell), int(p["z"] // _cell)
    for r in (1, 2, 3, 5):
        cand = [j for dx in range(-r, r + 1) for dy in range(-r, r + 1) for dz in range(-r, r + 1) for j in _grid.get((cx + dx, cy + dy, cz + dz), [])]
        if len(cand) >= k:
            break
    d = sorted(((uv_pos[j]["x"] - p["x"]) ** 2 + (uv_pos[j]["y"] - p["y"]) ** 2 + (uv_pos[j]["z"] - p["z"]) ** 2, j) for j in cand)[:k]
    return d
grafted = 0
for j in range(nv_all):
    if (j in w and sum(w[j].values()) > 0) or primary.get(j) not in ("pelvis", "hip"):
        continue
    blend = {}
    near = _nearest(uv_pos[j])
    tot = 0.0
    for d2, k in near:
        wt = 1.0 / (d2 + 1e-6)
        tot += wt
        for bone, v in w[k].items():
            blend[bone] = blend.get(bone, 0.0) + v * wt
    if tot > 0:
        w[j] = {b: v / tot for b, v in blend.items()}
        grafted += 1
print("graft vertices skinned from the body:", grafted)

# Refine: the graft is welded to the body along a seam (pairs of vertices that coincide). Seam
# vertices take their body twin's weights exactly; the vertices between them take the smooth
# average of their neighbours' weights (relaxed over the graft's own edges), so the patch follows
# the thighs and hips as one piece instead of tearing between a left and a right attachment.
pairs, graft_start = [], mesh["startGraftVertIndex"]
for o in env.objects:
    if o.type.name == "MonoBehaviour" and o.read(check_read=False).m_Script.read().m_ClassName == "DAZMesh":
        t = o.read_typetree()
        if t["meshGraft"]["vertexPairs"]:
            pairs = [(q["vertexNum"], q["graftToVertexNum"]) for q in t["meshGraft"]["vertexPairs"]]
            break
if pairs:
    def _norm(d):
        tot = sum(d.values()) or 1.0
        return {b: v / tot for b, v in d.items()}
    G = [j for j in range(nv_all) if j not in general_idx and primary.get(j) in ("pelvis", "hip")]
    GB = sorted({base_of(j) for j in G})
    GBset = set(GB)
    cur = {}
    for b in GB:  # start from the nearest-body-vertices blend
        near = _nearest(uv_pos[b] if b < nv_all else uv_pos[0])
        blend, tot = {}, 0.0
        for d2, k in near:
            wt = 1.0 / (d2 + 1e-6)
            tot += wt
            for bone, v in w[k].items():
                blend[bone] = blend.get(bone, 0.0) + v * wt
        cur[b] = _norm(blend) if tot > 0 else {}
    # The graft's seam vertices are copies of body vertices (they map back to a body index below
    # the graft's start): those keep the body's own weights.
    fixed = {}
    for b in GB:
        if b < graft_start and b in w and sum(w[b].values()) > 0:
            fixed[b] = _norm(w[b])
            cur[b] = fixed[b]
    nbr = {b: set() for b in GB}
    for pl in mesh["_UVPolyList"]:
        poly = pl["vertices"]
        for a, c in zip(poly, poly[1:] + poly[:1]):
            ba, bc = base_of(a), base_of(c)
            if ba in GBset and bc in GBset and ba != bc:
                nbr[ba].add(bc)
                nbr[bc].add(ba)
    free = [b for b in GB if b not in fixed and nbr[b]]
    for _ in range(400):
        for b in free:
            acc = {}
            for q in nbr[b]:
                for bone, v in cur[q].items():
                    acc[bone] = acc.get(bone, 0.0) + v
            cur[b] = _norm(acc)
    for j in G:
        if cur.get(base_of(j)):
            w[j] = cur[base_of(j)]
    print("graft relaxed:", len(free), "free of", len(GB), "vertices;", len(fixed), "welded to the seam")

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
lost = {}
for j in range(nv):
    # skin weights are indexed by UV vertex (seam copies carry their own entry); fall back to the base vertex
    d = w.get(j) or w.get(base_of(j)) or ({primary[j]: 1.0} if j in primary else {})
    items = sorted(((joint_index[k], v) for k, v in d.items() if k in joint_index and v > 0), key=lambda x: -x[1])[:4]
    s = sum(v for _, v in items)
    if s <= 0 and primary.get(j) in joint_index:  # blended weights were all zero: use the primary bone
        items, s = [(joint_index[primary[j]], 1.0)], 1.0
    if s <= 0:
        items, s = [(joint_index[root], 1.0)], 1.0
        unweighted += 1
        lost[str(sorted(d))] = lost.get(str(sorted(d)), 0) + 1
    items += [(0, 0.0)] * (4 - len(items))
    jb += struct.pack("<4H", *[i for i, _ in items])
    wb += struct.pack("<4f", *[v / s for _, v in items])
a_j = add_acc(add_view(bytes(jb), 34962), 5123, nv, "VEC4")
a_w = add_acc(add_view(bytes(wb), 34962), 5126, nv, "VEC4")
print("vertices", nv, "unweighted (bound to root):", unweighted, dict(list(lost.items())[:6]))

# ---- skin: one material per slot (Head, Torso, ...) with its main colour map, from the skin bundle
import io
import re
from PIL import Image

SKIN = sys.argv[3] if len(sys.argv) > 3 else ("f_1" if who == "female" else "m_1")
TEX_MAX = 2048
senv = UnityPy.load(*[os.path.join(SA, n) for n in (SKIN + "_mat", "p_eye_mat", "p_sha_mat", "z_sha")])
slot_mat = {}
for o in senv.objects:
    if o.type.name == "Material":
        d = o.read()
        if d.m_Name.endswith("ForDefaults"):
            continue
        slot = re.sub(r"-\d+$", "", d.m_Name)
        # an exact name beats a numbered copy; otherwise the first one wins
        if slot not in slot_mat or d.m_Name == slot:
            slot_mat[slot] = d
print("skin", SKIN, "materials:", len(slot_mat))
images, textures, tex_cache = [], [], {}

def embed(ptr, alpha_from_mask=False):
    """Embed a texture as an image once and return its glTF texture index (None if unreadable)."""
    try:
        t = ptr.read()
    except BaseException:
        return None
    key = (t.m_Name, alpha_from_mask)
    if key in tex_cache:
        return tex_cache[key]
    img = t.image.convert("RGBA")
    if max(img.size) > TEX_MAX:
        k = TEX_MAX / max(img.size)
        img = img.resize((max(1, int(img.width * k)), max(1, int(img.height * k))), Image.LANCZOS)
    buf = io.BytesIO()
    if alpha_from_mask:
        a = img.getchannel("A")
        mask = a if a.getextrema()[0] < 250 else img.getchannel("R")
        Image.merge("RGBA", (*Image.new("RGB", img.size, (255, 255, 255)).split(), mask)).save(buf, "PNG")
        mime = "image/png"
    else:
        img.convert("RGB").save(buf, "JPEG", quality=90)
        mime = "image/jpeg"
    images.append({"bufferView": add_view(buf.getvalue()), "mimeType": mime, "name": t.m_Name})
    textures.append({"source": len(images) - 1, "sampler": 0})
    tex_cache[key] = len(textures) - 1
    return tex_cache[key]

def build_material(slot):
    d = slot_mat.get(slot)
    pbr = {"baseColorFactor": [0.85, 0.7, 0.62, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.7}
    mat = {"name": slot, "pbrMetallicRoughness": pbr, "doubleSided": False}
    if d is None:
        return mat
    cols = {k: c for k, c in d.m_SavedProperties.m_Colors}
    c = cols.get("_Color")
    envs = {k: v for k, v in d.m_SavedProperties.m_TexEnvs}
    main, alpha = envs.get("_MainTex"), envs.get("_AlphaTex")
    pbr["baseColorFactor"] = [c.r, c.g, c.b, 1.0] if c else [1.0, 1.0, 1.0, 1.0]
    if alpha is not None and alpha.m_Texture.path_id:  # eyelashes: colour from the material, shape from the mask
        i = embed(alpha.m_Texture, alpha_from_mask=True)
        if i is not None:
            pbr["baseColorTexture"] = {"index": i}
            mat.update({"alphaMode": "MASK", "alphaCutoff": 0.5, "doubleSided": True})
    elif main is not None and main.m_Texture.path_id:
        i = embed(main.m_Texture)
        if i is not None:
            pbr["baseColorTexture"] = {"index": i}
    return mat

HIDDEN = ("Hidden", "Cornea", "Tear", "EyeReflection")
prims, materials = [], []
for m, polys in sorted(by_mat.items()):
    if mat_names[m] in HIDDEN:
        continue
    idx = []
    for v in polys:
        tris = [(v[0], v[1], v[2])] if len(v) == 3 else [(v[0], v[1], v[2]), (v[0], v[2], v[3])]
        for a, b, c in tris:
            idx += [a, c, b]  # mirrored axis flips the winding
    iv = add_view(struct.pack(f"<{len(idx)}I", *idx), 34963)
    materials.append(build_material(mat_names[m]))
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

# the joints VaM lets you move by hand (its FreeControllerV3 list, minus the nipples and genitals);
# fingers, face and tongue are driven by pose-control sliders there, so they are minor here
CONTROLS = ["hip", "pelvis", "abdomen", "abdomen2", "chest", "neck", "head", "lCollar", "rCollar", "lShldr", "rShldr",
            "lForeArm", "rForeArm", "lHand", "rHand", "lThigh", "rThigh", "lShin", "rShin", "lFoot", "rFoot", "lToe", "rToe"]
controls = [bone_node[b] for b in CONTROLS if b in bone_node]
print("controls:", len(controls), "of", len(need), "bones")

# ---- joint limits: the Limited ConfigurableJoint of each bone in the person prefab (VaM's own numbers)
tr, go_tr, go_name = {}, {}, {}
for o in penv.objects:
    if o.type.name == "Transform":
        t = o.read_typetree()
        tr[o.path_id] = t
        go_tr[t["m_GameObject"]["m_PathID"]] = o.path_id
    elif o.type.name == "GameObject":
        go_name[o.path_id] = o.read().m_Name

def qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz)

def qrot(q, v):
    x, y, z, w = q
    c1 = (y * v[2] - z * v[1], z * v[0] - x * v[2], x * v[1] - y * v[0])
    c2 = (y * c1[2] - z * c1[1], z * c1[0] - x * c1[2], x * c1[1] - y * c1[0])
    return tuple(v[i] + 2 * (w * c1[i] + c2[i]) for i in range(3))

def world_q(pid):
    t = tr[pid]
    q = tuple(t["m_LocalRotation"][k] for k in "xyzw")
    f = t["m_Father"]["m_PathID"]
    return qmul(world_q(f), q) if f in tr else q

def mat_to_quat(x, y, z):  # columns of a rotation matrix -> (x, y, z, w)
    m00, m10, m20, m01, m11, m21, m02, m12, m22 = x[0], x[1], x[2], y[0], y[1], y[2], z[0], z[1], z[2]
    tr_ = m00 + m11 + m22
    if tr_ > 0:
        s_ = (tr_ + 1.0) ** 0.5 * 2
        q = ((m21 - m12) / s_, (m02 - m20) / s_, (m10 - m01) / s_, 0.25 * s_)
    elif m00 > m11 and m00 > m22:
        s_ = (1.0 + m00 - m11 - m22) ** 0.5 * 2
        q = (0.25 * s_, (m01 + m10) / s_, (m02 + m20) / s_, (m21 - m12) / s_)
    elif m11 > m22:
        s_ = (1.0 + m11 - m00 - m22) ** 0.5 * 2
        q = ((m01 + m10) / s_, 0.25 * s_, (m12 + m21) / s_, (m02 - m20) / s_)
    else:
        s_ = (1.0 + m22 - m00 - m11) ** 0.5 * 2
        q = ((m02 + m20) / s_, (m12 + m21) / s_, 0.25 * s_, (m10 - m01) / s_)
    n = sum(c * c for c in q) ** 0.5 or 1.0
    return [c / n for c in q]

def axial(v):  # a rotation axis seen through the mirrored z axis: -M v (same angle)
    return (-v[0], -v[1], v[2])

limits = []
for o in penv.objects:
    if o.type.name != "ConfigurableJoint":
        continue
    t = o.read_typetree()
    name = go_name.get(t["m_GameObject"]["m_PathID"])
    if name not in bone_node or t["m_AngularXMotion"] != 1:
        continue
    q = world_q(go_tr[t["m_GameObject"]["m_PathID"]])
    ax = qrot(q, tuple(t["m_Axis"][k] for k in "xyz"))
    sec = qrot(q, tuple(t["m_SecondaryAxis"][k] for k in "xyz"))
    zz = (ax[1] * sec[2] - ax[2] * sec[1], ax[2] * sec[0] - ax[0] * sec[2], ax[0] * sec[1] - ax[1] * sec[0])
    frame = mat_to_quat(axial(ax), axial(sec), axial(zz))
    limits.append({"node": bone_node[name], "frame": frame, "twist": [-t["m_HighAngularXLimit"]["limit"], -t["m_LowAngularXLimit"]["limit"]],  # PhysX reports twist with the opposite sign
                   "swing": [t["m_AngularYLimit"]["limit"], t["m_AngularZLimit"]["limit"]]})
print("joint limits:", len(limits))

# ---- pose controls: VaM's slider list (fingers, toes, tongue, breasts...) that turn minor joints.
# Only the ones that are pure bone rotations (no corrective shapes) and touch minor joints; a
# value of 1 turns each joint by `x, y, z` degrees about its own DAZ axes (frame + order below).
menv = UnityPy.load(os.path.join(SA, "f_mb" if who == "female" else "m_mb"))
mgo = {o.path_id: o.read().m_Name for o in menv.objects if o.type.name == "GameObject"}
ROT = {13: 0, 14: 1, 15: 2}
minor_ok = set(bone_node) - set(CONTROLS)
raw = {}
for o in menv.objects:
    if o.type.name != "MonoBehaviour" or o.read(check_read=False).m_Script.read().m_ClassName != "DAZMorphSubBank":
        continue
    t = o.read_typetree()
    region = mgo.get(t["m_GameObject"]["m_PathID"], "")
    if not region.startswith("Pose Controls/") or region.startswith("Pose Controls/Hidden"):
        continue
    for m in t["_morphs"]:
        if m.get("numDeltas", 0) or not m["formulas"]:
            continue
        rot, links, ok = {}, [], True
        for f in m["formulas"]:
            tt = f["targetType"]
            if tt in ROT:
                if f["target"] not in minor_ok:
                    ok = False
                    break
                rot.setdefault(f["target"], [0.0, 0.0, 0.0])[ROT[tt]] += f["multiplier"]
            elif tt == 0:
                links.append((f["target"], f["multiplier"]))
            else:
                ok = False
                break
        if ok and (rot or links):
            raw[m["morphName"]] = {"label": m["displayName"].strip(), "group": region[len("Pose Controls/"):], "min": m["min"], "max": m["max"], "rot": rot, "links": links}
# links must point at controls we kept; a control that only links to nothing is dropped
changed = True
while changed:
    changed = False
    for k in list(raw):
        raw[k]["links"] = [l for l in raw[k]["links"] if l[0] in raw]
        if not raw[k]["rot"] and not raw[k]["links"]:
            del raw[k]
            changed = True
used = sorted({b for c in raw.values() for b in c["rot"]})
order_key = "_rotationOrder" if who == "female" else "_maleRotationOrder"
pc_bones = []
for b in used:
    d = pick(b)
    q = world_q(go_tr[d["m_GameObject"]["m_PathID"]])  # the bone's own axes, in Unity world space (not mirrored)
    pc_bones.append({"node": bone_node[b], "frame": list(q), "order": d[order_key]})
pc_controls = [{"name": k, "label": c["label"], "group": c["group"], "min": c["min"], "max": c["max"],
                "rot": [{"node": bone_node[b], "deg": v} for b, v in c["rot"].items()], "links": [{"to": a, "mult": m} for a, m in c["links"]]} for k, c in raw.items()]
print("pose controls:", len(pc_controls), "turning", len(pc_bones), "joints")

# ---- physics: the numbers VaM's bodies, controllers and colliders run on (from its prefab and
# its manager settings), so the editor's physics can use the same ones.
def world_scale(pid):
    t = tr[pid]
    f = t["m_Father"]["m_PathID"]
    s_ = max(abs(t["m_LocalScale"][k]) for k in "xyz")
    return s_ * (world_scale(f) if f in tr else 1.0)

def world_pos(pid):
    t = tr[pid]
    f = t["m_Father"]["m_PathID"]
    lp = tuple(t["m_LocalPosition"][k] for k in "xyz")
    if f not in tr:
        return lp
    fp, fq, fs = world_pos(f), world_q(f), world_scale(f)
    r = qrot(fq, tuple(c * fs for c in lp))
    return tuple(fp[i] + r[i] for i in range(3))

phys_bones = {}
for o in penv.objects:
    if o.type.name == "Rigidbody":
        t = o.read_typetree()
        name = go_name.get(t["m_GameObject"]["m_PathID"])
        if name in bone_node and name in CONTROLS:
            phys_bones.setdefault(name, {}).update({"mass": t["m_Mass"], "drag": t["m_Drag"], "angDrag": t["m_AngularDrag"]})
for o in penv.objects:
    if o.type.name == "ConfigurableJoint":
        t = o.read_typetree()
        name = go_name.get(t["m_GameObject"]["m_PathID"])
        if name in phys_bones and t["m_AngularXMotion"] == 1:  # the joint to the parent body: the muscle
            d = t["m_SlerpDrive"]
            phys_bones[name]["muscle"] = [d["positionSpring"], d["positionDamper"], d["maximumForce"]]

def bone_of(gid):
    pid = go_tr.get(gid)
    for _ in range(8):
        pid = tr[pid]["m_Father"]["m_PathID"] if pid in tr else None
        if pid not in tr:
            return None
        n = go_name.get(tr[pid]["m_GameObject"]["m_PathID"])
        if n in bone_node:
            return n, pid
    return None, None

other_sex = "MaleAutoCollider" if who == "female" else None
colliders = []
for o in penv.objects:
    if o.type.name != "CapsuleCollider":
        continue
    t = o.read_typetree()
    gid = t["m_GameObject"]["m_PathID"]
    name = go_name.get(gid, "")
    if not name.startswith("AutoCollider") or (other_sex and other_sex in name.replace("Female", "")) or not t["m_Enabled"]:
        continue
    bone, bone_pid = bone_of(gid)
    if bone not in CONTROLS:
        continue
    pid = go_tr[gid]
    q, wp_, sc = world_q(pid), world_pos(pid), world_scale(pid)
    axis = qrot(q, [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)][t["m_Direction"]])
    c = tuple(t["m_Center"][k] * sc for k in "xyz")
    centre = tuple(wp_[i] + qrot(q, c)[i] for i in range(3))
    r_ = t["m_Radius"] * sc
    half = max(t["m_Height"] * sc / 2 - r_, 0.0)
    colliders.append((bone, bone_pid, [centre[i] - axis[i] * half for i in range(3)], [centre[i] + axis[i] * half for i in range(3)], r_))

pc_colliders = []
for bone, bone_pid, a_, b_, r_ in colliders:
    org = world_pos(bone_pid)  # the bone's own origin, in the same prefab space the colliders sit in
    loc = lambda p: [p[0] - org[0], p[1] - org[1], -(p[2] - org[2])]  # our axes: z mirrored
    pc_colliders.append({"node": bone_node[bone], "a": loc(a_), "b": loc(b_), "radius": r_})
pc_bones_phys = [{"node": bone_node[n], **v} for n, v in phys_bones.items()]
print("physics: bodies", len(pc_bones_phys), "colliders", len(pc_colliders))
physics = {"step": 1.0 / 90.0, "iterations": 15, "friction": 0.6, "bounciness": 0.0, "maxVelocity": 10.0,
           "hold": {"posSpring": 1000.0, "posDamper": 50.0, "posMax": 10000.0, "rotSpring": 1000.0, "rotDamper": 50.0, "rotMax": 10000.0},
           "bones": pc_bones_phys, "colliders": pc_colliders}

gltf = {
    "asset": {"version": "2.0", "generator": "vam-base-export"},
    "extensionsUsed": ["VRM", "PC_controls", "PC_limits", "PC_pose_controls", "PC_physics", "PC_morph"],
    "extensions": {"PC_controls": {"nodes": controls}, "PC_limits": {"joints": limits}, "PC_pose_controls": {"bones": pc_bones, "controls": pc_controls}, "PC_physics": physics, "PC_morph": {"vertexBase": [base_of(j) for j in range(nv)], "graftStart": mesh["startGraftVertIndex"]}, "VRM": {"exporterVersion": "vam-base-export", "specVersion": "0.0", "meta": {"title": "VaM base " + who, "version": "1"}, "humanoid": {"humanBones": human}}},
    "scene": 0, "scenes": [{"nodes": [0, mesh_node]}],
    "nodes": nodes, "meshes": [{"name": who, "primitives": prims}],
    "skins": [{"joints": [bone_node[b] for b in need], "inverseBindMatrices": a_ibm, "skeleton": 0}],
    "materials": materials, "accessors": accessors,
    **({"images": images, "textures": textures, "samplers": [{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}]} if images else {}), "bufferViews": views, "buffers": [{"byteLength": len(blob)}],
}
js = json.dumps(gltf, separators=(",", ":")).encode()
js += b" " * (-len(js) % 4)
bin_ = bytes(blob) + b"\0" * (-len(blob) % 4)
glb = struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(js) + 8 + len(bin_)) + struct.pack("<I4s", len(js), b"JSON") + js + struct.pack("<I4s", len(bin_), b"BIN\0") + bin_
os.makedirs(OUT, exist_ok=True)
path = os.path.join(OUT, f"vam_base_{who}.vrm")
open(path, "wb").write(glb)
print("wrote", path, len(glb) // 1024, "KB; materials", len(materials), "joints", len(need))
