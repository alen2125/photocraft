//! The Morphs page of the 3D editor. A figure made from VaM's own base brings the morphs built
//! into the game (a `.pcmorphs` file next to its `.vrm`); the user can also name their
//! Virt-A-Mate folder to add the morphs found there (loose files and package files). Looks
//! (`.vap`) are imported here too. The files are read where they lie; nothing is copied.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use photocraft_scene3d::model3d::morph::{FormulaKind, Morph, MorphPack};
use photocraft_vam::{GameLibrary, Gender};

use crate::scene3d_ui::Scene3dEditor;

/// How many morphs a morph may pull in by driving them (bounded: they can drive each other).
const MAX_DRIVEN: usize = 64;

/// The two pages of the editor's panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Scene,
    Morphs,
}

/// The tabs of the Morphs page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MorphTab {
    #[default]
    Active,
    All,
    /// The appearance presets of the game folder and its packages.
    Looks,
}

/// Where a morph comes from: the pack built into the figure, or a file of the VaM folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Source {
    Pack(usize),
    File(usize),
}

/// One line of the All tab.
struct Row {
    name: String,
    lower: String,
    source: Source,
    /// The package a file morph is from, or empty.
    package: String,
    genital: bool,
}

/// What the editor keeps about the VaM folder and the morphs.
#[derive(Default)]
pub(crate) struct VamState {
    /// The folder as typed.
    pub(crate) dir: String,
    pub(crate) library: Option<Arc<GameLibrary>>,
    pub(crate) message: String,
    pub(crate) filter: String,
    pub(crate) page: Page,
    pub(crate) tab: MorphTab,
    /// The tab shown last frame, to notice leaving the Active tab.
    shown: Option<(Page, MorphTab)>,
    /// The id each loaded morph got, by (asset, key of its source).
    ids: HashMap<(u32, String), String>,
    /// Where each loaded morph came from, by (asset, id), for sliders that work from the id.
    sources: HashMap<(u32, String), Source>,
    /// Morphs that stay in the Active tab even at 0 until the tab is left.
    touched: HashSet<(u32, String)>,
    /// The figures' built-in morphs, read from the file next to the model (`None`: not there).
    packs: HashMap<u32, Option<Arc<MorphPack>>>,
    rows: Vec<Row>,
    /// The Looks tab: presets that match the search (indices into the library), by library and search.
    look_rows: Option<(usize, String, Vec<usize>)>,
    /// Preset pictures by reference, `None` when one could not be read.
    thumbs: HashMap<String, Option<egui::TextureHandle>>,
    rows_for: Option<(usize, usize, u32, bool)>,
    /// Whether the folder was read back from the saved setting yet.
    pub(crate) restored: bool,
}

/// Which gender's morphs a model takes, from its file name (the VaM bases are named for it).
pub(crate) fn gender_of(name: &str) -> Option<Gender> {
    let n = name.to_lowercase();
    if n.contains("female") {
        Some(Gender::Female)
    } else if n.contains("male") {
        Some(Gender::Male)
    } else {
        None
    }
}

fn setting_id() -> egui::Id {
    egui::Id::new("scene3d-vam-folder")
}

/// The first model in the scene that is a VaM figure (it carries morph data), and its gender.
pub(crate) fn figure(ed: &Scene3dEditor) -> Option<(u32, Gender)> {
    ed.scene.assets.iter().find_map(|a| ed.scene.model(a.id).is_some_and(|m| !m.vertex_base.is_empty()).then(|| gender_of(&a.name).map(|g| (a.id, g))).flatten())
}

fn key(source: Source, lib: Option<&GameLibrary>, pack: Option<&MorphPack>) -> String {
    match source {
        Source::Pack(i) => format!("pack:{}", pack.and_then(|p| p.entries.get(i)).map_or("?", |e| e.info.id.as_str())),
        Source::File(i) => format!("file:{}", lib.and_then(|l| l.morphs.get(i)).map_or_else(String::new, |m| format!("{}:{}", m.package, m.stem))),
    }
}

/// The morphs built into the figure: the file next to its model, read once.
fn pack_for(ed: &mut Scene3dEditor, asset: u32) -> Option<Arc<MorphPack>> {
    if let Some(p) = ed.vam.packs.get(&asset) {
        return p.clone();
    }
    let pack = ed
        .scene
        .assets
        .iter()
        .find(|a| a.id == asset)
        .and_then(|a| a.path.as_ref())
        .and_then(|path| std::fs::read(std::path::Path::new(path).with_extension("pcmorphs")).ok())
        .and_then(|bytes| MorphPack::parse(bytes).ok())
        .map(Arc::new);
    ed.vam.packs.insert(asset, pack.clone());
    pack
}

/// Indexes the folder the user typed.
pub(crate) fn open_library(ed: &mut Scene3dEditor, ctx: &egui::Context) {
    let dir = ed.vam.dir.trim().trim_matches('"').to_string();
    ed.vam.library = None;
    ed.vam.rows_for = None;
    if dir.is_empty() {
        ed.vam.message = String::new();
        return;
    }
    match GameLibrary::open(std::path::Path::new(&dir)) {
        Ok(lib) => {
            let (female, male) = (lib.morphs_for(Gender::Female).count(), lib.morphs_for(Gender::Male).count());
            ed.vam.message = format!("{female} female and {male} male extra morphs (genital ones included) from {} packages and the game folder; {} packages unreadable, {} files outside the female/male folders left out", lib.packages.len(), lib.skipped.len(), lib.other_morphs);
            ed.vam.library = Some(Arc::new(lib));
            ctx.data_mut(|d| d.insert_persisted(setting_id(), dir));
            restore_morphs(ed);
        }
        Err(e) => ed.vam.message = e.to_string(),
    }
}

/// What a name (an id, a display name, a `Package.N:/path` reference) means: a built-in morph, or a
/// file of the VaM folder.
fn resolve(ed: &mut Scene3dEditor, asset: u32, gender: Gender, uid: &str) -> Option<Source> {
    let bare = uid.rsplit_once(":/").map_or(uid, |(_, rest)| rest);
    let stem = std::path::Path::new(bare).file_stem().and_then(|s| s.to_str()).unwrap_or(bare);
    // A reference into a package names that package's file, not the built-in of the same name.
    if !uid.contains(":/")
        && let Some(pack) = pack_for(ed, asset)
        && let Some(i) = pack.find(uid).or_else(|| pack.find(stem))
    {
        return Some(Source::Pack(i));
    }
    let lib = ed.vam.library.clone()?;
    let f = lib.resolve_morph(uid, gender)?;
    lib.morphs.iter().position(|m| std::ptr::eq(m, f)).map(Source::File)
}

/// Loads a morph (and the ones it drives) into the scene so its value can be set; returns its id.
fn ensure_loaded(ed: &mut Scene3dEditor, asset: u32, gender: Gender, source: Source) -> Result<String, String> {
    let lib = ed.vam.library.clone();
    let pack = pack_for(ed, asset);
    let first_key = key(source, lib.as_deref(), pack.as_deref());
    if let Some(id) = ed.vam.ids.get(&(asset, first_key.clone())) {
        return Ok(id.clone());
    }
    let mut queue = vec![source];
    let mut first_id = None;
    let mut loaded = 0;
    while let Some(src) = queue.pop() {
        let k = key(src, lib.as_deref(), pack.as_deref());
        if loaded >= MAX_DRIVEN || ed.vam.ids.contains_key(&(asset, k.clone())) {
            continue;
        }
        let morph: Morph = match src {
            Source::Pack(i) => {
                let mut m = pack.as_ref().ok_or("no built-in morphs")?.load(i).map_err(|e| e.to_string())?;
                if m.info.genital {
                    let start = ed.scene.model(asset).and_then(|m| m.graft_start).ok_or("this figure has no genital graft data: export it again")?;
                    for d in &mut m.deltas {
                        d.vertex = d.vertex.saturating_add(start);
                    }
                }
                m
            }
            Source::File(i) => {
                let lib = lib.as_ref().ok_or("no VaM folder")?;
                let f = lib.morphs.get(i).ok_or("morph index out of range")?;
                let mut m = lib.load_morph(f).map_err(|e| e.to_string())?;
                if f.genital {
                    // Genital morphs number their vertices from the start of the genital graft.
                    let start = ed.scene.model(asset).and_then(|m| m.graft_start).ok_or("this figure has no genital graft data: export it again")?;
                    for d in &mut m.deltas {
                        d.vertex = d.vertex.saturating_add(start);
                    }
                }
                m
            }
        };
        loaded += 1;
        for formula in morph.info.formulas.iter().filter(|x| x.kind == FormulaKind::MorphValue) {
            if let Some(t) = resolve(ed, asset, gender, &formula.target) {
                queue.push(t);
            }
        }
        let id = morph.info.id.clone();
        ed.scene.add_morph_data(asset, Arc::new(morph)).map_err(|e| e.to_string())?;
        ed.vam.ids.insert((asset, k), id.clone());
        ed.vam.sources.insert((asset, id.clone()), src);
        if first_id.is_none() {
            first_id = Some(id);
        }
    }
    first_id.or_else(|| ed.vam.ids.get(&(asset, first_key)).cloned()).ok_or_else(|| "morph not loaded".into())
}

/// After a scene was opened: finds the data for the morph values it saved.
pub(crate) fn restore_morphs(ed: &mut Scene3dEditor) {
    let wanted: Vec<(u32, String)> = ed.scene.morphs.iter().map(|m| (m.asset, m.id.clone())).collect();
    let mut missing = Vec::new();
    for (asset, id) in wanted {
        let gender = ed.scene.assets.iter().find(|a| a.id == asset).and_then(|a| gender_of(&a.name)).unwrap_or(Gender::Female);
        match resolve(ed, asset, gender, &id) {
            Some(source) => {
                if ensure_loaded(ed, asset, gender, source).is_err() {
                    missing.push(id);
                }
            }
            None => missing.push(id),
        }
    }
    let assets: Vec<u32> = ed.scene.assets.iter().map(|a| a.id).collect();
    for a in assets {
        if ed.scene.morphs.iter().any(|m| m.asset == a) || ed.scene.material_overrides.iter().any(|o| o.asset == a) {
            let _ = ed.scene.apply_morphs(a);
        }
    }
    if !missing.is_empty() {
        ed.status = format!("Morphs not found: {}", join_some(&missing, 6));
    }
}

/// The page switch at the top of the panel, shown when the scene has a VaM figure. True when the
/// Morphs page is up (and drawn), so the rest of the panel is left out.
pub(crate) fn page_switch(ui: &mut egui::Ui, ed: &mut Scene3dEditor) -> bool {
    if ed.scene.assets.iter().all(|a| ed.scene.model(a.id).is_none_or(|m| m.vertex_base.is_empty())) {
        ed.vam.page = Page::Scene;
        return false;
    }
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ed.vam.page, Page::Scene, tl!("Scene"));
        ui.selectable_value(&mut ed.vam.page, Page::Morphs, tl!("Morphs"));
    });
    ui.separator();
    if ed.vam.page == Page::Morphs {
        morphs_page(ui, ed);
        return true;
    }
    note_leaving(ed, None);
    false
}

/// Morphs that were only kept in the Active tab at 0 go once that tab is left.
fn note_leaving(ed: &mut Scene3dEditor, now: Option<(Page, MorphTab)>) {
    let was = ed.vam.shown;
    ed.vam.shown = now;
    if was == Some((Page::Morphs, MorphTab::Active)) && now != was {
        let keep: HashSet<(u32, String)> = ed.scene.morphs.iter().map(|m| (m.asset, m.id.clone())).collect();
        ed.vam.touched.retain(|k| keep.contains(k));
    }
}

fn slider_range(ed: &Scene3dEditor, asset: u32, id: &str) -> (f32, f32) {
    let (lo, hi) = ed.scene.morph_infos(asset).iter().find(|i| i.id == id).map_or((-1.0, 1.0), |i| (i.min, i.max));
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    (lo.min(-1.0), hi.max(1.0))
}

/// The All tab's lines, built once per folder, figure and pack: alphabetical, the built-in morphs
/// that are meant to be set by hand and the files of the VaM folder for the figure's gender.
fn rebuild_rows(ed: &mut Scene3dEditor, asset: u32, gender: Gender) {
    let lib = ed.vam.library.clone();
    let pack = pack_for(ed, asset);
    let stamp = (lib.as_ref().map_or(0, |l| Arc::as_ptr(l) as usize), pack.as_ref().map_or(0, |p| Arc::as_ptr(p) as usize), asset, gender == Gender::Male);
    if ed.vam.rows_for == Some(stamp) {
        return;
    }
    let mut rows = Vec::new();
    if let Some(p) = &pack {
        for (i, e) in p.entries.iter().enumerate() {
            let hidden = e.info.pose_control || e.info.region.starts_with("Hidden") || e.info.region.starts_with("Pose Controls");
            if !hidden {
                rows.push(Row { lower: e.info.display_name.to_lowercase(), name: e.info.display_name.clone(), source: Source::Pack(i), package: String::new(), genital: e.info.genital });
            }
        }
    }
    if let Some(l) = &lib {
        for (i, m) in l.morphs.iter().enumerate().filter(|(_, m)| m.gender == gender) {
            rows.push(Row { lower: m.stem.to_lowercase(), name: m.stem.clone(), source: Source::File(i), package: m.package.clone(), genital: m.genital });
        }
    }
    rows.sort_by(|a, b| a.lower.cmp(&b.lower));
    ed.vam.rows = rows;
    ed.vam.rows_for = Some(stamp);
}

fn morphs_page(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    if !ed.vam.restored {
        ed.vam.restored = true;
        if let Some(dir) = ui.ctx().data_mut(|d| d.get_persisted::<String>(setting_id())) {
            ed.vam.dir = dir;
            open_library(ed, &ui.ctx().clone());
        }
    }
    let Some((asset, gender)) = figure(ed) else {
        ui.weak(tl!("Import a VaM figure (vam_base_female / vam_base_male) to use morphs."));
        return;
    };
    let has_pack = pack_for(ed, asset).is_some();
    egui::CollapsingHeader::new(tl!("VaM folder")).default_open(ed.vam.library.is_none()).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut ed.vam.dir).desired_width(210.0).hint_text(r"F:\Virt-a-Mate"));
            if ui.button(tl!("Read")).clicked() {
                let ctx = ui.ctx().clone();
                open_library(ed, &ctx);
            }
        });
        if !ed.vam.message.is_empty() {
            ui.weak(&ed.vam.message);
        }
        if !has_pack {
            ui.weak(tl!("The figure's built-in morphs file (.pcmorphs) is not next to it: export the figure again."));
        }
    });
    if ui.button(tl!("Import look (.vap)…")).on_hover_text(tl!("Sets the morphs and the skin pictures from a VaM appearance preset. Clothing and hair are not imported yet.")).clicked() {
        ed.want_look = true;
    }
    rebuild_rows(ed, asset, gender);
    let lib = ed.vam.library.clone();
    let pack = pack_for(ed, asset);
    let active_ids: Vec<String> = {
        let mut v: Vec<String> = ed.scene.morphs.iter().filter(|m| m.asset == asset).map(|m| m.id.clone()).collect();
        v.extend(ed.vam.touched.iter().filter(|(a, _)| *a == asset).map(|(_, id)| id.clone()));
        v.sort_by_key(|s| s.to_lowercase());
        v.dedup();
        v
    };
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ed.vam.tab, MorphTab::Active, format!("{} ({})", tl!("Active morphs"), active_ids.len()));
        ui.selectable_value(&mut ed.vam.tab, MorphTab::All, format!("{} ({})", tl!("All morphs"), ed.vam.rows.len()));
        ui.selectable_value(&mut ed.vam.tab, MorphTab::Looks, format!("{} ({})", tl!("Looks"), lib.as_ref().map_or(0, |l| l.presets.len())));
        if ui.button(tl!("Reset morphs")).clicked() {
            let _ = ed.scene.reset_morphs(asset);
            ed.vam.touched.clear();
            ed.touch();
        }
    });
    note_leaving(ed, Some((Page::Morphs, ed.vam.tab)));
    ui.separator();
    // One change at most per frame: which morph, and to what value.
    let mut set: Option<(Source, f32)> = None;
    match ed.vam.tab {
        MorphTab::Active => {
            if active_ids.is_empty() {
                ui.weak(tl!("No morphs are on. Move one in the All tab and it appears here."));
            }
            egui::ScrollArea::vertical().id_salt("morphs-active").max_height(400.0).show(ui, |ui| {
                for id in &active_ids {
                    let mut v = ed.scene.morph_value(asset, id);
                    let (lo, hi) = slider_range(ed, asset, id);
                    let shown = pack.as_ref().and_then(|p| p.find(id).and_then(|i| p.entries.get(i))).map_or_else(|| id.clone(), |e| e.info.display_name.clone());
                    if ui.add(egui::Slider::new(&mut v, lo..=hi).text(shown).max_decimals(3)).changed() {
                        let source = ed.vam.sources.get(&(asset, id.clone())).copied().or_else(|| pack.as_ref().and_then(|p| p.find(id)).map(Source::Pack));
                        if let Some(s) = source {
                            set = Some((s, v));
                        }
                    }
                }
            });
        }
        MorphTab::Looks => looks_tab(ui, ed, lib.as_deref()),
        MorphTab::All => {
            ui.add(egui::TextEdit::singleline(&mut ed.vam.filter).hint_text(tl!("Search morphs…")).desired_width(210.0));
            let wanted = ed.vam.filter.trim().to_lowercase();
            let shown: Vec<usize> = (0..ed.vam.rows.len()).filter(|&i| wanted.is_empty() || ed.vam.rows[i].lower.contains(&wanted)).collect();
            let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical().id_salt("morphs-all").max_height(400.0).show_rows(ui, row_h, shown.len(), |ui, range| {
                for &i in shown.get(range).unwrap_or(&[]) {
                    let Some(row) = ed.vam.rows.get(i) else { continue };
                    let k = key(row.source, lib.as_deref(), pack.as_deref());
                    let id = ed.vam.ids.get(&(asset, k)).cloned().unwrap_or_else(|| match row.source {
                        Source::Pack(p) => pack.as_ref().and_then(|pk| pk.entries.get(p)).map_or_else(|| row.name.clone(), |e| e.info.id.clone()),
                        Source::File(_) => row.name.clone(),
                    });
                    let mut v = ed.scene.morph_value(asset, &id);
                    let name = if row.genital { format!("{} (genital)", row.name) } else { row.name.clone() };
                    let label = if row.package.is_empty() { name } else { format!("{name}  ·  {}", row.package) };
                    let (lo, hi) = slider_range(ed, asset, &id);
                    if ui.add(egui::Slider::new(&mut v, lo..=hi).text(label).max_decimals(3)).changed() {
                        set = Some((row.source, v));
                    }
                }
            });
        }
    }
    if let Some((source, value)) = set {
        match ensure_loaded(ed, asset, gender, source).and_then(|id| {
            ed.vam.touched.insert((asset, id.clone()));
            ed.scene.set_morph(asset, &id, value).map_err(|e| e.to_string())
        }) {
            Ok(()) => {
                ed.touch();
                ed.pending_change = true;
            }
            Err(e) => ed.status = e,
        }
    }
}

/// Pictures made per frame for the Looks list (decoding is the slow part).
const THUMBS_PER_FRAME: usize = 2;
const THUMB_EDGE: usize = 64;

/// A preset's picture, shrunk to a small square-ish thumbnail.
fn thumbnail(lib: &GameLibrary, reference: &str) -> Option<egui::ColorImage> {
    let bytes = lib.read_file(reference).ok()?;
    let img = photocraft_codecs::decode(&bytes).ok()?;
    let (w, h) = img.dimensions();
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || w.max(h) > 16_384 {
        return None;
    }
    let rgba = img.to_rgba8();
    let scale = (w.max(h) as f32 / THUMB_EDGE as f32).max(1.0);
    let (tw, th) = (((w as f32 / scale) as usize).max(1), ((h as f32 / scale) as usize).max(1));
    let mut out = Vec::with_capacity(tw * th * 4);
    for y in 0..th {
        for x in 0..tw {
            let (sx, sy) = (((x as f32 + 0.5) * scale) as usize, ((y as f32 + 0.5) * scale) as usize);
            let at = (sy.min(h - 1) * w + sx.min(w - 1)) * 4;
            out.extend_from_slice(rgba.get(at..at + 4)?);
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([tw, th], &out))
}

/// The Looks tab: the appearance presets of the folder and the packages, with their pictures.
fn looks_tab(ui: &mut egui::Ui, ed: &mut Scene3dEditor, lib: Option<&GameLibrary>) {
    let Some(lib) = lib else {
        ui.weak(tl!("Set the VaM folder above to list its looks."));
        return;
    };
    ui.add(egui::TextEdit::singleline(&mut ed.vam.filter).hint_text(tl!("Search looks…")).desired_width(210.0));
    let wanted = ed.vam.filter.trim().to_lowercase();
    let stamp = std::ptr::from_ref(lib) as usize;
    if ed.vam.look_rows.as_ref().is_none_or(|(p, f, _)| *p != stamp || *f != wanted) {
        let mut idx: Vec<usize> = (0..lib.presets.len()).filter(|&i| lib.presets.get(i).is_some_and(|p| wanted.is_empty() || p.name.to_lowercase().contains(&wanted) || p.package.to_lowercase().contains(&wanted))).collect();
        idx.sort_by_key(|&i| lib.presets.get(i).map(|p| (p.name.to_lowercase(), p.package.clone())));
        ed.vam.look_rows = Some((stamp, wanted, idx));
    }
    let rows: Vec<usize> = ed.vam.look_rows.as_ref().map(|(_, _, r)| r.clone()).unwrap_or_default();
    if rows.is_empty() {
        ui.weak(tl!("No looks found."));
    }
    let mut pick: Option<usize> = None;
    let mut made = 0;
    let row_h = THUMB_EDGE as f32 + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical().id_salt("looks").max_height(420.0).show_rows(ui, row_h, rows.len(), |ui, range| {
        for &i in rows.get(range).unwrap_or(&[]) {
            let Some(preset) = lib.presets.get(i) else { continue };
            if let Some(t) = &preset.thumbnail
                && !ed.vam.thumbs.contains_key(t)
                && made < THUMBS_PER_FRAME
            {
                made += 1;
                let tex = thumbnail(lib, t).map(|img| ui.ctx().load_texture(t.clone(), img, egui::TextureOptions::LINEAR));
                ed.vam.thumbs.insert(t.clone(), tex);
            }
            ui.horizontal(|ui| {
                let tex = preset.thumbnail.as_ref().and_then(|t| ed.vam.thumbs.get(t)).and_then(Option::as_ref);
                let side = egui::vec2(THUMB_EDGE as f32, THUMB_EDGE as f32);
                let clicked_picture = match tex {
                    Some(tex) => ui.add(egui::Image::from_texture(tex).fit_to_exact_size(side).sense(egui::Sense::click())).clicked(),
                    None => ui.add_sized(side, egui::Button::new("")).clicked(),
                };
                ui.vertical(|ui| {
                    let clicked_name = ui.add(egui::Button::new(&preset.name).frame(false)).clicked();
                    if !preset.package.is_empty() {
                        ui.weak(&preset.package);
                    }
                    if clicked_name || clicked_picture {
                        pick = Some(i);
                    }
                });
            });
        }
    });
    if made >= THUMBS_PER_FRAME {
        ui.ctx().request_repaint();
    }
    if let Some(i) = pick
        && let Some(preset) = lib.presets.get(i)
    {
        match lib.read_file(&preset.reference).map_err(|e| e.to_string()).and_then(|bytes| import_look(ed, &preset.name, &bytes)) {
            Ok(summary) => ed.status = summary,
            Err(e) => ed.status = e,
        }
    }
}

/// A short list for the status line.
fn join_some(v: &[String], max: usize) -> String {
    let mut s = v.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if v.len() > max {
        s.push_str(&format!(" and {} more", v.len() - max));
    }
    s
}

/// Decodes a skin picture from the install into the scene's texture format.
fn load_texture(lib: &GameLibrary, reference: &str) -> Result<Arc<photocraft_scene3d::model3d::TextureData>, String> {
    let bytes = lib.read_file(reference).map_err(|e| e.to_string())?;
    let img = photocraft_codecs::decode(&bytes).map_err(|e| format!("{reference}: {e}"))?;
    let (width, height) = img.dimensions();
    if width.max(height) > photocraft_scene3d::model3d::MAX_TEXTURE_EDGE {
        return Err(format!("{reference}: {width}x{height} is larger than the {} pixels a picture may have", photocraft_scene3d::model3d::MAX_TEXTURE_EDGE));
    }
    Ok(Arc::new(photocraft_scene3d::model3d::TextureData { width, height, rgba: Arc::new(img.to_rgba8()) }))
}

/// A one-pixel picture of the average colour of another (skipping transparent pixels).
fn average_picture(t: &photocraft_scene3d::model3d::TextureData) -> Arc<photocraft_scene3d::model3d::TextureData> {
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for p in t.rgba.as_chunks::<4>().0.iter().step_by(7) {
        if p[3] > 200 {
            sum[0] += u64::from(p[0]);
            sum[1] += u64::from(p[1]);
            sum[2] += u64::from(p[2]);
            n += 1;
        }
    }
    let n = n.max(1);
    Arc::new(photocraft_scene3d::model3d::TextureData { width: 1, height: 1, rgba: Arc::new(vec![(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]) })
}

/// Imports a VaM appearance preset onto the figure in the scene: every morph value, and the skin
/// pictures and tint. Clothing and hair are only counted. Returns a summary to show.
pub(crate) fn import_look(ed: &mut Scene3dEditor, name: &str, bytes: &[u8]) -> Result<String, String> {
    let look = photocraft_vam::parse_vap(bytes).map_err(|e| format!("{name}: {e}"))?;
    let (asset, gender) = figure(ed).ok_or("Import a VaM figure (vam_base_female / vam_base_male) first")?;
    let wants_male = look.character.to_lowercase().starts_with("male");
    if !look.character.is_empty() && wants_male != (gender == Gender::Male) {
        return Err(format!("{name} is for a {} figure; the figure in the scene is {}", if wants_male { "male" } else { "female" }, if gender == Gender::Male { "male" } else { "female" }));
    }
    // Morphs: every one the look lists; the rest go back to 0 (a look sets what it does not name to default).
    let mut values = Vec::new();
    let mut missing = Vec::new();
    for m in &look.morphs {
        match resolve(ed, asset, gender, &m.uid).or_else(|| resolve(ed, asset, gender, &m.name)) {
            Some(source) => match ensure_loaded(ed, asset, gender, source) {
                Ok(id) => values.push((id, m.value)),
                Err(e) => missing.push(format!("{} ({e})", m.name)),
            },
            None => missing.push(m.name.clone()),
        }
    }
    let unknown = ed.scene.set_morph_values(asset, &values, true).map_err(|e| e.to_string())?;
    ed.vam.touched.clear();
    // Skin: clear what an earlier look put on, then the pictures and the tint.
    let skin_names: Vec<&str> = ["face", "torso", "limbs", "genitals"].iter().flat_map(|r| photocraft_vam::skin_region_materials(gender, r).iter().copied()).collect();
    ed.scene.material_overrides.retain(|o| !(o.asset == asset && skin_names.contains(&o.material.as_str())));
    let lib = ed.vam.library.clone();
    let mut pictures = 0;
    let mut maps_left_out = 0;
    let mut failed = Vec::new();
    // The normal maps' strength: VaM has one number for the diffuse light and one for the highlight.
    let bump = match (look.diffuse_bumpiness, look.specular_bumpiness) {
        (Some(a), Some(b)) => Some((a + b) / 2.0),
        (a, b) => a.or(b),
    };
    for region in ["face", "torso", "limbs", "genitals"] {
        let diffuse = look.textures.iter().find(|(k, _)| *k == format!("{region}DiffuseUrl")).map(|(_, u)| u.clone());
        maps_left_out += look.textures.iter().filter(|(k, _)| k.starts_with(region) && (k.ends_with("DetailUrl") || k.ends_with("DecalUrl"))).count();
        let mut fetch = |ed: &mut Scene3dEditor, key: String| -> Option<String> {
            let url = look.textures.iter().find(|(k, _)| *k == key).map(|(_, u)| u.clone())?;
            if !ed.scene.has_texture_data(&url) {
                match lib.as_ref().ok_or_else(|| "no VaM folder".to_string()).and_then(|l| load_texture(l, &url)) {
                    Ok(t) => ed.scene.add_texture_data(&url, t),
                    Err(e) => {
                        failed.push(e);
                        return None;
                    }
                }
            }
            Some(url)
        };
        let normal = fetch(ed, format!("{region}NormalUrl"));
        // A specular map wins; a gloss map (sweat, oil) stands in for it.
        let spec_map = fetch(ed, format!("{region}SpecularUrl")).or_else(|| fetch(ed, format!("{region}GlossUrl")));
        let mut picture = diffuse.clone();
        if let Some(url) = &diffuse
            && !ed.scene.has_texture_data(url)
        {
            match lib.as_ref().ok_or_else(|| "no VaM folder".to_string()).and_then(|l| load_texture(l, url)) {
                Ok(t) => ed.scene.add_texture_data(url, t),
                Err(e) => {
                    failed.push(e);
                    picture = None;
                }
            }
        }
        // The genital graft has its own picture layout: with none of its own, VaM blends it from the
        // torso's. Here it takes the torso picture's average colour, so it does not show as a patch.
        if region == "genitals"
            && picture.is_none()
            && let Some(torso) = look.textures.iter().find(|(k, _)| k == "torsoDiffuseUrl").and_then(|(_, u)| ed.scene.texture_data(u))
        {
            let uid = format!("average of {}", look.textures.iter().find(|(k, _)| k == "torsoDiffuseUrl").map_or("", |(_, u)| u.as_str()));
            ed.scene.add_texture_data(&uid, average_picture(&torso));
            picture = Some(uid);
        }
        for material in photocraft_vam::skin_region_materials(gender, region) {
            if picture.is_some() || look.skin_color.is_some() {
                ed.scene.material_overrides.push(photocraft_scene3d::morph::MaterialOverride {
                    asset,
                    material: (*material).to_string(),
                    texture: picture.clone(),
                    tint: look.skin_color,
                    normal: normal.clone(),
                    specular_map: spec_map.clone(),
                    bump,
                    specular: look.specular_intensity.map(|v| v * 0.04),
                    shininess: look.gloss.map(|g| 4.0 + g * 8.0),
                });
            }
        }
        if picture.is_some() {
            pictures += 1;
        }
    }
    ed.scene.apply_morphs(asset).map_err(|e| e.to_string())?;
    ed.touch();
    ed.pending_change = true;
    missing.extend(unknown);
    let mut summary = format!("Look {name}: {} of {} morphs, {pictures} skin pictures", values.len(), look.morphs.len());
    if !missing.is_empty() {
        summary.push_str(&format!("; not found: {}", join_some(&missing, 6)));
    }
    if !failed.is_empty() {
        summary.push_str(&format!("; pictures not read: {}", join_some(&failed, 2)));
    }
    let items = look.clothing.iter().chain(&look.hair).filter(|(_, on)| *on).count();
    if items > 0 {
        summary.push_str(&format!("; {items} clothing/hair items are not imported yet"));
    }
    if maps_left_out > 0 {
        summary.push_str(&format!("; {maps_left_out} detail/decal maps are not shown"));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_morph_moved_back_to_zero_stays_in_the_active_tab_until_that_tab_is_left() {
        let mut ed = Scene3dEditor::default();
        ed.scene.morphs.push(photocraft_scene3d::morph::MorphValue { asset: 1, id: "Kept".into(), value: 0.4 });
        ed.vam.touched.insert((1, "Kept".into()));
        ed.vam.touched.insert((1, "Zeroed".into()));
        // Still looking at the Active tab: nothing goes.
        note_leaving(&mut ed, Some((Page::Morphs, MorphTab::Active)));
        note_leaving(&mut ed, Some((Page::Morphs, MorphTab::Active)));
        assert_eq!(ed.vam.touched.len(), 2);
        // The All tab: the one at zero goes, the one with a value stays.
        note_leaving(&mut ed, Some((Page::Morphs, MorphTab::All)));
        assert_eq!(ed.vam.touched.iter().map(|k| k.1.as_str()).collect::<Vec<_>>(), vec!["Kept"]);
        // Leaving the page counts too.
        ed.vam.touched.insert((1, "Later".into()));
        note_leaving(&mut ed, Some((Page::Morphs, MorphTab::Active)));
        note_leaving(&mut ed, None);
        assert!(!ed.vam.touched.iter().any(|k| k.1 == "Later"));
    }

    #[test]
    fn the_gender_comes_from_the_figures_name() {
        assert_eq!(gender_of("dir/vam_base_female.vrm"), Some(Gender::Female));
        assert_eq!(gender_of("vam_base_male.vrm"), Some(Gender::Male));
        assert_eq!(gender_of("hero.vrm"), None);
    }
}
