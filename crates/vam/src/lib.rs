//! A Virt-A-Mate install, read where it lies. The user names the game folder; this finds the
//! morph files in it (loose ones under `Custom/Atom/Person/Morphs` and those inside the zip
//! packages in `AddonPackages`), and files named by a look or scene (`Package.3:/Custom/...`).
//! Nothing is copied anywhere. Files are hostile input: sizes are capped and nothing panics.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod dazmesh;
mod hair;
mod vap;
mod zipfile;

use std::path::{Path, PathBuf};

use photocraft_model3d::morph::{parse_morph, parse_vmi, Morph, MorphInfo};

pub use hair::{parse_hair, HairStrands};
pub use dazmesh::{parse_dazmesh, DazMesh, Poly, WrapVert};
pub use vap::{hair_look, hsv_to_rgb, parse_item_hair, HairLook, item_reference, localize, parse_item_materials, parse_wrap_params, parse_storables, parse_vap, part_materials, Appearance, PartLook, VapMorph, PART_IDS};
pub use zipfile::{ZipEntry, ZipFile};

/// Largest single file read from the install (a morph, a texture).
pub const MAX_FILE_BYTES: usize = 256 * 1024 * 1024;
/// Most morph files indexed in all.
pub const MAX_MORPH_FILES: usize = 200_000;
/// Most packages looked into.
pub const MAX_PACKAGES: usize = 20_000;

const MORPH_DIR: &str = "Custom/Atom/Person/Morphs";
const LOOK_DIR: &str = "Custom/Atom/Person/Appearance";
const TEXTURE_DIR: &str = "Custom/Atom/Person/Textures";
const CLOTHING_DIR: &str = "Custom/Clothing";
const HAIR_DIR: &str = "Custom/Hair";
/// Most clothing and hair items indexed in all.
pub const MAX_ITEMS: usize = 100_000;
/// Most appearance presets indexed in all.
pub const MAX_PRESETS: usize = 100_000;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VamError {
    #[error("{0}: {1}")]
    Io(String, String),
    #[error("{0}")]
    Zip(String),
    #[error("{0} does not look like a Virt-A-Mate folder (no Custom or AddonPackages inside)")]
    NotAnInstall(String),
    #[error("{0}")]
    Morph(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gender {
    Female,
    Male,
}

/// Where a morph's files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MorphSource {
    /// A `.vmi` on disk (its `.vmb` sits beside it).
    Loose(PathBuf),
    /// Entries of a package: its index in [`GameLibrary::packages`] and the entry indices.
    Packaged { package: usize, vmi: usize, vmb: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorphFile {
    /// The file name without its extension (what looks refer to a morph by).
    pub stem: String,
    pub gender: Gender,
    /// The folder under `Morphs/<gender>/` the file is in (empty at the top).
    pub folder: String,
    /// A genital morph: its vertices are numbered from 0 within the figure's genital graft.
    pub genital: bool,
    /// The package's name (`Creator.Name.3`) or empty for a loose file.
    pub package: String,
    pub source: MorphSource,
}

/// An appearance preset (`.vap`) in the install or in a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    /// The file name without its extension.
    pub name: String,
    /// The package's name or empty for a loose file.
    pub package: String,
    /// What [`GameLibrary::read_file`] takes: a path under the game folder or `Package.N:/path`.
    pub reference: String,
    /// The picture beside it (same name, `.jpg` or `.png`), by the same kind of reference.
    pub thumbnail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Clothing,
    Hair,
}

/// A clothing or hair item (`.vam`, with its mesh `.vab` and settings `.vaj` beside it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub kind: ItemKind,
    /// Which figure it is for; `None` for items made for both.
    pub gender: Option<Gender>,
    /// The folders it is in under the gender folder.
    pub folder: String,
    /// The package's name or empty for a loose file.
    pub package: String,
    /// The `.vam` file, as [`GameLibrary::read_file`] takes it (the `.vab` and `.vaj` swap the extension).
    pub reference: String,
    pub thumbnail: Option<String>,
}

/// Whether a texture file is an iris picture to pick from: a colour picture in a folder of eyes
/// (VaM keeps them in `Textures/Eyes`). Make-up and face pictures that only have "eye" in their
/// names (`hearts+red eye+lips+blush.png`) are not: they lie on the face, not on the iris.
fn is_eye_picture(path: &str) -> bool {
    let p = path.to_ascii_lowercase().replace('\\', "/");
    let picture = p.ends_with(".png") || p.ends_with(".jpg") || p.ends_with(".jpeg");
    let mut parts: Vec<&str> = p.split('/').collect();
    let name = parts.pop().unwrap_or("");
    let in_eye_folder = parts.iter().any(|d| matches!(*d, "eye" | "eyes" | "iris" | "irises" | "irides")) && parts.iter().any(|d| *d == "textures" || *d == "person" || *d == "atom");
    picture && in_eye_folder && !["normal", "spec", "gloss", "bump", "alpha", "mask", "_n.", "_s.", "_g.", "_a.", "_nrm", "_bump", "sclera", "lash", "lip", "blush", "brow", "makeup", "shadow", "liner", "reflection", "+"].iter().any(|k| name.contains(k))
}

/// A reference with another extension.
pub fn with_extension(reference: &str, ext: &str) -> String {
    match reference.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.{ext}"),
        None => format!("{reference}.{ext}"),
    }
}

/// `Female/Maker/Style/Name.vam` (under `Custom/Clothing`) into gender and folder.
fn split_item_path(rel: &str) -> Option<(Option<Gender>, String)> {
    let rel = rel.replace('\\', "/");
    let mut parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    parts.pop()?;
    let first = parts.first().map(|p| p.to_ascii_lowercase());
    let (gender, rest) = match first.as_deref() {
        Some("female") => (Some(Gender::Female), &parts[1..]),
        Some("male") => (Some(Gender::Male), &parts[1..]),
        Some("neutral") => (None, &parts[1..]),
        _ => (None, &parts[..]),
    };
    Some((gender, rest.join("/")))
}

#[derive(Debug)]
pub struct GameLibrary {
    root: PathBuf,
    pub packages: Vec<(String, ZipFile)>,
    pub morphs: Vec<MorphFile>,
    /// Appearance presets, loose ones first, then those in packages.
    pub presets: Vec<Preset>,
    /// Clothing and hair items, loose ones first, then those in packages.
    pub items: Vec<Item>,
    /// Eye pictures (iris textures), as presets: the picture is its own thumbnail.
    pub eyes: Vec<Preset>,
    /// Packages that could not be read, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
    /// Morph files found outside the female/male (and genitalia) folders: listed, never applied.
    pub other_morphs: usize,
    /// Every picture in the install by lower-case file name, as references (see [`GameLibrary::read_file`]).
    files_by_name: std::collections::HashMap<String, Vec<String>>,
    /// How many files were found by name because their own reference did not open.
    matched_by_name: std::sync::atomic::AtomicUsize,
}

/// Most pictures indexed by name.
pub const MAX_NAMED_FILES: usize = 2_000_000;

fn is_picture(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".tif", ".tiff", ".tga", ".bmp"].iter().any(|e| n.ends_with(e))
}

/// The lower-case file name of a reference or path.
fn base_name(reference: &str) -> String {
    reference.rsplit(['/', '\\']).next().unwrap_or(reference).to_ascii_lowercase()
}

/// A package's version number (`Maker.Pack.12` is 12; none is 0).
fn package_version(reference: &str) -> u32 {
    reference.split_once(":/").map_or(reference, |(p, _)| p).rsplit('.').next().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// The materials of a VaM figure that each skin texture region (`face`, `torso`, `limbs`,
/// `genitals`) covers, by material name (from the game's own character texture control).
pub fn skin_region_materials(gender: Gender, region: &str) -> &'static [&'static str] {
    match (region, gender) {
        ("face", _) => &["Nostrils", "Lips", "Face"],
        ("torso", _) => &["Head", "Ears", "Neck", "Hips", "Torso", "Nipples", "Hidden"],
        ("limbs", _) => &["Legs", "Toenails", "Fingernails", "Hands", "Shoulders", "Forearms", "Feet"],
        ("genitals", Gender::Female) => &["defaultMat"],
        ("genitals", Gender::Male) => &["Genitalia"],
        _ => &[],
    }
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, cap: usize) {
    if depth > 24 || out.len() >= cap {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, depth + 1, out, cap);
        } else {
            out.push(p);
            if out.len() >= cap {
                return;
            }
        }
    }
}

fn is_ext(p: &Path, ext: &str) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// `female/AUTO/Name.vmi` (a path under `Morphs`) into gender, whether it is a genital morph,
/// folder and stem. `female_genitalia` and `male_genitalia` hold the genital morphs.
fn split_morph_path(rel: &str) -> Option<(Gender, bool, String, String)> {
    let rel = rel.replace('\\', "/");
    let mut parts = rel.split('/').filter(|p| !p.is_empty());
    let (gender, genital) = match parts.next()?.to_ascii_lowercase().as_str() {
        "female" => (Gender::Female, false),
        "male" => (Gender::Male, false),
        "female_genitalia" => (Gender::Female, true),
        "male_genitalia" => (Gender::Male, true),
        _ => return None,
    };
    let rest: Vec<&str> = parts.collect();
    let (file, folders) = rest.split_last()?;
    let stem = Path::new(file).file_stem()?.to_str()?.to_string();
    Some((gender, genital, folders.join("/"), stem))
}

impl GameLibrary {
    /// Opens the install at `root` and indexes its morphs.
    pub fn open(root: &Path) -> Result<Self, VamError> {
        if !root.join("Custom").is_dir() && !root.join("AddonPackages").is_dir() {
            return Err(VamError::NotAnInstall(root.display().to_string()));
        }
        let mut lib = GameLibrary { root: root.to_path_buf(), packages: Vec::new(), morphs: Vec::new(), presets: Vec::new(), items: Vec::new(), eyes: Vec::new(), skipped: Vec::new(), other_morphs: 0, files_by_name: Default::default(), matched_by_name: Default::default() };
        // Loose morphs.
        let base = root.join(MORPH_DIR);
        let mut files = Vec::new();
        walk(&base, 0, &mut files, MAX_MORPH_FILES * 2);
        for p in files.iter().filter(|p| is_ext(p, "vmi")) {
            if lib.morphs.len() >= MAX_MORPH_FILES {
                break;
            }
            let Ok(rel) = p.strip_prefix(&base) else { continue };
            let Some((gender, genital, folder, stem)) = split_morph_path(&rel.to_string_lossy()) else {
                lib.other_morphs += 1;
                continue;
            };
            if p.with_extension("vmb").is_file() {
                lib.morphs.push(MorphFile { stem, gender, folder, genital, package: String::new(), source: MorphSource::Loose(p.clone()) });
            }
        }
        // Loose presets.
        let mut looks = Vec::new();
        walk(&root.join(LOOK_DIR), 0, &mut looks, MAX_PRESETS * 3);
        for p in looks.iter().filter(|p| is_ext(p, "vap")) {
            if lib.presets.len() >= MAX_PRESETS {
                break;
            }
            let Some(name) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            let Ok(rel) = p.strip_prefix(root) else { continue };
            let thumbnail = ["jpg", "jpeg", "png"].iter().map(|e| p.with_extension(e)).find(|t| t.is_file()).and_then(|t| t.strip_prefix(root).ok().map(|r| r.to_string_lossy().replace('\\', "/")));
            lib.presets.push(Preset { name: name.to_string(), package: String::new(), reference: rel.to_string_lossy().replace('\\', "/"), thumbnail });
        }
        // Every loose picture, by name, for finding a file whose own reference does not open.
        for dir in ["Custom", "Saves"] {
            let mut pics = Vec::new();
            walk(&root.join(dir), 0, &mut pics, MAX_NAMED_FILES);
            for p in pics.iter().filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(is_picture)) {
                let Ok(rel) = p.strip_prefix(root) else { continue };
                let reference = rel.to_string_lossy().replace('\\', "/");
                lib.files_by_name.entry(base_name(&reference)).or_default().push(reference);
            }
        }
        // Loose eye pictures.
        let mut textures = Vec::new();
        walk(&root.join(TEXTURE_DIR), 0, &mut textures, MAX_ITEMS * 4);
        for p in textures.iter().filter(|p| p.strip_prefix(root).is_ok_and(|r| is_eye_picture(&r.to_string_lossy().replace('\\', "/")))) {
            if lib.eyes.len() >= MAX_ITEMS {
                break;
            }
            let (Some(name), Ok(rel)) = (p.file_stem().and_then(|s| s.to_str()), p.strip_prefix(root)) else { continue };
            let reference = rel.to_string_lossy().replace('\\', "/");
            lib.eyes.push(Preset { name: name.to_string(), package: String::new(), reference: reference.clone(), thumbnail: Some(reference) });
        }
        // Loose clothing and hair.
        for (dir, kind) in [(CLOTHING_DIR, ItemKind::Clothing), (HAIR_DIR, ItemKind::Hair)] {
            let mut files = Vec::new();
            walk(&root.join(dir), 0, &mut files, MAX_ITEMS * 4);
            let base = root.join(dir);
            for p in files.iter().filter(|p| is_ext(p, "vam") && p.with_extension("vab").is_file()) {
                if lib.items.len() >= MAX_ITEMS {
                    break;
                }
                let (Some(name), Ok(rel), Ok(from_root)) = (p.file_stem().and_then(|s| s.to_str()), p.strip_prefix(&base), p.strip_prefix(root)) else { continue };
                let Some((gender, folder)) = split_item_path(&rel.to_string_lossy()) else { continue };
                let thumbnail = ["jpg", "jpeg", "png"].iter().map(|e| p.with_extension(e)).find(|t| t.is_file()).and_then(|t| t.strip_prefix(root).ok().map(|r| r.to_string_lossy().replace('\\', "/")));
                lib.items.push(Item { name: name.to_string(), kind, gender, folder, package: String::new(), reference: from_root.to_string_lossy().replace('\\', "/"), thumbnail });
            }
        }
        // Packages.
        let mut pkgs = Vec::new();
        walk(&root.join("AddonPackages"), 0, &mut pkgs, MAX_PACKAGES);
        for p in pkgs.into_iter().filter(|p| is_ext(p, "var")) {
            match ZipFile::open(&p) {
                Ok(z) => {
                    let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
                    let index = lib.packages.len();
                    let prefix = format!("{MORPH_DIR}/");
                    let look_prefix = format!("{LOOK_DIR}/");
                    for e in z.entries.iter().filter(|e| is_picture(&e.name)) {
                        if lib.files_by_name.len() < MAX_NAMED_FILES {
                            lib.files_by_name.entry(base_name(&e.name)).or_default().push(format!("{name}:/{}", e.name));
                        }
                    }
                    for e in z.entries.iter().filter(|e| e.name.starts_with(&format!("{TEXTURE_DIR}/")) && is_eye_picture(&e.name)) {
                        if lib.eyes.len() >= MAX_ITEMS {
                            break;
                        }
                        let Some(stem) = Path::new(&e.name).file_stem().and_then(|s| s.to_str()) else { continue };
                        let reference = format!("{name}:/{}", e.name);
                        lib.eyes.push(Preset { name: stem.to_string(), package: name.clone(), reference: reference.clone(), thumbnail: Some(reference) });
                    }
                    for (dir, kind) in [(CLOTHING_DIR, ItemKind::Clothing), (HAIR_DIR, ItemKind::Hair)] {
                        let item_prefix = format!("{dir}/");
                        for e in z.entries.iter().filter(|e| e.name.starts_with(&item_prefix) && is_ext(Path::new(&e.name), "vam")) {
                            let base = e.name.get(..e.name.len() - 4).unwrap_or_default();
                            if lib.items.len() >= MAX_ITEMS || !z.entries.iter().any(|x| x.name == format!("{base}.vab")) {
                                continue;
                            }
                            let (Some(name), Some(rel)) = (Path::new(&e.name).file_stem().and_then(|s| s.to_str()), e.name.strip_prefix(&item_prefix)) else { continue };
                            let Some((gender, folder)) = split_item_path(rel) else { continue };
                            let thumbnail = ["jpg", "jpeg", "png"].iter().map(|x| format!("{base}.{x}")).find(|t| z.entries.iter().any(|y| y.name == *t)).map(|t| format!("{name_pkg}:/{t}", name_pkg = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default()));
                            lib.items.push(Item { name: name.to_string(), kind, gender, folder, package: p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string(), reference: format!("{}:/{}", p.file_stem().and_then(|s| s.to_str()).unwrap_or_default(), e.name), thumbnail });
                        }
                    }
                    let names: std::collections::HashSet<&str> = z.entries.iter().map(|e| e.name.as_str()).collect();
                    for e in z.entries.iter().filter(|e| e.name.starts_with(&look_prefix) && is_ext(Path::new(&e.name), "vap")) {
                        if lib.presets.len() >= MAX_PRESETS {
                            break;
                        }
                        let Some(stem) = Path::new(&e.name).file_stem().and_then(|s| s.to_str()) else { continue };
                        let base = e.name.get(..e.name.len() - 4).unwrap_or_default();
                        let thumbnail = ["jpg", "jpeg", "png"].iter().map(|x| format!("{base}.{x}")).find(|t| names.contains(t.as_str())).map(|t| format!("{name}:/{t}"));
                        lib.presets.push(Preset { name: stem.to_string(), package: name.clone(), reference: format!("{name}:/{}", e.name), thumbnail });
                    }
                    for (i, e) in z.entries.iter().enumerate() {
                        let Some(rel) = e.name.strip_prefix(&prefix).filter(|_| is_ext(Path::new(&e.name), "vmi")) else { continue };
                        let Some((gender, genital, folder, stem)) = split_morph_path(rel) else {
                            lib.other_morphs += 1;
                            continue;
                        };
                        let vmb_name = format!("{}.vmb", e.name.trim_end_matches(".vmi").trim_end_matches(".VMI"));
                        let Some(vmb) = z.entries.iter().position(|x| x.name == vmb_name) else { continue };
                        if lib.morphs.len() < MAX_MORPH_FILES {
                            lib.morphs.push(MorphFile { stem, gender, folder, genital, package: name.clone(), source: MorphSource::Packaged { package: index, vmi: i, vmb } });
                        }
                    }
                    lib.packages.push((name, z));
                }
                Err(e) => lib.skipped.push((p, e.to_string())),
            }
        }
        Ok(lib)
    }

    /// The presets whose name or package holds `filter` (any case), by name.
    pub fn presets_matching(&self, filter: &str) -> Vec<&Preset> {
        let f = filter.to_lowercase();
        let mut v: Vec<&Preset> = self.presets.iter().filter(|p| f.is_empty() || p.name.to_lowercase().contains(&f) || p.package.to_lowercase().contains(&f)).collect();
        v.sort_by_key(|p| (p.name.to_lowercase(), p.package.clone()));
        v
    }

    /// The items of one kind (for a gender or for both), matching `filter` (any case), by name.
    pub fn items_matching(&self, kind: ItemKind, gender: Gender, filter: &str) -> Vec<&Item> {
        let f = filter.to_lowercase();
        let mut v: Vec<&Item> = self
            .items
            .iter()
            .filter(|i| i.kind == kind && i.gender.is_none_or(|g| g == gender) && (f.is_empty() || i.name.to_lowercase().contains(&f) || i.package.to_lowercase().contains(&f) || i.folder.to_lowercase().contains(&f)))
            .collect();
        v.sort_by_key(|i| (i.name.to_lowercase(), i.package.clone()));
        v
    }

    /// The item a preset names by its `.vam` reference (`Package.N:/Custom/...vam`, `.latest` too).
    pub fn find_item(&self, reference: &str) -> Option<&Item> {
        let (package, path) = reference.split_once(":/")?;
        let name = self.package_named(package).unwrap_or_default();
        self.items.iter().find(|i| i.package == name && i.reference.split_once(":/").map_or(i.reference.as_str(), |(_, p)| p) == path)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The morph files for one gender.
    pub fn morphs_for(&self, gender: Gender) -> impl Iterator<Item = &MorphFile> {
        self.morphs.iter().filter(move |m| m.gender == gender)
    }

    fn read_source(&self, m: &MorphFile) -> Result<(Vec<u8>, Vec<u8>), VamError> {
        match &m.source {
            MorphSource::Loose(vmi) => {
                let rd = |p: &Path| std::fs::read(p).map_err(|e| VamError::Io(p.display().to_string(), e.to_string()));
                Ok((rd(vmi)?, rd(&vmi.with_extension("vmb"))?))
            }
            MorphSource::Packaged { package, vmi, vmb } => {
                let (_, z) = self.packages.get(*package).ok_or_else(|| VamError::Zip("package index out of range".into()))?;
                let get = |i: usize| z.entries.get(i).ok_or_else(|| VamError::Zip("entry index out of range".into())).and_then(|e| z.read(e, MAX_FILE_BYTES));
                Ok((get(*vmi)?, get(*vmb)?))
            }
        }
    }

    /// A morph's header, without its vertex offsets.
    pub fn morph_info(&self, m: &MorphFile) -> Result<MorphInfo, VamError> {
        match &m.source {
            MorphSource::Loose(vmi) => {
                let bytes = std::fs::read(vmi).map_err(|e| VamError::Io(vmi.display().to_string(), e.to_string()))?;
                parse_vmi(&bytes).map_err(|e| VamError::Morph(e.to_string()))
            }
            MorphSource::Packaged { package, vmi, .. } => {
                let (_, z) = self.packages.get(*package).ok_or_else(|| VamError::Zip("package index out of range".into()))?;
                let e = z.entries.get(*vmi).ok_or_else(|| VamError::Zip("entry index out of range".into()))?;
                parse_vmi(&z.read(e, 4 * 1024 * 1024)?).map_err(|e| VamError::Morph(e.to_string()))
            }
        }
    }

    /// A whole morph, offsets included.
    pub fn load_morph(&self, m: &MorphFile) -> Result<Morph, VamError> {
        let (vmi, vmb) = self.read_source(m)?;
        parse_morph(&vmi, &vmb).map_err(|e| VamError::Morph(format!("{}: {e}", m.stem)))
    }

    /// The morph a look names: `Package.N:/Custom/.../Name.vmi` (also `Package.latest`), a path, or
    /// just a name. `None` when no file matches.
    pub fn resolve_morph(&self, uid: &str, gender: Gender) -> Option<&MorphFile> {
        let (package, path) = match uid.split_once(":/") {
            Some((p, rest)) => (Some(p), rest),
            None => (None, uid),
        };
        let stem = Path::new(path.trim_start_matches('/')).file_stem()?.to_str()?;
        let rel = path.replace('\\', "/");
        let wanted_package = package.map(|p| self.package_named(p));
        let mut candidates = self.morphs.iter().filter(|m| m.gender == gender && m.stem == stem);
        match wanted_package {
            Some(Some(name)) => candidates.find(|m| m.package == name),
            // The package is not installed: the same-named loose or other-package morph stands in.
            Some(None) | None => {
                let all: Vec<&MorphFile> = candidates.collect();
                // Prefer the one whose folder path appears in the uid.
                all.iter().copied().find(|m| !m.folder.is_empty() && rel.contains(&format!("{}/{}", m.folder, m.stem))).or_else(|| all.first().copied())
            }
        }
    }

    /// The installed package a reference names, resolving `.latest` to the highest version.
    fn package_named(&self, reference: &str) -> Option<String> {
        if self.packages.iter().any(|(n, _)| n == reference) {
            return Some(reference.to_string());
        }
        let (stem, version) = reference.rsplit_once('.')?;
        if version != "latest" {
            return None;
        }
        self.packages
            .iter()
            .filter_map(|(n, _)| n.rsplit_once('.').filter(|(s, _)| *s == stem).and_then(|(_, v)| v.parse::<u32>().ok()).map(|v| (v, n.clone())))
            .max()
            .map(|(_, n)| n)
    }

    /// A file by the reference a look uses: `Package.N:/path` inside a package, or a path under the
    /// game folder (or absolute). A picture whose own reference does not open (a package that is not
    /// installed, another version, a moved file) is looked for by its file name everywhere in the
    /// install, and the closest match is read.
    pub fn read_file(&self, reference: &str) -> Result<Vec<u8>, VamError> {
        match self.read_exact(reference) {
            Ok(bytes) => Ok(bytes),
            Err(e) => {
                if let Some(alt) = self.find_by_name(reference)
                    && let Ok(bytes) = self.read_exact(&alt)
                {
                    self.matched_by_name.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Ok(bytes);
                }
                Err(e)
            }
        }
    }

    /// How many pictures were found by name since the last call.
    pub fn take_name_matches(&self) -> usize {
        self.matched_by_name.swap(0, std::sync::atomic::Ordering::Relaxed)
    }

    /// The reference of the picture that best stands in for `reference`: the same file name, the most
    /// path parts in common (counted from the end), then the newest package.
    pub fn find_by_name(&self, reference: &str) -> Option<String> {
        if !is_picture(reference) {
            return None;
        }
        let wanted: Vec<String> = reference.split_once(":/").map_or(reference, |(_, p)| p).replace('\\', "/").to_ascii_lowercase().split('/').map(str::to_string).collect();
        let tail_match = |candidate: &str| -> usize {
            let have: Vec<String> = candidate.split_once(":/").map_or(candidate, |(_, p)| p).to_ascii_lowercase().split('/').map(str::to_string).collect();
            wanted.iter().rev().zip(have.iter().rev()).take_while(|(a, b)| a == b).count()
        };
        self.files_by_name.get(&base_name(reference))?.iter().filter(|c| c.as_str() != reference).max_by_key(|c| (tail_match(c), package_version(c))).cloned()
    }

    fn read_exact(&self, reference: &str) -> Result<Vec<u8>, VamError> {
        if let Some((package, path)) = reference.split_once(":/") {
            if let Some(name) = self.package_named(package)
                && let Some((_, z)) = self.packages.iter().find(|(n, _)| *n == name)
            {
                return z.read_by_name(path, MAX_FILE_BYTES);
            }
            return Err(VamError::Zip(format!("package {package} is not installed")));
        }
        let p = Path::new(reference);
        // Never follow a path out of the install by `..`.
        if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(VamError::Io(reference.to_string(), "path leaves the game folder".into()));
        }
        let full = if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) };
        std::fs::read(&full).map_err(|e| VamError::Io(full.display().to_string(), e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::zipfile::tests::{make_zip, temp_file};
    use super::*;

    fn vmb(count: usize) -> Vec<u8> {
        let mut out = (count as i32).to_le_bytes().to_vec();
        for i in 0..count {
            out.extend((i as i32).to_le_bytes());
            out.extend([0u8; 12]);
        }
        out
    }

    fn install() -> PathBuf {
        let root = std::env::temp_dir().join(format!("photocraft-vam-install-{}", std::process::id()));
        let morphs = root.join("Custom/Atom/Person/Morphs/female/AUTO");
        std::fs::create_dir_all(&morphs).unwrap();
        std::fs::write(morphs.join("Big.vmi"), br#"{"id":"Big","displayName":"Big one","group":"G/Body","min":"-1","max":"1","numDeltas":"2","formulas":[]}"#).unwrap();
        std::fs::write(morphs.join("Big.vmb"), vmb(2)).unwrap();
        std::fs::write(morphs.join("NoOffsets.vmi"), br#"{"id":"NoOffsets"}"#).unwrap();
        let genital = root.join("Custom/Atom/Person/Morphs/female_genitalia");
        std::fs::create_dir_all(&genital).unwrap();
        std::fs::write(genital.join("Shape.vmi"), br#"{"id":"Shape"}"#).unwrap();
        std::fs::write(genital.join("Shape.vmb"), vmb(1)).unwrap();
        let odd = root.join("Custom/Atom/Person/Morphs/Pack");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("Lips.vmi"), br#"{"id":"Lips"}"#).unwrap();
        std::fs::write(odd.join("Lips.vmb"), vmb(1)).unwrap();
        std::fs::create_dir_all(root.join("AddonPackages")).unwrap();
        for (name, tall) in [("Maker.Looks.2.var", 3), ("Maker.Looks.10.var", 4)] {
            let zip = make_zip(&[
                ("Custom/Atom/Person/Morphs/female/Maker/Tall.vmi", br#"{"id":"Tall","numDeltas":"1"}"#, true),
                ("Custom/Atom/Person/Morphs/female/Maker/Tall.vmb", &vmb(1), true),
                ("Custom/Atom/Person/Morphs/male/Maker/Tall.vmi", br#"{"id":"TallM"}"#, false),
                ("Custom/Atom/Person/Morphs/male/Maker/Tall.vmb", &vmb(tall), false),
                ("Custom/Atom/Person/Textures/skin.jpg", b"jpegbytes", true),
            ]);
            std::fs::write(root.join("AddonPackages").join(name), zip).unwrap();
        }
        std::fs::write(root.join("AddonPackages/Broken.var"), b"not a zip").unwrap();
        let looks = root.join("Custom/Atom/Person/Appearance");
        std::fs::create_dir_all(&looks).unwrap();
        std::fs::write(looks.join("Mine.vap"), b"{}").unwrap();
        std::fs::write(looks.join("Mine.jpg"), b"jpg").unwrap();
        let zip = make_zip(&[("Custom/Atom/Person/Appearance/Preset_Alma.vap", b"{}", true), ("Custom/Atom/Person/Appearance/Preset_Alma.jpg", b"jpgbytes", true), ("Custom/Atom/Person/Appearance/Other.vap", b"{}", true), ("Custom/Hair/Female/Maker/Bob/Bob.vam", b"{}", true), ("Custom/Hair/Female/Maker/Bob/Bob.vab", b"mesh", true), ("Custom/Hair/Female/Maker/Bob/Bob.jpg", b"pic", true), ("Custom/Clothing/Neutral/Maker/Hat.vam", b"{}", true), ("Custom/Clothing/Neutral/Maker/Hat.vab", b"mesh", true), ("Custom/Clothing/Male/Maker/Tie.vam", b"{}", true), ("Custom/Clothing/Male/Maker/Tie.vab", b"mesh", true), ("Custom/Clothing/Female/Maker/Lonely.vam", b"{}", true), ("Custom/Atom/Person/Textures/Eyes/Blue_Eye.png", b"png", true), ("Custom/Atom/Person/Textures/Eyes/Blue_Eye_Normal.png", b"png", true)]);
        let clothes = root.join("Custom/Clothing/Female/Maker/Dress");
        std::fs::create_dir_all(&clothes).unwrap();
        for f in ["Dress.vam", "Dress.vab", "Dress.jpg"] {
            std::fs::write(clothes.join(f), b"x").unwrap();
        }
        std::fs::write(root.join("AddonPackages/Maker.Presets.1.var"), zip).unwrap();
        root
    }

    #[test]
    fn the_index_finds_loose_and_packaged_morphs_and_reports_broken_packages() {
        let lib = GameLibrary::open(&install()).unwrap();
        assert_eq!(lib.morphs_for(Gender::Female).count(), 4, "Big, the genital Shape, and Tall from each package; NoOffsets has no .vmb");
        assert_eq!(lib.other_morphs, 1, "Lips is outside the gender folders");
        let shape = lib.morphs_for(Gender::Female).find(|m| m.stem == "Shape").unwrap();
        assert!(shape.genital && !lib.morphs_for(Gender::Female).find(|m| m.stem == "Big").unwrap().genital);
        assert_eq!(lib.morphs_for(Gender::Male).count(), 2);
        assert_eq!(lib.skipped.len(), 1);
        let big = lib.morphs_for(Gender::Female).find(|m| m.stem == "Big").unwrap();
        assert_eq!((big.folder.as_str(), big.package.as_str()), ("AUTO", ""));
        assert_eq!(lib.morph_info(big).unwrap().display_name, "Big one");
        assert_eq!(lib.load_morph(big).unwrap().deltas.len(), 2);
        let tall = lib.morphs_for(Gender::Male).find(|m| m.package == "Maker.Looks.10").unwrap();
        assert_eq!(lib.load_morph(tall).unwrap().deltas.len(), 4);
    }

    #[test]
    fn references_resolve_by_package_latest_path_or_bare_name() {
        let lib = GameLibrary::open(&install()).unwrap();
        let f = |uid: &str| lib.resolve_morph(uid, Gender::Female).map(|m| (m.stem.clone(), m.package.clone()));
        assert_eq!(f("Maker.Looks.2:/Custom/Atom/Person/Morphs/female/Maker/Tall.vmi"), Some(("Tall".into(), "Maker.Looks.2".into())));
        assert_eq!(f("Maker.Looks.latest:/Custom/Atom/Person/Morphs/female/Maker/Tall.vmi"), Some(("Tall".into(), "Maker.Looks.10".into())), "latest is the highest version, not the last in the list");
        assert_eq!(f("Big"), Some(("Big".into(), String::new())));
        assert_eq!(f("Other.Pkg.1:/Custom/Atom/Person/Morphs/female/AUTO/Big.vmi").map(|m| m.0), Some("Big".into()), "an uninstalled package falls back to the same-named morph");
        assert_eq!(f("Missing"), None);
        assert_eq!(f(""), None);
    }

    #[test]
    fn files_are_read_from_packages_or_the_game_folder_and_never_from_outside_it() {
        let root = install();
        let lib = GameLibrary::open(&root).unwrap();
        assert_eq!(lib.read_file("Maker.Looks.2:/Custom/Atom/Person/Textures/skin.jpg").unwrap(), b"jpegbytes");
        assert!(lib.read_file("Nobody.Here.1:/x.jpg").is_err());
        assert!(lib.read_file("Maker.Looks.2:/nope.jpg").is_err());
        assert!(lib.read_file("Custom/Atom/Person/Morphs/female/AUTO/Big.vmi").is_ok());
        assert!(lib.read_file("../outside.txt").is_err());
        assert!(GameLibrary::open(&std::env::temp_dir().join("photocraft-vam-no-such-folder")).is_err());
        let _ = temp_file("unused.txt", b"");
    }

    /// A real install indexes and loads (run with `VAM_DIR` set to the game folder).
    #[test]
    #[ignore]
    fn a_real_install_indexes_and_its_morphs_load() {
        let Ok(dir) = std::env::var("VAM_DIR") else { return };
        let t = std::time::Instant::now();
        let lib = GameLibrary::open(Path::new(&dir)).unwrap();
        eprintln!("VAMLIB {} morphs in {} packages, {} skipped, {:?}", lib.morphs.len(), lib.packages.len(), lib.skipped.len(), t.elapsed());
        assert!(lib.morphs.len() >= 800);
        let (mut ok, mut bad) = (0, 0);
        for m in lib.morphs.iter().filter(|m| !m.package.is_empty()).take(300) {
            match lib.load_morph(m) {
                Ok(_) => ok += 1,
                Err(e) => {
                    if bad < 4 {
                        eprintln!("VAMLIB first failures: {} {e}", m.package);
                    }
                    bad += 1;
                }
            }
        }
        eprintln!("VAMLIB packaged morphs loaded {ok}, failed {bad}");
        assert!(bad * 20 <= ok + bad, "{bad} of {} failed", ok + bad);
    }

    #[test]
    fn clothing_and_hair_are_indexed_from_the_folder_and_packages_for_the_figures_gender() {
        let lib = GameLibrary::open(&install()).unwrap();
        let names = |kind, gender| lib.items_matching(kind, gender, "").iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(ItemKind::Clothing, Gender::Female), ["Dress", "Hat"], "an item with no mesh is left out; neutral items fit both");
        assert_eq!(names(ItemKind::Clothing, Gender::Male), ["Hat", "Tie"]);
        assert_eq!(names(ItemKind::Hair, Gender::Female), ["Bob"]);
        let dress = lib.items_matching(ItemKind::Clothing, Gender::Female, "dress")[0];
        assert_eq!((dress.package.as_str(), dress.folder.as_str(), dress.reference.as_str(), dress.thumbnail.as_deref()), ("", "Maker/Dress", "Custom/Clothing/Female/Maker/Dress/Dress.vam", Some("Custom/Clothing/Female/Maker/Dress/Dress.jpg")));
        let bob = lib.find_item("Maker.Presets.1:/Custom/Hair/Female/Maker/Bob/Bob.vam").unwrap();
        assert_eq!(bob.thumbnail.as_deref(), Some("Maker.Presets.1:/Custom/Hair/Female/Maker/Bob/Bob.jpg"));
        assert_eq!(with_extension(&bob.reference, "vab"), "Maker.Presets.1:/Custom/Hair/Female/Maker/Bob/Bob.vab");
        assert_eq!(lib.read_file(&with_extension(&bob.reference, "vab")).unwrap(), b"mesh");
        assert!(lib.find_item("Maker.Presets.1:/Custom/Hair/Female/Maker/Nobody.vam").is_none());
        assert_eq!(lib.eyes.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Blue_Eye"], "normal maps are not eye pictures");
    }

    #[test]
    fn a_picture_whose_reference_does_not_open_is_found_by_its_name() {
        let lib = GameLibrary::open(&install()).unwrap();
        // Another version of the package, and a package that is not installed at all.
        assert_eq!(lib.read_file("Maker.Looks.1:/Custom/Atom/Person/Textures/skin.jpg").unwrap(), b"jpegbytes");
        assert_eq!(lib.read_file("Nobody.Here.3:/Whatever/Folder/SKIN.JPG").unwrap(), b"jpegbytes", "names match without case");
        assert!(lib.take_name_matches() >= 2 && lib.take_name_matches() == 0);
        // Of two candidates the one with more of the path in common wins, then the newer package.
        assert_eq!(lib.find_by_name("X.1:/Custom/Atom/Person/Textures/skin.jpg").as_deref(), Some("Maker.Looks.10:/Custom/Atom/Person/Textures/skin.jpg"));
        // The exact reference is used when it opens; things that are not pictures are never guessed.
        assert_eq!(lib.take_name_matches(), 0);
        assert!(lib.read_file("Maker.Looks.2:/Custom/nope.vmi").is_err());
        assert!(lib.read_file("../skin.jpg").is_err() || lib.take_name_matches() > 0, "no path out of the folder is opened");
    }

    #[test]
    fn eye_pictures_are_found_by_name_and_maps_are_left_out() {
        assert!(is_eye_picture("Custom/Atom/Person/Textures/Eyes/Green_Eye_01.png"));
        assert!(is_eye_picture("Custom/Atom/Person/Textures/Maker/eyes/iris.JPG"));
        assert!(!is_eye_picture("Custom/Atom/Person/Textures/Alter3go/[Alter3go] hearts+red eye+lips+nose+blush.png"), "make-up with eye in its name");
        assert!(!is_eye_picture("Custom/Clothing/Female/Maker/Eyes reflection/Eyes reflection.jpg"), "clothing is not an iris");
        assert!(!is_eye_picture("Custom/Atom/Person/Textures/Maker/eyes/iris_Normal.png"));
        assert!(!is_eye_picture("Custom/Atom/Person/Textures/Maker/skin.png"));
        assert!(!is_eye_picture("Custom/Atom/Person/Textures/Eyes/readme.txt"));
    }

    #[test]
    fn presets_are_listed_from_the_folder_and_from_packages_with_their_pictures() {
        let root = install();
        let lib = GameLibrary::open(&root).unwrap();
        let all = lib.presets_matching("");
        assert_eq!(all.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Mine", "Other", "Preset_Alma"]);
        let mine = all[0];
        assert_eq!((mine.package.as_str(), mine.reference.as_str(), mine.thumbnail.as_deref()), ("", "Custom/Atom/Person/Appearance/Mine.vap", Some("Custom/Atom/Person/Appearance/Mine.jpg")));
        let alma = all[2];
        assert_eq!(alma.reference, "Maker.Presets.1:/Custom/Atom/Person/Appearance/Preset_Alma.vap");
        assert_eq!(lib.read_file(alma.thumbnail.as_deref().unwrap()).unwrap(), b"jpgbytes");
        assert_eq!(lib.read_file(&alma.reference).unwrap(), b"{}");
        assert_eq!(all[1].thumbnail, None);
        assert_eq!(lib.presets_matching("ALMA").len(), 1);
        assert_eq!(lib.presets_matching("maker.pres").len(), 2);
    }
}
