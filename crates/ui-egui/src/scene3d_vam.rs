//! The Morphs page of the 3D editor. A figure made from VaM's own base brings the morphs built
//! into the game (a `.pcmorphs` file next to its `.vrm`); the user can also name their
//! Virt-A-Mate folder to add the morphs found there (loose files and package files). Looks
//! (`.vap`) are imported here too. The files are read where they lie; nothing is copied.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use photocraft_scene3d::model3d::morph::{FormulaKind, Morph, MorphPack};
use photocraft_scene3d::cloth::ClothInput;
use photocraft_vam::{GameLibrary, Gender, Item, ItemKind};

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
    Clothing,
    Hair,
    /// Iris pictures from the game folder and packages.
    Eyes,
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
    // Pictures of the skin and of worn items, and the worn items' meshes, come back from the install.
    let lib = ed.vam.library.clone();
    let mut failed = Vec::new();
    let uids: Vec<String> = ed.scene.material_overrides.iter().flat_map(|o| [o.texture.clone(), o.normal.clone(), o.specular_map.clone()]).flatten().filter(|u| !u.contains('+') && !u.starts_with("average of ") && !u.starts_with("hair:")).collect();
    for uid in uids {
        fetch_picture(ed, lib.as_deref(), &mut failed, &uid);
    }
    if let Some(l) = lib.as_deref() {
        let worn: Vec<(u32, String)> = ed.scene.clothing.iter().filter(|c| !ed.scene.cloth_loaded(c.asset, &c.reference)).map(|c| (c.asset, c.reference.clone())).collect();
        for (asset, reference) in worn {
            match l.find_item(&reference) {
                Some(item) => {
                    if let Err(e) = wear_item(ed, l, asset, item, &ItemLooks::default(), false) {
                        missing.push(e);
                    }
                }
                None => missing.push(reference.rsplit('/').next().unwrap_or(&reference).to_string()),
            }
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
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut ed.vam.tab, MorphTab::Active, format!("{} ({})", tl!("Active morphs"), active_ids.len()));
        ui.selectable_value(&mut ed.vam.tab, MorphTab::All, format!("{} ({})", tl!("All morphs"), ed.vam.rows.len()));
        ui.selectable_value(&mut ed.vam.tab, MorphTab::Looks, format!("{} ({})", tl!("Looks"), lib.as_ref().map_or(0, |l| l.presets.len())));
        ui.selectable_value(&mut ed.vam.tab, MorphTab::Clothing, tl!("Clothing"));
        ui.selectable_value(&mut ed.vam.tab, MorphTab::Hair, tl!("Hair"));
        ui.selectable_value(&mut ed.vam.tab, MorphTab::Eyes, tl!("Eyes"));
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
        MorphTab::Clothing => items_tab(ui, ed, lib.as_deref(), ItemKind::Clothing, asset, gender),
        MorphTab::Eyes => eyes_tab(ui, ed, lib.as_deref(), asset),
        MorphTab::Hair => items_tab(ui, ed, lib.as_deref(), ItemKind::Hair, asset, gender),
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

/// A scrolling list of rows with a small picture each; returns the position of the row clicked.
/// Pictures are made a couple per frame while the rows scroll into view.
fn pick_list(ui: &mut egui::Ui, ed: &mut Scene3dEditor, lib: &GameLibrary, salt: &str, count: usize, row: impl Fn(usize) -> Option<(String, String, Option<String>)>) -> Option<usize> {
    let mut pick: Option<usize> = None;
    let mut made = 0;
    let row_h = THUMB_EDGE as f32 + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical().id_salt(salt).max_height(420.0).show_rows(ui, row_h, count, |ui, range| {
        for pos in range {
            let Some((name, sub, thumb)) = row(pos) else { continue };
            if let Some(t) = &thumb
                && !ed.vam.thumbs.contains_key(t)
                && made < THUMBS_PER_FRAME
            {
                made += 1;
                let tex = thumbnail(lib, t).map(|img| ui.ctx().load_texture(t.clone(), img, egui::TextureOptions::LINEAR));
                ed.vam.thumbs.insert(t.clone(), tex);
            }
            ui.horizontal(|ui| {
                let tex = thumb.as_ref().and_then(|t| ed.vam.thumbs.get(t)).and_then(Option::as_ref);
                let side = egui::vec2(THUMB_EDGE as f32, THUMB_EDGE as f32);
                let clicked_picture = match tex {
                    Some(tex) => ui.add(egui::Image::from_texture(tex).fit_to_exact_size(side).sense(egui::Sense::click())).clicked(),
                    None => ui.add_sized(side, egui::Button::new("")).clicked(),
                };
                ui.vertical(|ui| {
                    let name_resp = ui.add(egui::Button::new(&name).frame(false));
                    let clicked_name = name_resp.clicked();
                    if !sub.is_empty() {
                        ui.weak(&sub);
                    }
                    if clicked_name || clicked_picture {
                        pick = Some(pos);
                    }
                });
            });
        }
    });
    if made >= THUMBS_PER_FRAME {
        ui.ctx().request_repaint();
    }
    pick
}

/// The library indices of what matches the search, by name, kept while the search and library stay.
fn filtered(ed: &mut Scene3dEditor, lib: &GameLibrary, kind_mark: usize, names: impl Fn(usize) -> Option<(String, String)>, total: usize) -> Vec<usize> {
    let wanted = ed.vam.filter.trim().to_lowercase();
    let stamp = std::ptr::from_ref(lib) as usize ^ kind_mark;
    if ed.vam.look_rows.as_ref().is_none_or(|(p, f, _)| *p != stamp || *f != wanted) {
        let mut idx: Vec<(String, String, usize)> = (0..total).filter_map(|i| names(i).map(|(a, b)| (a.to_lowercase(), b, i))).filter(|(n, b, _)| wanted.is_empty() || n.contains(&wanted) || b.to_lowercase().contains(&wanted)).collect();
        idx.sort();
        ed.vam.look_rows = Some((stamp, wanted, idx.into_iter().map(|(_, _, i)| i).collect()));
    }
    ed.vam.look_rows.as_ref().map(|(_, _, r)| r.clone()).unwrap_or_default()
}

/// The Looks tab: the appearance presets of the folder and the packages, with their pictures.
fn looks_tab(ui: &mut egui::Ui, ed: &mut Scene3dEditor, lib: Option<&GameLibrary>) {
    let Some(lib) = lib else {
        ui.weak(tl!("Set the VaM folder above to list its looks."));
        return;
    };
    ui.add(egui::TextEdit::singleline(&mut ed.vam.filter).hint_text(tl!("Search looks…")).desired_width(210.0));
    let rows = filtered(ed, lib, 0, |i| lib.presets.get(i).map(|p| (p.name.clone(), p.package.clone())), lib.presets.len());
    if rows.is_empty() {
        ui.weak(tl!("No looks found."));
    }
    let pick = pick_list(ui, ed, lib, "looks", rows.len(), |pos| rows.get(pos).and_then(|&i| lib.presets.get(i)).map(|p| (p.name.clone(), p.package.clone(), p.thumbnail.clone())));
    if let Some(preset) = pick.and_then(|pos| rows.get(pos)).and_then(|&i| lib.presets.get(i)) {
        match lib.read_file(&preset.reference).map_err(|e| e.to_string()).and_then(|bytes| import_look_from(ed, &preset.name, &bytes, &preset.package)) {
            Ok(summary) => ed.status = summary,
            Err(e) => ed.status = e,
        }
        ui.ctx().request_repaint();
    }
}

/// The Eyes tab: iris pictures to put on the figure's eyes.
fn eyes_tab(ui: &mut egui::Ui, ed: &mut Scene3dEditor, lib: Option<&GameLibrary>, asset: u32) {
    let Some(lib) = lib else {
        ui.weak(tl!("Set the VaM folder above to list its eye pictures."));
        return;
    };
    ui.add(egui::TextEdit::singleline(&mut ed.vam.filter).hint_text(tl!("Search eyes…")).desired_width(210.0));
    let rows = filtered(ed, lib, 3, |i| lib.eyes.get(i).map(|p| (p.name.clone(), p.package.clone())), lib.eyes.len());
    if rows.is_empty() {
        ui.weak(tl!("Nothing found."));
    }
    let pick = pick_list(ui, ed, lib, "eyes", rows.len(), |pos| rows.get(pos).and_then(|&i| lib.eyes.get(i)).map(|p| (p.name.clone(), p.package.clone(), p.thumbnail.clone())));
    if let Some(eye) = pick.and_then(|pos| rows.get(pos)).and_then(|&i| lib.eyes.get(i)) {
        let mut failed = Vec::new();
        if fetch_picture(ed, Some(lib), &mut failed, &eye.reference).is_some() {
            ed.scene.material_overrides.retain(|o| !(o.asset == asset && o.material == "Irises"));
            ed.scene.material_overrides.push(photocraft_scene3d::morph::MaterialOverride { asset, material: "Irises".into(), texture: Some(eye.reference.clone()), ..Default::default() });
            match ed.scene.apply_morphs(asset) {
                Ok(()) => {
                    ed.touch();
                    ed.pending_change = true;
                    ed.status = format!("Eyes: {}", eye.name);
                }
                Err(e) => ed.status = e.to_string(),
            }
        } else {
            ed.status = failed.join("; ");
        }
        ui.ctx().request_repaint();
    }
}

/// The Clothing and Hair tabs: what the figure wears, and the items of the folder and packages.
fn items_tab(ui: &mut egui::Ui, ed: &mut Scene3dEditor, lib: Option<&GameLibrary>, kind: ItemKind, asset: u32, gender: Gender) {
    let Some(lib) = lib else {
        ui.weak(tl!("Set the VaM folder above to list its clothing and hair."));
        return;
    };
    // What is worn, each with a button to take it off.
    let worn: Vec<(String, String)> = ed.scene.worn(asset).iter().filter(|w| lib.find_item(&w.reference).is_none_or(|i| i.kind == kind)).map(|w| (w.name.clone(), w.reference.clone())).collect();
    let mut off: Option<String> = None;
    for (name, reference) in &worn {
        ui.horizontal(|ui| {
            if ui.small_button("✕").on_hover_text(tl!("Take off")).clicked() {
                off = Some(reference.clone());
            }
            ui.label(name);
        });
    }
    if let Some(reference) = off {
        ui.ctx().request_repaint();
        match ed.scene.remove_cloth(asset, &reference) {
            Ok(()) => {
                ed.scene.material_overrides.retain(|o| !o.material.starts_with(&format!("{}:", worn.iter().find(|w| w.1 == reference).map_or("", |w| w.0.as_str()))));
                let _ = ed.scene.apply_morphs(asset);
                ed.touch();
                ed.pending_change = true;
            }
            Err(e) => ed.status = e.to_string(),
        }
    }
    ui.add(egui::TextEdit::singleline(&mut ed.vam.filter).hint_text(if kind == ItemKind::Hair { tl!("Search hair…") } else { tl!("Search clothing…") }).desired_width(210.0));
    let items: Vec<usize> = (0..lib.items.len()).filter(|&i| lib.items.get(i).is_some_and(|it| it.kind == kind && it.gender.is_none_or(|g| g == gender))).collect();
    let rows = filtered(ed, lib, if kind == ItemKind::Hair { 2 } else { 1 }, |i| items.get(i).and_then(|&k| lib.items.get(k)).map(|it| (it.name.clone(), format!("{} {}", it.package, it.folder))), items.len());
    if rows.is_empty() {
        ui.weak(tl!("Nothing found."));
    }
    let pick = pick_list(ui, ed, lib, if kind == ItemKind::Hair { "hair" } else { "clothing" }, rows.len(), |pos| {
        rows.get(pos).and_then(|&i| items.get(i)).and_then(|&k| lib.items.get(k)).map(|it| (it.name.clone(), if it.package.is_empty() { it.folder.clone() } else { it.package.clone() }, it.thumbnail.clone()))
    });
    if let Some(item) = pick.and_then(|pos| rows.get(pos)).and_then(|&i| items.get(i)).and_then(|&k| lib.items.get(k)) {
        match wear_item(ed, lib, asset, item, &ItemLooks::default(), true) {
            Ok(()) => ed.status = format!("Put on {}", item.name),
            Err(e) => ed.status = e,
        }
        ui.ctx().request_repaint();
    }
}

/// What a look says about the items it puts on: their material settings and their hair looks.
#[derive(Clone, Copy, Default)]
pub(crate) struct ItemLooks<'a> {
    pub(crate) parts: &'a [photocraft_vam::PartLook],
    pub(crate) hair: &'a [(String, photocraft_vam::HairLook)],
}

/// The picture a strand hair material has: the colour from the root (top) to the tip, and an
/// opacity that is full along the middle of the ribbon and falls to nothing at its edges.
fn hair_gradient(look: &photocraft_vam::HairLook) -> photocraft_scene3d::model3d::TextureData {
    const W: usize = 8;
    const H: usize = 64;
    let mut rgba = Vec::with_capacity(W * H * 4);
    for y in 0..H {
        let t = (y as f32 / (H - 1) as f32).powf(look.color_rolloff);
        for x in 0..W {
            let edge = 1.0 - (2.0 * (x as f32 + 0.5) / W as f32 - 1.0).abs();
            for c in 0..3 {
                let v = look.root_color[c] + (look.tip_color[c] - look.root_color[c]) * t;
                rgba.push((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
            rgba.push((edge.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
    }
    photocraft_scene3d::model3d::TextureData { width: W as u32, height: H as u32, rgba: Arc::new(rgba) }
}

/// The name a hair look's picture goes by (its colours, so a saved scene can make it again).
fn hair_picture_uid(look: &photocraft_vam::HairLook) -> String {
    let c = |v: [f32; 3]| format!("{:.3},{:.3},{:.3}", v[0], v[1], v[2]);
    format!("hair:{}:{}:{:.2}", c(look.root_color), c(look.tip_color), look.color_rolloff)
}

/// The hair look a picture name stands for.
fn hair_look_from_uid(uid: &str) -> Option<photocraft_vam::HairLook> {
    let mut parts = uid.strip_prefix("hair:")?.split(':');
    let rgb = |s: &str| -> Option<[f32; 3]> {
        let v: Vec<f32> = s.split(',').filter_map(|n| n.parse().ok()).collect();
        <[f32; 3]>::try_from(v).ok()
    };
    Some(photocraft_vam::HairLook { root_color: rgb(parts.next()?)?, tip_color: rgb(parts.next()?)?, color_rolloff: parts.next()?.parse().ok()?, ..Default::default() })
}

/// Puts strand hair on the figure: the guide strands bound to the body, drawn as ribbons.
fn wear_hair(ed: &mut Scene3dEditor, lib: &GameLibrary, asset: u32, item: &Item, bytes: &[u8], looks: &ItemLooks, set_materials: bool) -> Result<(), String> {
    let strands = photocraft_vam::parse_hair(bytes).map_err(|e| format!("{}: {e}", item.name))?;
    if strands.strands.is_empty() {
        return Err(format!("{} has no strands", item.name));
    }
    let meta: serde_json::Value = lib.read_file(&item.reference).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let text = |k: &str| meta.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    let uid = Some(text("uid")).filter(|u| !u.is_empty()).unwrap_or_else(|| item.name.clone());
    let internal = if uid.contains(':') { uid.clone() } else { format!("{}:{uid}", text("creatorName")) };
    let vaj = lib.read_file(&photocraft_vam::with_extension(&item.reference, "vaj")).unwrap_or_default();
    let sim_id = format!("{internal}Sim");
    let known = looks.hair.iter().find(|(id, _)| *id == sim_id).map(|(_, h)| h.clone()).or_else(|| photocraft_vam::parse_item_hair(&vaj, &uid));
    let hair_material = photocraft_scene3d::cloth::cloth_material_name(&item.name, "Hair");
    // A saved scene keeps the look in the name of its picture.
    let saved = ed.scene.material_overrides.iter().find(|o| o.asset == asset && o.material == hair_material).and_then(|o| o.texture.as_deref()).and_then(hair_look_from_uid);
    let look = if set_materials { known.unwrap_or_default() } else { saved.or(known).unwrap_or_default() };
    // The points of every strand, thinned (every other point) when a strand is long, as one list.
    let step = if strands.segments > 24 { 2 } else { 1 };
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for s in &strands.strands {
        let start = positions.len() as u32;
        positions.extend(s.iter().step_by(step));
        if (s.len() - 1) % step != 0 {
            positions.extend(s.last());
        }
        ranges.push((start, positions.len() as u32 - start));
    }
    // Ribbons per guide: a fifth of the game's hairs per guide (the scene trims them to its budget).
    let copies = (look.hair_multiplier / 5.0).round().clamp(1.0, 3.0) as u32;
    let spread = look.spread.map(|f| f * look.max_spread * 0.5);
    let spec = photocraft_scene3d::cloth::HairSpec { strands: ranges, half_width: 0.0010, copies, spread };
    let input = ClothInput { name: item.name.clone(), positions, hair: Some(spec), pieces: vec![("Hair".to_string(), Vec::new()), ("Scalp".to_string(), Vec::new())], ..ClothInput::default() };
    ed.scene.add_cloth(asset, &item.reference, input).map_err(|e| e.to_string())?;
    let uid_picture = hair_picture_uid(&look);
    ed.scene.add_texture_data(&uid_picture, Arc::new(hair_gradient(&look)));
    if set_materials {
        ed.scene.material_overrides.retain(|o| !o.material.starts_with(&format!("{}:", item.name)));
        for part in ["Hair", "Scalp"] {
            let material = photocraft_scene3d::cloth::cloth_material_name(&item.name, part);
            ed.scene.material_overrides.push(photocraft_scene3d::morph::MaterialOverride { asset, material, texture: Some(uid_picture.clone()), mask_cutoff: Some(0.4), specular: Some(0.08), shininess: Some(60.0), ..Default::default() });
        }
    }
    ed.scene.apply_morphs(asset).map_err(|e| e.to_string())?;
    ed.touch();
    ed.pending_change = true;
    Ok(())
}

/// Puts an item on the figure: its mesh bound to the body, and its materials set from the item's own
/// defaults and then from `look_parts` (what a look says about this item's materials).
pub(crate) fn wear_item(ed: &mut Scene3dEditor, lib: &GameLibrary, asset: u32, item: &Item, looks: &ItemLooks, set_materials: bool) -> Result<(), String> {
    let look_parts = looks.parts;
    let bytes = lib.read_file(&photocraft_vam::with_extension(&item.reference, "vab")).map_err(|e| e.to_string())?;
    let mesh = match photocraft_vam::parse_dazmesh(&bytes) {
        Ok(m) => m,
        // Strand hair is made of guide strands, not of a mesh.
        Err(e) if e.to_string().contains("no DAZMesh section") => return wear_hair(ed, lib, asset, item, &bytes, looks, set_materials),
        Err(e) => return Err(format!("{}: {e}", item.name)),
    };
    if mesh.base.is_empty() {
        return Err(format!("{} has no mesh", item.name));
    }
    let meta: serde_json::Value = lib.read_file(&item.reference).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let text = |k: &str| meta.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    let uid = Some(text("uid")).filter(|u| !u.is_empty()).unwrap_or_else(|| item.name.clone());
    // Some items carry the creator in their uid already (`Creator:Name`).
    let internal = if uid.contains(':') { uid.clone() } else { format!("{}:{uid}", text("creatorName")) };
    let vaj = lib.read_file(&photocraft_vam::with_extension(&item.reference, "vaj")).unwrap_or_default();
    let (surface_offset, thickness) = photocraft_vam::parse_wrap_params(&vaj, &uid);
    let wrap = mesh.wrap.iter().map(|w| photocraft_scene3d::cloth::WrapVert { vertices: w.vertices, normal: w.normal, tangent1: w.tangent1, tangent2: w.tangent2, normal_dot: w.normal_dot, tangent1_dot: w.tangent1_dot, tangent2_dot: w.tangent2_dot }).collect();
    let input = ClothInput { hair: None, wrap, surface_offset, thickness, name: item.name.clone(), positions: mesh.uv_positions(), uvs: mesh.uvs.iter().map(|uv| [uv[0], 1.0 - uv[1]]).collect(), uv_base: mesh.uv_base(), pieces: mesh.materials.iter().cloned().zip(mesh.triangles_by_material()).filter(|(_, t)| !t.is_empty()).collect() };
    let pieces: Vec<String> = input.pieces.iter().map(|(m, _)| m.clone()).collect();
    ed.scene.add_cloth(asset, &item.reference, input).map_err(|e| e.to_string())?;
    if !set_materials {
        // Putting a saved item back: its materials are already in the scene.
        ed.scene.apply_morphs(asset).map_err(|e| e.to_string())?;
        return Ok(());
    }
    // Materials: the item's defaults, over them what the look sets.
    let defaults = photocraft_vam::parse_item_materials(&vaj, &uid);
    ed.scene.material_overrides.retain(|o| !o.material.starts_with(&format!("{}:", item.name)));
    let mut failed = Vec::new();
    let library = Some(lib);
    for mat in &pieces {
        // The game names a material by its name without a trailing "-1" (a copy of the same material).
        let short = mat.rsplit_once('-').filter(|(_, n)| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit())).map_or(mat.as_str(), |(h, _)| h);
        let mut part = defaults.iter().find(|(m, _)| m == mat || m == short).map(|(_, p)| p.clone());
        let keys = [format!("{internal}Material{mat}"), format!("{internal}Material{short}")];
        if let Some(over) = look_parts.iter().find(|p| keys.contains(&p.id)) {
            part = Some(over.clone());
        }
        // An item with one material can name its settings differently from its mesh material (hair:
        // `<uid>KrayonScalpMaterial`, eye layers: `<uid>MaterialCombined`): the one it has is its.
        if part.is_none() && pieces.len() == 1 {
            part = look_parts.iter().find(|p| p.id.starts_with(&internal) && p.id.contains("Material")).cloned().or_else(|| defaults.first().filter(|_| defaults.len() == 1).map(|(_, p)| p.clone()));
        }
        let Some(part) = part else { continue };
        apply_part(ed, library, &mut failed, &PartTarget { asset, materials: &[photocraft_scene3d::cloth::cloth_material_name(&item.name, mat)], context: &item.reference, figure_part: false }, &part);
    }
    ed.scene.apply_morphs(asset).map_err(|e| e.to_string())?;
    ed.touch();
    ed.pending_change = true;
    if failed.is_empty() { Ok(()) } else { Err(format!("Put on {}; pictures not read: {}", item.name, join_some(&failed, 2))) }
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
    let short = reference.rsplit(['/', '\\']).next().unwrap_or(reference).to_string();
    let bytes = lib.read_file(reference).map_err(|_| format!("{short} (not in the install)"))?;
    let img = photocraft_codecs::decode(&bytes).map_err(|e| format!("{short}: {e}"))?;
    let (width, height) = img.dimensions();
    let (w, h) = (width as usize, height as usize);
    let max = photocraft_scene3d::model3d::MAX_TEXTURE_EDGE as usize;
    if w == 0 || h == 0 || w.max(h) > 16_384 {
        return Err(format!("{short}: {width}x{height} is not a picture size this editor takes"));
    }
    let rgba = img.to_rgba8();
    // A picture over the limit (some are 8192 wide) is shrunk by whole factors, averaging each block.
    let k = w.max(h).div_ceil(max);
    if k <= 1 {
        return Ok(Arc::new(photocraft_scene3d::model3d::TextureData { width, height, rgba: Arc::new(rgba) }));
    }
    let (nw, nh) = (w / k, h / k);
    let mut out = Vec::with_capacity(nw * nh * 4);
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0u32; 4];
            for dy in 0..k {
                for dx in 0..k {
                    let i = ((y * k + dy) * w + x * k + dx) * 4;
                    if let Some(p) = rgba.get(i..i + 4) {
                        for (s, v) in sum.iter_mut().zip(p) {
                            *s += u32::from(*v);
                        }
                    }
                }
            }
            let n = (k * k) as u32;
            out.extend(sum.map(|s| (s / n) as u8));
        }
    }
    Ok(Arc::new(photocraft_scene3d::model3d::TextureData { width: nw as u32, height: nh as u32, rgba: Arc::new(out) }))
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

/// Reads a picture from the install once and keeps its pixels in the scene under its reference.
fn fetch_picture(ed: &mut Scene3dEditor, lib: Option<&GameLibrary>, failed: &mut Vec<String>, url: &str) -> Option<Arc<photocraft_scene3d::model3d::TextureData>> {
    if url.is_empty() {
        return None;
    }
    if let Some(t) = ed.scene.texture_data(url) {
        return Some(t);
    }
    match lib.ok_or_else(|| "no VaM folder".to_string()).and_then(|l| load_texture(l, url)) {
        Ok(t) => {
            ed.scene.add_texture_data(url, t.clone());
            Some(t)
        }
        Err(e) => {
            failed.push(e);
            None
        }
    }
}

type Picture = photocraft_scene3d::model3d::TextureData;

/// A skin picture with its decal (a picture with see-through parts: tattoos, make-up) laid over it
/// and its detail map (fine grain, grey = no change) multiplied in. Both are scaled to the base.
/// With no base the tint (or white) is the ground, at the decal's size.
fn bake_overlays(base: Option<&Picture>, tint: Option<[f32; 3]>, decal: Option<&Picture>, detail: Option<&Picture>, alpha: Option<&Picture>, adjust: f32) -> Option<Picture> {
    let size_of = |t: &Picture| (t.width as usize, t.height as usize);
    let (w, h) = base.map(size_of).or_else(|| decal.map(size_of)).or_else(|| detail.map(size_of)).or_else(|| alpha.map(size_of))?;
    if w == 0 || h == 0 || w.checked_mul(h)?.checked_mul(4)? > 512 * 1024 * 1024 {
        return None;
    }
    let ground = tint.map_or([255u8; 3], |t| t.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8));
    let at = |t: &Picture, x: usize, y: usize| -> Option<[u8; 4]> {
        let (tw, th) = (t.width as usize, t.height as usize);
        let i = ((y * th / h).min(th.checked_sub(1)?) * tw + (x * tw / w).min(tw.checked_sub(1)?)) * 4;
        let p = t.rgba.get(i..i + 4)?;
        Some([p[0], p[1], p[2], p[3]])
    };
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let mut px = base.and_then(|b| at(b, x, y)).map_or([ground[0], ground[1], ground[2], 255], |p| [p[0], p[1], p[2], 255]);
            if let Some(d) = decal.and_then(|d| at(d, x, y)) {
                let a = f32::from(d[3]) / 255.0;
                for c in 0..3 {
                    px[c] = (f32::from(px[c]) * (1.0 - a) + f32::from(d[c]) * a) as u8;
                }
            }
            if let Some(d) = detail.and_then(|d| at(d, x, y)) {
                for c in 0..3 {
                    let k = 1.0 + (f32::from(d[c]) / 255.0 * 2.0 - 1.0) * 0.5;
                    px[c] = (f32::from(px[c]) * k).clamp(0.0, 255.0) as u8;
                }
            }
            // Opacity: the alpha mask (else the picture's own alpha), plus `Alpha Adjust`.
            let source = alpha.and_then(|m| at(m, x, y)).map(|m| m[0]).or_else(|| base.and_then(|b| at(b, x, y)).map(|b| b[3])).unwrap_or(255);
            px[3] = ((f32::from(source) / 255.0 + adjust).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            out.extend_from_slice(&px);
        }
    }
    Some(Picture { width: w as u32, height: h as u32, rgba: Arc::new(out) })
}

/// Sets what a material storable says on the figure's materials called `materials`: pictures (with
/// decal and alpha mask baked in), tint, normal and specular maps. True when anything was set.
/// Where a material storable's settings go: the figure and the materials, the context its relative
/// references are read against (a package or the item's `.vam`), and whether they are the figure's own.
struct PartTarget<'a> {
    asset: u32,
    materials: &'a [String],
    context: &'a str,
    figure_part: bool,
}

fn apply_part(ed: &mut Scene3dEditor, lib: Option<&GameLibrary>, failed: &mut Vec<String>, target: &PartTarget, part: &photocraft_vam::PartLook) -> bool {
    let PartTarget { asset, materials, context: package, figure_part } = *target;
    let r = |s: &str| photocraft_vam::item_reference(s, package);
    let (diffuse_url, decal_url, alpha_url) = (r(&part.diffuse), r(&part.decal), r(&part.alpha));
    let diffuse = fetch_picture(ed, lib, failed, &diffuse_url);
    let decal = fetch_picture(ed, lib, failed, &decal_url);
    let alpha = fetch_picture(ed, lib, failed, &alpha_url);
    let adjust = part.alpha_adjust.filter(|a| a.abs() > 1e-4).unwrap_or(0.0);
    let mut picture = diffuse.as_ref().map(|_| diffuse_url.clone());
    // A decal, an alpha mask or an `Alpha Adjust` over a picture is baked into the picture's alpha.
    if decal.is_some() || alpha.is_some() || (adjust != 0.0 && diffuse.is_some()) {
        let uid = format!("{diffuse_url}+{decal_url}+{alpha_url}+{adjust}");
        if !ed.scene.has_texture_data(&uid)
            && let Some(t) = bake_overlays(diffuse.as_deref(), part.color, decal.as_deref(), None, alpha.as_deref(), adjust)
        {
            ed.scene.add_texture_data(&uid, Arc::new(t));
        }
        if ed.scene.has_texture_data(&uid) {
            picture = Some(uid);
        }
    }
    // With no picture to carry it, the opacity is a number on the material.
    let opacity = (picture.is_none() && adjust != 0.0).then(|| (1.0 + adjust).clamp(0.0, 1.0));
    let normal = Some(r(&part.normal)).filter(|u| fetch_picture(ed, lib, failed, u).is_some());
    let spec_map = [r(&part.specular), r(&part.gloss_map)].into_iter().find(|u| fetch_picture(ed, lib, failed, u).is_some());
    // White on a picture means no change; on a bare material it is the colour.
    let tint = part.color.filter(|c| (picture.is_none() && !figure_part) || c.iter().any(|v| *v < 0.999));
    if picture.is_none() && normal.is_none() && spec_map.is_none() && tint.is_none() && opacity.is_none() && !part.hidden {
        return false;
    }
    for material in materials {
        ed.scene.material_overrides.push(photocraft_scene3d::morph::MaterialOverride {
            asset,
            material: material.clone(),
            texture: picture.clone(),
            tint,
            normal: normal.clone(),
            specular_map: spec_map.clone(),
            bump: part.bumpiness,
            spec_bump: part.spec_bumpiness,
            specular: part.specular_intensity.map(|v| v * 0.04),
            shininess: part.gloss.map(|g| 4.0 + g * 8.0),
            alpha_mask: (alpha.is_some() || adjust != 0.0) && picture.is_some(),
            opacity,
            hidden: part.hidden,
            mask_cutoff: None,
            tint_multiplies: figure_part,
        });
    }
    true
}

/// Imports a VaM appearance preset onto the figure in the scene: every morph value, the skin pictures
/// (with their normal, specular, gloss, decal and detail maps) and tint, and the eyes, teeth and
/// tongue. Clothing and hair are only counted. Returns a summary to show.
pub(crate) fn import_look(ed: &mut Scene3dEditor, name: &str, bytes: &[u8]) -> Result<String, String> {
    import_look_from(ed, name, bytes, "")
}

/// [`import_look`] for a preset that lies in package `package` (empty: a loose file); the
/// preset's `SELF:` references mean that package.
pub(crate) fn import_look_from(ed: &mut Scene3dEditor, name: &str, bytes: &[u8], package: &str) -> Result<String, String> {
    let mut look = photocraft_vam::parse_vap(bytes).map_err(|e| format!("{name}: {e}"))?;
    look.localize(package);
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
    // Skin and parts: clear what an earlier look put on, then the pictures and the tints.
    let regions = ["face", "torso", "limbs", "genitals"];
    let mut covered: Vec<&str> = regions.iter().flat_map(|r| photocraft_vam::skin_region_materials(gender, r).iter().copied()).collect();
    covered.extend(photocraft_vam::PART_IDS.iter().flat_map(|p| photocraft_vam::part_materials(p).iter().copied()));
    ed.scene.material_overrides.retain(|o| !(o.asset == asset && covered.contains(&o.material.as_str())));
    let lib = ed.vam.library.clone();
    let lib = lib.as_deref();
    let mut pictures = 0;
    let mut failed = Vec::new();
    let url_of = |key: String| look.textures.iter().find(|(k, _)| *k == key).map(|(_, u)| u.clone()).unwrap_or_default();
    for region in regions {
        let diffuse_url = url_of(format!("{region}DiffuseUrl"));
        let decal_url = url_of(format!("{region}DecalUrl"));
        let detail_url = url_of(format!("{region}DetailUrl"));
        let diffuse = fetch_picture(ed, lib, &mut failed, &diffuse_url);
        let decal = fetch_picture(ed, lib, &mut failed, &decal_url);
        let detail = fetch_picture(ed, lib, &mut failed, &detail_url);
        let normal = Some(url_of(format!("{region}NormalUrl"))).filter(|u| fetch_picture(ed, lib, &mut failed, u).is_some());
        // A specular map wins; a gloss map (sweat, oil) stands in for it.
        let spec_map = [format!("{region}SpecularUrl"), format!("{region}GlossUrl")].into_iter().map(url_of).find(|u| fetch_picture(ed, lib, &mut failed, u).is_some());
        let mut picture = diffuse.as_ref().map(|_| diffuse_url.clone());
        if decal.is_some() || detail.is_some() {
            // With no picture of its own, the decal goes over the one the figure was made with.
            let ground = diffuse.clone().or_else(|| photocraft_vam::skin_region_materials(gender, region).first().and_then(|m| ed.scene.original_picture(asset, m)));
            let uid = format!("{}+{decal_url}+{detail_url}", if diffuse.is_some() { diffuse_url.clone() } else { format!("figure {region}") });
            if !ed.scene.has_texture_data(&uid)
                && let Some(t) = bake_overlays(ground.as_deref(), look.skin_color, decal.as_deref(), detail.as_deref(), None, 0.0)
            {
                ed.scene.add_texture_data(&uid, Arc::new(t));
            }
            if ed.scene.has_texture_data(&uid) {
                picture = Some(uid);
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
                    bump: look.diffuse_bumpiness,
                    spec_bump: look.specular_bumpiness,
                    specular: look.specular_intensity.map(|v| v * 0.04),
                    shininess: look.gloss.map(|g| 4.0 + g * 8.0),
                    alpha_mask: false,
                    opacity: None,
                    hidden: false,
                    mask_cutoff: None,
                    tint_multiplies: false,
                });
            }
        }
        if picture.is_some() {
            pictures += 1;
        }
    }
    // Eyes, teeth, tongue: their own pictures (the built-in choices such as "Color 11" keep the
    // figure's own pictures) and tint.
    let mut parts = 0;
    for part in &look.parts {
        let names: Vec<String> = photocraft_vam::part_materials(&part.id).iter().map(|m| (*m).to_string()).collect();
        if apply_part(ed, lib, &mut failed, &PartTarget { asset, materials: &names, context: package, figure_part: true }, part) {
            parts += 1;
        }
    }
    // Clothing and hair: take off what is worn, put on what the look lists (those installed).
    for worn in ed.scene.worn(asset).iter().map(|w| w.reference.clone()).collect::<Vec<_>>() {
        let _ = ed.scene.remove_cloth(asset, &worn);
    }
    ed.scene.material_overrides.retain(|o| !o.material.contains(':'));
    let mut worn_items = 0;
    let mut absent = Vec::new();
    if let Some(l) = lib {
        for (id, on) in look.clothing.iter().chain(&look.hair).filter(|(id, on)| *on && id.contains(":/")) {
            let _ = on;
            match l.find_item(id) {
                Some(item) => match wear_item(ed, l, asset, item, &ItemLooks { parts: &look.item_parts, hair: &look.hair_looks }, true) {
                    Ok(()) => worn_items += 1,
                    Err(e) if e.starts_with("Put on") => {
                        worn_items += 1;
                        absent.push(e);
                    }
                    Err(e) => absent.push(e),
                },
                None => absent.push(id.rsplit('/').next().unwrap_or(id).trim_end_matches(".vam").to_string()),
            }
        }
    }
    ed.scene.apply_morphs(asset).map_err(|e| e.to_string())?;
    ed.touch();
    ed.pending_change = true;
    missing.extend(unknown);
    let mut summary = format!("Look {name}: {} of {} morphs, {pictures} skin pictures, {parts} eye/mouth materials", values.len(), look.morphs.len());
    if !missing.is_empty() {
        summary.push_str(&format!("; not found: {}", join_some(&missing, 6)));
    }
    if !failed.is_empty() {
        failed.sort();
        failed.dedup();
        summary.push_str(&format!("; pictures not read: {}", join_some(&failed, 2)));
    }
    if let Some(l) = lib {
        let found = l.take_name_matches();
        if found > 0 {
            summary.push_str(&format!("; {found} pictures found by name in other places"));
        }
    }
    summary.push_str(&format!("; {worn_items} clothing/hair items put on"));
    if !absent.is_empty() {
        summary.push_str(&format!("; items not found or not read: {}", join_some(&absent, 14)));
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
