"""Export the genital blend mask of every character bundle (`DAZCharacterTextureControl.genitalBlendMaskTexture`)
as `genmask_<bundle>.png` (1024 px, grey) next to the figures. VaM blends the genital graft's picture
from the torso's and the genitals' by it (`autoBlendGenitalTextures`):

    python export_genmasks.py [out_dir]
"""
import os
import sys

import UnityPy

SA = r"F:\Virt-a-Mate_1.22.0.3\VaM_1.22.0.3\VaM_Data\StreamingAssets"
OUT = sys.argv[1] if len(sys.argv) > 1 else "."
SIZE = 1024

gen = UnityPy.load(os.path.join(SA, "p_gen_mat"))
textures = {o.path_id: o for o in gen.objects if o.type.name == "Texture2D"}
os.makedirs(OUT, exist_ok=True)
done = 0
for n in sorted(os.listdir(SA)):
    if not (n.startswith("f_") or n.startswith("m_")) or n.endswith("_mat") or n.endswith(".manifest") or n in ("f_mb", "m_mb"):
        continue
    try:
        env = UnityPy.load(os.path.join(SA, n))
    except BaseException:
        continue
    for o in env.objects:
        if o.type.name != "MonoBehaviour":
            continue
        try:
            cls = o.read(check_read=False).m_Script.read().m_ClassName
        except BaseException:
            continue
        if cls != "DAZCharacterTextureControl":
            continue
        ptr = o.read_typetree().get("genitalBlendMaskTexture") or {}
        tex = textures.get(ptr.get("m_PathID"))
        if tex is None:
            continue
        img = tex.read().image.convert("RGBA").getchannel("R").resize((SIZE, SIZE))
        img.save(os.path.join(OUT, f"genmask_{n}.png"))
        done += 1
        print(n, "->", tex.read().m_Name)
        break
print(done, "masks")
