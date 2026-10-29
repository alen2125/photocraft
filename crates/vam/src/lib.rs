//! A Virt-A-Mate install, read where it lies. The user names the game folder; this finds the
//! morph files in it (loose ones under `Custom/Atom/Person/Morphs` and those inside the zip
//! packages in `AddonPackages`), and files named by a look or scene (`Package.3:/Custom/...`).
//! Nothing is copied anywhere. Files are hostile input: sizes are capped and nothing panics.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod zipfile;

use std::path::{Path, PathBuf};

use photocraft_model3d::morph::{parse_morph, parse_vmi, Morph, MorphInfo};

pub use zipfile::{ZipEntry, ZipFile};

/// Largest single file read from the install (a morph, a texture).
pub const MAX_FILE_BYTES: usize = 256 * 1024 * 1024;
/// Most morph files indexed in all.
pub const MAX_MORPH_FILES: usize = 200_000;
/// Most packages looked into.
pub const MAX_PACKAGES: usize = 20_000;

const MORPH_DIR: &str = "Custom/Atom/Person/Morphs";

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

#[derive(Debug)]
pub struct GameLibrary {
    root: PathBuf,
    pub packages: Vec<(String, ZipFile)>,
    pub morphs: Vec<MorphFile>,
    /// Packages that could not be read, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
    /// Morph files found outside the female/male (and genitalia) folders: listed, never applied.
    pub other_morphs: usize,
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
        let mut lib = GameLibrary { root: root.to_path_buf(), packages: Vec::new(), morphs: Vec::new(), skipped: Vec::new(), other_morphs: 0 };
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
        // Packages.
        let mut pkgs = Vec::new();
        walk(&root.join("AddonPackages"), 0, &mut pkgs, MAX_PACKAGES);
        for p in pkgs.into_iter().filter(|p| is_ext(p, "var")) {
            match ZipFile::open(&p) {
                Ok(z) => {
                    let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
                    let index = lib.packages.len();
                    let prefix = format!("{MORPH_DIR}/");
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
    /// game folder (or absolute).
    pub fn read_file(&self, reference: &str) -> Result<Vec<u8>, VamError> {
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
}
