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

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Appearance {
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
