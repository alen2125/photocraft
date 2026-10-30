"""Export the morphs built into a VaM base figure (the game's own `f_mb` / `m_mb` bundles) to a
`.pcmorphs` pack that the pose editor loads next to the figure's `.vrm`.

Pack format (little-endian): magic `PCMP1`, u32 count, then per morph: u32 header length, the header
as JSON (id, displayName, region, group, min, max, isPoseControl, genital, formulas), u32 offset count,
u32 compressed length, and the raw-deflate-compressed offsets (per offset: i32 base-vertex index and
three f32). Usage: python export_morphs.py female|male [out_dir]
"""
import json
import os
import struct
import sys
import zlib

import UnityPy

SA = r"F:\Virt-a-Mate_1.22.0.3\VaM_1.22.0.3\VaM_Data\StreamingAssets"
who = sys.argv[1]
out_dir = sys.argv[2] if len(sys.argv) > 2 else "."
bundle = "f_mb" if who == "female" else "m_mb"
KINDS = ["MorphValue", "BoneCenterX", "BoneCenterY", "BoneCenterZ", "OrientationX", "OrientationY", "OrientationZ", "GeneralScale", "ScaleX", "ScaleY", "ScaleZ", "MCM", "MCMMult", "RotationX", "RotationY", "RotationZ"]

env = UnityPy.load(os.path.join(SA, bundle))
go = {o.path_id: o.read().m_Name for o in env.objects if o.type.name == "GameObject"}
# Which bank a sub-bank belongs to: climb its GameObject's Transform parents to the object that
# carries a DAZMorphBank. The genital bank (`Genitalia-*`) numbers its vertices inside the graft.
transform_of, father_of, owner_of = {}, {}, {}
for o in env.objects:
    if o.type.name == "Transform":
        t = o.read_typetree()
        owner_of[o.path_id] = t["m_GameObject"]["m_PathID"]
        father_of[o.path_id] = t["m_Father"]["m_PathID"]
        transform_of[t["m_GameObject"]["m_PathID"]] = o.path_id
bank_geometry = {}
for o in env.objects:
    if o.type.name == "MonoBehaviour" and o.read(check_read=False).m_Script.read().m_ClassName == "DAZMorphBank":
        t = o.read_typetree()
        bank_geometry[t["m_GameObject"]["m_PathID"]] = t["geometryId"]

def geometry_of(go_id):
    tr = transform_of.get(go_id)
    for _ in range(64):
        if tr is None or tr == 0:
            return ""
        g = owner_of.get(tr)
        if g in bank_geometry:
            return bank_geometry[g]
        tr = father_of.get(tr)
    return ""

records = []
seen = set()
for o in env.objects:
    if o.type.name != "MonoBehaviour" or o.read(check_read=False).m_Script.read().m_ClassName != "DAZMorphSubBank":
        continue
    t = o.read_typetree()
    region = go.get(t["m_GameObject"]["m_PathID"], "")
    genital = geometry_of(t["m_GameObject"]["m_PathID"]).lower().startswith("genitalia")
    for m in t["_morphs"]:
        name = m["morphName"]
        if name in seen:
            continue
        seen.add(name)
        deltas = m["deltas"]
        if not deltas and not m["formulas"]:
            continue
        formulas = []
        for f in m["formulas"]:
            k = f["targetType"]
            if 0 <= k < len(KINDS):
                formulas.append({"targetType": KINDS[k], "target": f["target"], "multiplier": f["multiplier"]})
        header = {"id": name, "displayName": (m["displayName"] or name).strip(), "group": m["group"], "region": region, "min": m["min"], "max": m["max"],
                  "numDeltas": len(deltas), "isPoseControl": bool(m["isPoseControl"]), "genital": genital, "formulas": formulas}
        raw = bytearray()
        for d in deltas:
            v = d["delta"]
            raw += struct.pack("<i3f", d["vertex"], v["x"], v["y"], v["z"])
        c = zlib.compressobj(6, zlib.DEFLATED, -15)
        packed = c.compress(bytes(raw)) + c.flush()
        records.append((json.dumps(header).encode("utf-8"), len(deltas), packed))

path = os.path.join(out_dir, f"vam_base_{who}.pcmorphs")
with open(path, "wb") as f:
    f.write(b"PCMP1")
    f.write(struct.pack("<I", len(records)))
    for head, n, packed in records:
        f.write(struct.pack("<I", len(head)))
        f.write(head)
        f.write(struct.pack("<II", n, len(packed)))
        f.write(packed)
print("genital morphs:", sum(1 for r in records if b'"genital": true' in r[0]))
print(f"wrote {path}: {len(records)} morphs, {sum(r[1] for r in records)} offsets, {os.path.getsize(path) // 1024} KB")
