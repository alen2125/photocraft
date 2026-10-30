//! A VaM appearance preset (`.vap`): JSON with a list of storables. The ones that matter for how a
//! figure looks are `geometry` (the morph values, and which clothing and hair are on), `skin` (the
//! tint) and `textures` (the skin images). Presets are hostile input: every field is optional and
//! malformed ones are skipped.

use serde_json::Value;

use crate::VamError;

/// Most morph values one preset may list.
pub const MAX_PRESET_MORPHS: usize = 20_000;

/// One morph value of a preset.
#[derive(Clone, Debug, PartialEq)]
pub struct VapMorph {
    /// `Package.N:/Custom/.../Name.vmi` or a bare name.
    pub uid: String,
    pub name: String,
    pub value: f32,
}

/// What a preset sets on one of the figure's smaller materials (eyes, teeth, tongue …). Empty text
/// means not set.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PartLook {
    /// The storable's id: `irises`, `sclera`, `lacrimals`, `teeth`, `tongue` or `mouth`.
    pub id: String,
    pub diffuse: String,
    pub specular: String,
    pub gloss_map: String,
    pub normal: String,
    pub decal: String,
    /// A mask whose red channel is the opacity (clothing with cut-outs).
    pub alpha: String,
    /// `Diffuse Color` as RGB.
    pub color: Option<[f32; 3]>,
    pub specular_intensity: Option<f32>,
    pub gloss: Option<f32>,
    pub bumpiness: Option<f32>,
}

/// The storable ids that are small materials of the figure.
pub const PART_IDS: [&str; 6] = ["irises", "sclera", "lacrimals", "teeth", "tongue", "mouth"];

/// The figure's materials a part's storable covers.
pub fn part_materials(id: &str) -> &'static [&'static str] {
    match id {
        "irises" => &["Irises"],
        "sclera" => &["Sclera"],
        "lacrimals" => &["Lacrimals"],
        "teeth" => &["Teeth", "Gums"],
        "tongue" => &["Tongue"],
        "mouth" => &["InnerMouth"],
        _ => &[],
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Appearance {
    /// Eyes, teeth, tongue … : what the preset sets on each.
    pub parts: Vec<PartLook>,
    /// What the preset sets on the materials of worn items, by storable id (`Creator:ItemMaterialName`).
    pub item_parts: Vec<PartLook>,
    /// `Female Custom`, `Male Custom` or the name of a built-in character.
    pub character: String,
    pub morphs: Vec<VapMorph>,
    /// The skin images (`faceDiffuseUrl` …) that are set, as (key, reference).
    pub textures: Vec<(String, String)>,
    /// The skin tint, as RGB 0–1 (the preset stores it as hue, saturation, value).
    pub skin_color: Option<[f32; 3]>,
    /// The skin material's numbers: `Specular Intensity`, `Gloss`, `Diffuse Bumpiness`, `Specular Bumpiness`.
    pub specular_intensity: Option<f32>,
    pub gloss: Option<f32>,
    pub diffuse_bumpiness: Option<f32>,
    pub specular_bumpiness: Option<f32>,
    /// Clothing and hair items the preset lists, with whether each is on.
    pub clothing: Vec<(String, bool)>,
    pub hair: Vec<(String, bool)>,
}

fn flag(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn number(v: Option<&Value>) -> Option<f32> {
    let f = match v? {
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        other => other.as_f64()?,
    };
    f.is_finite().then_some(f as f32)
}

/// Hue, saturation and value (each 0–1) to RGB.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let fin = |x: f32, d: f32| if x.is_finite() { x } else { d };
    let (h, s, v) = (fin(h, 0.0).rem_euclid(1.0) * 6.0, fin(s, 0.0).clamp(0.0, 1.0), fin(v, 1.0).clamp(0.0, 1.0));
    let (i, f) = (h.floor(), h - h.floor());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as u32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// What a material storable (an eye, a piece of clothing) sets.
fn part_look(id: &str, s: &Value) -> PartLook {
    let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let color = s.get("Diffuse Color").filter(|c| c.is_object()).map(|c| hsv_to_rgb(number(c.get("h")).unwrap_or(0.0), number(c.get("s")).unwrap_or(0.0), number(c.get("v")).unwrap_or(1.0)));
    let bump = match (number(s.get("Diffuse Bumpiness")), number(s.get("Specular Bumpiness"))) {
        (Some(a), Some(b)) => Some((a + b) / 2.0),
        (a, b) => a.or(b),
    };
    PartLook {
        id: id.to_string(),
        diffuse: text("customTexture_MainTex"),
        specular: text("customTexture_SpecTex"),
        gloss_map: text("customTexture_GlossTex"),
        normal: text("customTexture_BumpMap"),
        decal: text("customTexture_DecalTex"),
        alpha: text("customTexture_AlphaTex"),
        color,
        specular_intensity: number(s.get("Specular Intensity")),
        gloss: number(s.get("Gloss")),
        bumpiness: bump,
    }
}

/// The default look of each material of a clothing or hair item, from its `.vaj`: the storables
/// named `<uid>Material<material>`. Returns the material name with what is set on it.
pub fn parse_item_materials(vaj: &[u8], uid: &str) -> Vec<(String, PartLook)> {
    let body = vaj.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(vaj);
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return Vec::new() };
    let prefix = format!("{uid}Material");
    v.get("storables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s.get("id").and_then(Value::as_str)?;
            let material = id.strip_prefix(&prefix)?;
            Some((material.to_string(), part_look(id, s)))
        })
        .collect()
}

/// The wrap settings of an item's `.vaj`: how far its surface sits off the body, and how much its
/// own thickness counts (the `<uid>WrapControl` storable).
pub fn parse_wrap_params(vaj: &[u8], uid: &str) -> (f32, f32) {
    let body = vaj.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(vaj);
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return (0.0, 0.0) };
    let id = format!("{uid}WrapControl");
    let s = v.get("storables").and_then(Value::as_array).into_iter().flatten().find(|s| s.get("id").and_then(Value::as_str) == Some(id.as_str()));
    (s.and_then(|s| number(s.get("surfaceOffset"))).unwrap_or(0.0), s.and_then(|s| number(s.get("additionalThicknessMultiplier"))).unwrap_or(0.0))
}

/// A reference in an item's files. `context` is the package the item is in (`Pkg.1`) or the item's
/// own `.vam` reference (`Pkg.1:/Custom/Clothing/Female/Maker/Thing/Thing.vam`), which also lets
/// paths relative to the item (`./Textures/a.png`, `Tex/a.png`) be found. Full references stay.
pub fn item_reference(reference: &str, context: &str) -> String {
    if reference.is_empty() {
        return String::new();
    }
    let folder_of = |path: &str| path.rsplit_once('/').map_or("", |(d, _)| d).to_string();
    let (package, dir) = match context.split_once(":/") {
        Some((p, path)) => (p, folder_of(path)),
        // A loose item's own file: no package, its folder is the base.
        None if context.to_ascii_lowercase().ends_with(".vam") => ("", folder_of(context)),
        None => (context, String::new()),
    };
    let wrap = |path: &str| if package.is_empty() { path.to_string() } else { format!("{package}:/{path}") };
    if let Some(rest) = reference.strip_prefix("SELF:/") {
        return wrap(rest);
    }
    if reference.contains(":/") {
        return reference.to_string();
    }
    if reference.starts_with("Custom/") || reference.starts_with("Saves/") {
        return wrap(reference);
    }
    // Relative to the item's folder; "./" and "../" are resolved.
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    let unified = reference.replace('\\', "/");
    for seg in unified.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    wrap(&parts.join("/"))
}

/// A reference in the preset: `SELF:/path` means the package the preset itself is in (or the game
/// folder, for a loose file).
pub fn localize(reference: &str, package: &str) -> String {
    match reference.strip_prefix("SELF:/") {
        Some(rest) if package.is_empty() => rest.to_string(),
        Some(rest) => format!("{package}:/{rest}"),
        None => reference.to_string(),
    }
}

impl Appearance {
    /// Replaces `SELF:` in every reference with the package the preset came from.
    pub fn localize(&mut self, package: &str) {
        let fix = |s: &mut String| *s = localize(s, package);
        for m in &mut self.morphs {
            fix(&mut m.uid);
        }
        for (_, u) in &mut self.textures {
            fix(u);
        }
        for p in &mut self.parts {
            for s in [&mut p.diffuse, &mut p.specular, &mut p.gloss_map, &mut p.normal, &mut p.decal, &mut p.alpha] {
                fix(s);
            }
        }
        for p in &mut self.item_parts {
            for s in [&mut p.diffuse, &mut p.specular, &mut p.gloss_map, &mut p.normal, &mut p.decal, &mut p.alpha] {
                fix(s);
            }
        }
        for (id, _) in self.clothing.iter_mut().chain(self.hair.iter_mut()) {
            fix(id);
        }
    }
}

/// The storables of a preset (or of a person in a scene), read.
pub fn parse_storables(storables: &[Value]) -> Appearance {
    let mut a = Appearance::default();
    for s in storables {
        match s.get("id").and_then(Value::as_str) {
            Some("geometry") => {
                a.character = s.get("character").and_then(Value::as_str).unwrap_or_default().to_string();
                for m in s.get("morphs").and_then(Value::as_array).into_iter().flatten().take(MAX_PRESET_MORPHS) {
                    let (Some(uid), Some(value)) = (m.get("uid").and_then(Value::as_str), number(m.get("value"))) else { continue };
                    a.morphs.push(VapMorph { uid: uid.to_string(), name: m.get("name").and_then(Value::as_str).unwrap_or(uid).to_string(), value });
                }
                let items = |key: &str| -> Vec<(String, bool)> {
                    s.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(|c| Some((c.get("id").and_then(Value::as_str)?.to_string(), flag(c.get("enabled"))))).collect()
                };
                a.clothing = items("clothing");
                a.hair = items("hair");
            }
            Some("textures") => {
                if let Some(o) = s.as_object() {
                    for (k, v) in o {
                        if k.ends_with("Url") && let Some(url) = v.as_str().filter(|u| !u.is_empty()) {
                            a.textures.push((k.clone(), url.to_string()));
                        }
                    }
                }
            }
            Some("skin") => {
                a.specular_intensity = number(s.get("Specular Intensity"));
                a.gloss = number(s.get("Gloss"));
                a.diffuse_bumpiness = number(s.get("Diffuse Bumpiness"));
                a.specular_bumpiness = number(s.get("Specular Bumpiness"));
                if let Some(c) = s.get("Skin Color").filter(|c| c.is_object()) {
                    a.skin_color = Some(hsv_to_rgb(number(c.get("h")).unwrap_or(0.0), number(c.get("s")).unwrap_or(0.0), number(c.get("v")).unwrap_or(1.0)));
                }
            }
            Some(id) if PART_IDS.contains(&id) => a.parts.push(part_look(id, s)),
            Some(id) if id.contains(':') && id.contains("Material") => a.item_parts.push(part_look(id, s)),
            _ => {}
        }
    }
    a
}

/// Reads a `.vap` file.
pub fn parse_vap(bytes: &[u8]) -> Result<Appearance, VamError> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let v: Value = serde_json::from_slice(body).map_err(|e| VamError::Morph(format!("not a readable appearance preset: {e}")))?;
    let storables = v.get("storables").and_then(Value::as_array).ok_or_else(|| VamError::Morph("the preset has no storables".into()))?;
    Ok(parse_storables(storables))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_gives_its_morphs_textures_tint_and_items() {
        let json = br#"{ "setUnlistedParamsToDefault":"true", "storables":[
            { "id":"geometry", "character":"Female Custom", "clothing":[{"id":"Tank Top","enabled":"true"},{"id":"Shorts","enabled":"false"}], "hair":[{"id":"Hair A","enabled":"true"}],
              "morphs":[ {"uid":"Pkg.2:/Custom/Atom/Person/Morphs/female/x/Wide.vmi","name":"Wide","value":"0.65"}, {"uid":"Bare","name":"Bare","value":-0.2}, {"uid":"Broken","value":"nope"}, {"name":"NoUid","value":"1"} ] },
            { "id":"textures", "faceDiffuseUrl":"Pkg.2:/Custom/Atom/Person/Textures/f.jpg", "torsoDiffuseUrl":"", "limbsDiffuseUrl":"x.png", "autoBlend":"true" },
            { "id":"skin", "Skin Color":{"h":"0.5","s":"1","v":"1"}, "Specular Intensity":"3.5", "Gloss":"5", "Diffuse Bumpiness":"0.2" },
            { "id":"unrelated" } ] }"#;
        let a = parse_vap(json).unwrap();
        assert_eq!(a.character, "Female Custom");
        assert_eq!(a.morphs.len(), 2, "the ones with no uid or a bad value are skipped");
        assert_eq!((a.morphs[0].name.as_str(), a.morphs[0].value, a.morphs[1].value), ("Wide", 0.65, -0.2));
        assert_eq!(a.textures, vec![("faceDiffuseUrl".into(), "Pkg.2:/Custom/Atom/Person/Textures/f.jpg".into()), ("limbsDiffuseUrl".into(), "x.png".into())]);
        let c = a.skin_color.unwrap();
        assert!(c[0].abs() < 1e-6 && (c[1] - 1.0).abs() < 1e-6 && (c[2] - 1.0).abs() < 1e-6, "hue 0.5 is cyan: {c:?}");
        assert_eq!((a.specular_intensity, a.gloss, a.diffuse_bumpiness, a.specular_bumpiness), (Some(3.5), Some(5.0), Some(0.2), None));
        assert_eq!(a.clothing, vec![("Tank Top".into(), true), ("Shorts".into(), false)]);
        assert_eq!(a.hair, vec![("Hair A".into(), true)]);
    }

    #[test]
    fn self_references_name_the_presets_own_package_and_eyes_and_teeth_are_read() {
        let json = br#"{"storables":[
            {"id":"geometry","morphs":[{"uid":"SELF:/Custom/Atom/Person/Morphs/female/x/Wide.vmi","name":"Wide","value":"1"}]},
            {"id":"textures","faceDecalUrl":"SELF:/Custom/Atom/Person/Textures/blush.png"},
            {"id":"irises","customTexture_MainTex":"SELF:/Custom/eye.png","customTexture_BumpMap":"Other.1:/n.png","Diffuse Color":{"h":"0","s":"0","v":"0.5"},"Gloss":"3"},
            {"id":"tongue"} ] }"#;
        let mut a = parse_vap(json).unwrap();
        a.localize("Maker.Looks.3");
        assert_eq!(a.morphs[0].uid, "Maker.Looks.3:/Custom/Atom/Person/Morphs/female/x/Wide.vmi");
        assert_eq!(a.textures[0].1, "Maker.Looks.3:/Custom/Atom/Person/Textures/blush.png");
        let eye = &a.parts[0];
        assert_eq!((eye.id.as_str(), eye.diffuse.as_str(), eye.normal.as_str(), eye.gloss), ("irises", "Maker.Looks.3:/Custom/eye.png", "Other.1:/n.png", Some(3.0)));
        assert_eq!(eye.color, Some([0.5, 0.5, 0.5]));
        assert_eq!(a.parts.len(), 2);
        let mut loose = parse_vap(json).unwrap();
        loose.localize("");
        assert_eq!(loose.parts[0].diffuse, "Custom/eye.png", "a loose preset's SELF is the game folder");
        assert_eq!(localize("Pkg.1:/x.png", "Maker.1"), "Pkg.1:/x.png");
    }

    #[test]
    fn item_references_resolve_against_the_items_folder() {
        let item = "Maker.Items.2:/Custom/Clothing/Female/Maker/Thing/Thing.vam";
        assert_eq!(item_reference("./Textures/a.png", item), "Maker.Items.2:/Custom/Clothing/Female/Maker/Thing/Textures/a.png");
        assert_eq!(item_reference("../Shared/b.png", item), "Maker.Items.2:/Custom/Clothing/Female/Maker/Shared/b.png");
        assert_eq!(item_reference("Custom/Atom/Person/Textures/c.png", item), "Maker.Items.2:/Custom/Atom/Person/Textures/c.png");
        assert_eq!(item_reference("SELF:/Custom/x.png", item), "Maker.Items.2:/Custom/x.png");
        assert_eq!(item_reference("Other.1:/y.png", item), "Other.1:/y.png");
        assert_eq!(item_reference("./t.png", "Custom/Clothing/Female/L/L.vam"), "Custom/Clothing/Female/L/t.png", "a loose item has no package");
        assert_eq!(item_reference("", item), "");
        assert_eq!(item_reference("z.png", "Maker.Items.2"), "Maker.Items.2:/z.png");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for bad in [&b""[..], b"[]", b"{}", b"{\"storables\":5}", b"\xff\xfe"] {
            assert!(parse_vap(bad).is_err());
        }
        assert!(parse_vap(b"{\"storables\":[5,null,{\"id\":3}]}").unwrap().morphs.is_empty());
        let white = hsv_to_rgb(0.3, 0.0, 1.0);
        assert_eq!(white, [1.0, 1.0, 1.0]);
        assert!(hsv_to_rgb(f32::NAN, 2.0, -1.0).iter().all(|c| c.is_finite()));
    }
}
