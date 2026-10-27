//! The Color panel's settings (sliders, mixer, wheel, lock) remembered across restarts
//! (fork-local, see MY_CHANGES.md). Same scheme as `brush_picker`: a JSON slot in the preferences,
//! written once the pointer is up and read back at launch.

use serde_json::Value;

use crate::PhotocraftApp;
use crate::state::ColorPanelState;

/// The setting that is kept; the editing chip and the mixer's last-seen colours are transient.
fn view(app: &PhotocraftApp) -> Value {
    let st = ColorPanelState { background: false, mixer_seen: [[0.0; 4]; 2], ..app.ui.color_panel };
    serde_json::to_value(st).unwrap_or(Value::Null)
}

/// Remember the panel's settings in the preferences when they change. Run every frame from the
/// frame loop (a small JSON compare), so a setting changed by the control channel counts too.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let now = view(app);
    if now != Value::Null && app.session.prefs().color_panel != now {
        app.session.prefs.edit(|p| p.color_panel = now);
    }
}

/// Restore the remembered settings at launch. Nothing saved, or something malformed, keeps the
/// defaults; the mixer's step count and colours are brought back into range.
pub fn restore(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().color_panel.clone();
    let Ok(mut st) = serde_json::from_value::<ColorPanelState>(saved) else { return };
    st.background = false;
    st.mixer_seen = [[0.0; 4]; 2];
    if st.mixer_steps != 0 {
        st.mixer_steps = st.mixer_steps.clamp(crate::color_mixer_ui::MIN_STEPS, crate::color_mixer_ui::MAX_STEPS);
    }
    if !st.mixer_ends.iter().flatten().all(|v| v.is_finite()) {
        st.mixer_ends = ColorPanelState::default().mixer_ends;
        st.mixer_fixed = false;
    }
    app.ui.color_panel = st;
}
