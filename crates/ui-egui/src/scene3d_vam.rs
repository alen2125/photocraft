//! The Morphs page of the 3D editor: the user names their Virt-A-Mate folder, and a figure made
//! from VaM's own base takes the morphs found there (loose files and package files) as sliders.
//! The files are read where they lie; nothing is copied.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use photocraft_vam::{GameLibrary, Gender, MorphFile};

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
}

/// What the editor keeps about the VaM folder.
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
    /// The id each loaded morph file got (its header's id), by (asset, file stem).
    pub(crate) ids: HashMap<(u32, String), String>,
    /// The file of each loaded morph, by (asset, id), for sliders that work from the id.
    files: HashMap<(u32, String), MorphFile>,
    /// Morphs that stay in the Active tab even at 0 until the tab is left.
    touched: HashSet<(u32, String)>,
    /// Indices into the library's morph list for the gender shown, sorted by name.
    sorted: Vec<usize>,
    sorted_for: Option<(usize, Gender)>,
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

/// Indexes the folder the user typed.
pub(crate) fn open_library(ed: &mut Scene3dEditor, ctx: &egui::Context) {
    let dir = ed.vam.dir.trim().trim_matches('"').to_string();
    ed.vam.library = None;
    ed.vam.sorted_for = None;
    if dir.is_empty() {
        ed.vam.message = String::new();
        return;
    }
    match GameLibrary::open(std::path::Path::new(&dir)) {
        Ok(lib) => {
            let (female, male) = (lib.morphs_for(Gender::Female).count(), lib.morphs_for(Gender::Male).count());
            ed.vam.message = format!("{female} female and {male} male morphs (genital ones included) from {} packages and the game folder; {} packages unreadable, {} files outside the female/male folders left out", lib.packages.len(), lib.skipped.len(), lib.other_morphs);
            ed.vam.library = Some(Arc::new(lib));
            ctx.data_mut(|d| d.insert_persisted(setting_id(), dir));
            restore_morphs(ed);
        }
        Err(e) => ed.vam.message = e.to_string(),
    }
}

/// Loads a morph (and the ones it drives) into the scene so its value can be set.
pub(crate) fn ensure_loaded(ed: &mut Scene3dEditor, asset: u32, gender: Gender, file: &MorphFile) -> Result<String, String> {
    let key = (asset, file.stem.clone());
    if let Some(id) = ed.vam.ids.get(&key) {
        return Ok(id.clone());
    }
    let Some(lib) = ed.vam.library.clone() else { return Err("no VaM folder".into()) };
    let mut queue = vec![file.clone()];
    let mut first_id = None;
    let mut loaded = 0;
    while let Some(f) = queue.pop() {
        if loaded >= MAX_DRIVEN || ed.vam.ids.contains_key(&(asset, f.stem.clone())) {
            continue;
        }
        let mut morph = lib.load_morph(&f).map_err(|e| e.to_string())?;
        loaded += 1;
        if f.genital {
            // Genital morphs number their vertices from the start of the genital graft.
            let start = ed.scene.model(asset).and_then(|m| m.graft_start).ok_or("this figure has no genital graft data: export it again")?;
            for d in &mut morph.deltas {
                d.vertex = d.vertex.saturating_add(start);
            }
        }
        for formula in morph.info.formulas.iter().filter(|x| x.kind == photocraft_scene3d::model3d::morph::FormulaKind::MorphValue) {
            if let Some(t) = lib.resolve_morph(&formula.target, gender) {
                queue.push(t.clone());
            }
        }
        let id = morph.info.id.clone();
        ed.scene.add_morph_data(asset, Arc::new(morph)).map_err(|e| e.to_string())?;
        ed.vam.ids.insert((asset, f.stem.clone()), id.clone());
        ed.vam.files.insert((asset, id.clone()), f.clone());
        if first_id.is_none() {
            first_id = Some(id);
        }
    }
    first_id.or_else(|| ed.vam.ids.get(&key).cloned()).ok_or_else(|| "morph not loaded".into())
}

/// After a scene was opened: finds the data for the morph values it saved.
pub(crate) fn restore_morphs(ed: &mut Scene3dEditor) {
    let Some(lib) = ed.vam.library.clone() else { return };
    let wanted: Vec<(u32, String)> = ed.scene.morphs.iter().map(|m| (m.asset, m.id.clone())).collect();
    let mut missing = Vec::new();
    for (asset, id) in wanted {
        let gender = ed.scene.assets.iter().find(|a| a.id == asset).and_then(|a| gender_of(&a.name)).unwrap_or(Gender::Female);
        match lib.resolve_morph(&id, gender).cloned() {
            Some(file) => {
                if ensure_loaded(ed, asset, gender, &file).is_err() {
                    missing.push(id);
                }
            }
            None => missing.push(id),
        }
    }
    let assets: Vec<u32> = ed.scene.assets.iter().map(|a| a.id).collect();
    for a in assets {
        if ed.scene.morphs.iter().any(|m| m.asset == a) {
            let _ = ed.scene.apply_morphs(a);
        }
    }
    if !missing.is_empty() {
        ed.status = format!("Morphs not found in the VaM folder: {}", missing.join(", "));
    }
}

/// The page switch at the top of the panel, shown when the scene has a VaM figure. True when the
/// Morphs page is up (and drawn), so the rest of the panel is left out.
pub(crate) fn page_switch(ui: &mut egui::Ui, ed: &mut Scene3dEditor) -> bool {
    if figure(ed).is_none() && ed.scene.assets.iter().all(|a| ed.scene.model(a.id).is_none_or(|m| m.vertex_base.is_empty())) {
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

fn morphs_page(ui: &mut egui::Ui, ed: &mut Scene3dEditor) {
    if !ed.vam.restored {
        ed.vam.restored = true;
        if let Some(dir) = ui.ctx().data_mut(|d| d.get_persisted::<String>(setting_id())) {
            ed.vam.dir = dir;
            open_library(ed, &ui.ctx().clone());
        }
    }
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
    });
    let Some(lib) = ed.vam.library.clone() else {
        note_leaving(ed, Some((Page::Morphs, ed.vam.tab)));
        return;
    };
    let Some((asset, gender)) = figure(ed) else {
        ui.weak(tl!("Import a VaM figure (vam_base_female / vam_base_male) to use morphs."));
        return;
    };
    // The figure's morph files in alphabetical order, built once per folder and gender.
    if ed.vam.sorted_for != Some((Arc::as_ptr(&lib) as usize, gender)) {
        let mut idx: Vec<usize> = lib.morphs.iter().enumerate().filter(|(_, m)| m.gender == gender).map(|(i, _)| i).collect();
        idx.sort_by_cached_key(|&i| lib.morphs.get(i).map(|m| m.stem.to_lowercase()).unwrap_or_default());
        ed.vam.sorted = idx;
        ed.vam.sorted_for = Some((Arc::as_ptr(&lib) as usize, gender));
    }
    let active_ids: Vec<String> = {
        let mut v: Vec<String> = ed.scene.morphs.iter().filter(|m| m.asset == asset).map(|m| m.id.clone()).collect();
        v.extend(ed.vam.touched.iter().filter(|(a, _)| *a == asset).map(|(_, id)| id.clone()));
        v.sort_by_key(|s| s.to_lowercase());
        v.dedup();
        v
    };
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ed.vam.tab, MorphTab::Active, format!("{} ({})", tl!("Active morphs"), active_ids.len()));
        ui.selectable_value(&mut ed.vam.tab, MorphTab::All, format!("{} ({})", tl!("All morphs"), ed.vam.sorted.len()));
        if ui.button(tl!("Reset morphs")).clicked() {
            let _ = ed.scene.reset_morphs(asset);
            ed.vam.touched.clear();
            ed.touch();
        }
    });
    note_leaving(ed, Some((Page::Morphs, ed.vam.tab)));
    ui.separator();
    // One change at most per frame: which file, and to what value.
    let mut set: Option<(MorphFile, f32)> = None;
    match ed.vam.tab {
        MorphTab::Active => {
            if active_ids.is_empty() {
                ui.weak(tl!("No morphs are on. Move one in the All tab and it appears here."));
            }
            egui::ScrollArea::vertical().id_salt("morphs-active").max_height(400.0).show(ui, |ui| {
                for id in &active_ids {
                    let mut v = ed.scene.morph_value(asset, id);
                    let (lo, hi) = slider_range(ed, asset, id);
                    if ui.add(egui::Slider::new(&mut v, lo..=hi).text(id).max_decimals(3)).changed()
                        && let Some(f) = ed.vam.files.get(&(asset, id.clone())).cloned().or_else(|| lib.resolve_morph(id, gender).cloned())
                    {
                        set = Some((f, v));
                    }
                }
            });
        }
        MorphTab::All => {
            ui.add(egui::TextEdit::singleline(&mut ed.vam.filter).hint_text(tl!("Search morphs…")).desired_width(210.0));
            let wanted = ed.vam.filter.trim().to_lowercase();
            let shown: Vec<usize> = if wanted.is_empty() { ed.vam.sorted.clone() } else { ed.vam.sorted.iter().copied().filter(|&i| lib.morphs.get(i).is_some_and(|m| m.stem.to_lowercase().contains(&wanted))).collect() };
            let row = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical().id_salt("morphs-all").max_height(400.0).show_rows(ui, row, shown.len(), |ui, range| {
                for &i in shown.get(range).unwrap_or(&[]) {
                    let Some(f) = lib.morphs.get(i) else { continue };
                    let id = ed.vam.ids.get(&(asset, f.stem.clone())).cloned().unwrap_or_else(|| f.stem.clone());
                    let mut v = ed.scene.morph_value(asset, &id);
                    let name = if f.genital { format!("{} (genital)", f.stem) } else { f.stem.clone() };
                    let label = if f.package.is_empty() { name } else { format!("{name}  ·  {}", f.package) };
                    let (lo, hi) = slider_range(ed, asset, &id);
                    if ui.add(egui::Slider::new(&mut v, lo..=hi).text(label).max_decimals(3)).changed() {
                        set = Some((f.clone(), v));
                    }
                }
            });
        }
    }
    if let Some((file, value)) = set {
        match ensure_loaded(ed, asset, gender, &file).and_then(|id| {
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
