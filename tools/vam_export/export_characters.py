"""List VaM's built-in characters (`geometry.character` of a look names one) and write `characters.json`:
display name -> {bundle, uv, male}. Each character has its own UV set and default skin pictures, so a
look's own pictures only fit the model exported from that character's bundle:

    python export_characters.py [out_dir]                 # write characters.json
    python export_characters.py [out_dir] f_12 f_rti      # also export those bundles' models (export.py)
"""
import json
import os
import subprocess
import sys

import UnityPy

SA = r"F:\Virt-a-Mate_1.22.0.3\VaM_1.22.0.3\VaM_Data\StreamingAssets"
OUT = sys.argv[1] if len(sys.argv) > 1 else "."
want = sys.argv[2:]

# The characters the game lists (a_per holds their components) and the root object of each bundle.
env = UnityPy.load(os.path.join(SA, "a_per"))
go = {o.path_id: o.read().m_Name for o in env.objects if o.type.name == "GameObject"}
chars = {}
for o in env.objects:
    if o.type.name != "MonoBehaviour":
        continue
    try:
        cls = o.read(check_read=False).m_Script.read().m_ClassName
    except BaseException:
        continue
    if cls == "DAZCharacter":
        t = o.read_typetree()
        chars[go.get(t["m_GameObject"]["m_PathID"], "")] = {"name": t.get("displayName"), "uv": (t.get("UVname") or "").replace("UV: ", ""), "male": bool(t.get("isMale"))}

bundle_root = {}
for n in sorted(os.listdir(SA)):
    if (n.startswith("f_") or n.startswith("m_")) and not n.endswith("_mat") and not n.endswith(".manifest") and n not in ("f_mb", "m_mb"):
        try:
            e = UnityPy.load(os.path.join(SA, n))
        except BaseException:
            continue
        for o in e.objects:
            if o.type.name == "GameObject":
                bundle_root.setdefault(o.read().m_Name.lower(), n)

out = {}
for root, c in chars.items():
    b = bundle_root.get(root.lower())
    if b and c["name"]:
        out[c["name"]] = {"bundle": b, "uv": c["uv"], "male": c["male"]}
os.makedirs(OUT, exist_ok=True)
with open(os.path.join(OUT, "characters.json"), "w", encoding="utf-8") as f:
    json.dump(out, f, indent=1, ensure_ascii=False)
print(len(out), "characters;", len(chars) - len(out), "without a bundle")

here = os.path.dirname(os.path.abspath(__file__))
for b in want:
    who = "female" if b.startswith("f_") else "male"
    print("exporting", b)
    subprocess.run([sys.executable, os.path.join(here, "export.py"), who, OUT, b], check=True)
