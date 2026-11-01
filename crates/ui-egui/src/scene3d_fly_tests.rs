use photocraft_scene3d::glam::Vec3;

use super::*;

fn editor() -> Scene3dEditor {
    let mut ed = Scene3dEditor::default();
    ed.scene.camera = Camera { position: [0.0, 1.0, 5.0], target: [0.0, 1.0, 0.0], ..Camera::default() };
    ed.orbit = Orbit::from_camera(&ed.scene.camera);
    ed
}

/// Presses `key` with `modifiers` for one frame and runs the module's key handler; true when it used the key.
fn press(ed: &mut Scene3dEditor, key: Key, modifiers: Modifiers) -> bool {
    let ctx = egui::Context::default();
    let ev = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let input = egui::RawInput { events: vec![ev], ..Default::default() };
    let mut used = false;
    ctx.run_ui(input, |ui| used = keys(ed, ui.ctx())).textures_delta.clear();
    used
}

fn pos(ed: &Scene3dEditor) -> Vec3 {
    Vec3::from(ed.scene.camera.position)
}

fn same(a: &Camera, b: &Camera) -> bool {
    Vec3::from(a.position).distance(Vec3::from(b.position)) < 1e-4 && Vec3::from(a.target).distance(Vec3::from(b.target)) < 1e-4 && (a.roll - b.roll).abs() < 1e-4 && (a.fov - b.fov).abs() < 1e-4
}

#[test]
fn tab_switches_flying_and_the_fly_keys_are_free_when_not_flying() {
    let mut ed = editor();
    assert!(!ed.fly.active);
    // Not flying: W A S D Z X C V are not ours (they pick tools elsewhere).
    for key in [Key::W, Key::A, Key::S, Key::D, Key::Z, Key::X, Key::C, Key::V] {
        assert!(!press(&mut ed, key, Modifiers::NONE), "{key:?}");
    }
    assert!(press(&mut ed, Key::Tab, Modifiers::NONE));
    assert!(ed.fly.active);
    // Flying: they are taken.
    for key in [Key::W, Key::A, Key::S, Key::D, Key::Z, Key::X, Key::C, Key::V] {
        assert!(press(&mut ed, key, Modifiers::NONE), "{key:?}");
    }
    assert!(press(&mut ed, Key::Tab, Modifiers::NONE));
    assert!(!ed.fly.active);
    // Ctrl+Tab belongs to the document switcher.
    assert!(!press(&mut ed, Key::Tab, Modifiers::COMMAND));
    assert!(!ed.fly.active);
}

#[test]
fn moving_goes_along_the_view_by_speed_times_time_and_keeps_the_orbit_in_step() {
    let mut ed = editor();
    ed.fly.speed = 4.0;
    let before = pos(&ed);
    assert!(step(&mut ed, FlyInput { local: Vec3::new(0.0, 0.0, 1.0), roll: 0.0 }, 0.5));
    // Looking down −Z: forward is −Z, 4 units a second for half a second.
    assert!((pos(&ed) - (before + Vec3::new(0.0, 0.0, -2.0))).length() < 1e-4, "{:?}", pos(&ed));
    // The target moves with the camera: still 5 units ahead.
    assert!((Vec3::from(ed.scene.camera.target) - Vec3::new(0.0, 1.0, -2.0)).length() < 1e-4, "the view direction and distance do not change");
    assert_eq!(ed.orbit, Orbit::from_camera(&ed.scene.camera), "an orbit drag afterwards continues from here");
    // Right is +X, up is the world's up (Z up, X down), diagonals are not faster.
    let before = pos(&ed);
    step(&mut ed, FlyInput { local: Vec3::new(1.0, 0.0, 0.0), roll: 0.0 }, 0.25);
    assert!((pos(&ed) - (before + Vec3::X)).length() < 1e-4);
    let before = pos(&ed);
    step(&mut ed, FlyInput { local: Vec3::new(0.0, 1.0, 0.0), roll: 0.0 }, 0.25);
    assert!((pos(&ed) - (before + Vec3::Y)).length() < 1e-4);
    let before = pos(&ed);
    step(&mut ed, FlyInput { local: Vec3::new(1.0, 0.0, 1.0), roll: 0.0 }, 1.0);
    assert!(((pos(&ed) - before).length() - 4.0).abs() < 1e-3);
}

#[test]
fn no_input_bad_time_or_bad_numbers_move_nothing() {
    let mut ed = editor();
    let start = ed.scene.camera.clone();
    let go = FlyInput { local: Vec3::Z, roll: 1.0 };
    assert!(!step(&mut ed, FlyInput::default(), 0.1));
    for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(!step(&mut ed, go, dt));
    }
    assert!(!step(&mut ed, FlyInput { local: Vec3::new(f32::NAN, 0.0, 0.0), roll: 0.0 }, 0.1));
    assert_eq!(ed.scene.camera, start);
    scale_speed(&mut ed.fly, f32::NAN);
    scale_speed(&mut ed.fly, -2.0);
    assert_eq!(ed.fly.speed, 2.0);
    scale_speed(&mut ed.fly, 1e9);
    assert_eq!(ed.fly.speed, 50.0);
    scale_speed(&mut ed.fly, 1e-9);
    assert_eq!(ed.fly.speed, 0.1);
}

#[test]
fn shift_c_and_v_bank_by_exactly_fifteen_degrees_a_press() {
    let mut ed = editor();
    ed.fly.active = true;
    let shift = Modifiers::SHIFT;
    assert!(press(&mut ed, Key::V, shift));
    assert_eq!(ed.scene.camera.roll, 15.0);
    assert!(press(&mut ed, Key::V, shift));
    assert_eq!(ed.scene.camera.roll, 30.0);
    assert!(press(&mut ed, Key::C, shift));
    assert_eq!(ed.scene.camera.roll, 15.0);
    assert!(press(&mut ed, Key::C, shift));
    assert!(press(&mut ed, Key::C, shift));
    assert_eq!(ed.scene.camera.roll, -15.0);
    // A plain press (no Shift) does not step: the held key banks smoothly instead.
    assert!(press(&mut ed, Key::C, Modifiers::NONE));
    assert_eq!(ed.scene.camera.roll, -15.0);
    // From a free bank it lands on the next round number; it wraps past 180.
    assert_eq!(roll_step(7.3, 1.0), 15.0);
    assert_eq!(roll_step(7.3, -1.0), 0.0);
    assert_eq!(roll_step(180.0, 1.0), -165.0);
    assert_eq!(roll_step(-180.0 + 15.0, -1.0), 180.0);
    assert_eq!(roll_step(f32::NAN, 1.0), 15.0);
}

#[test]
fn a_held_c_or_v_banks_smoothly_and_the_view_direction_stays() {
    let mut ed = editor();
    let dir = ed.scene.camera.axes().2;
    step(&mut ed, FlyInput { local: Vec3::ZERO, roll: 1.0 }, 0.5);
    assert!((ed.scene.camera.roll - 30.0).abs() < 1e-3);
    step(&mut ed, FlyInput { local: Vec3::ZERO, roll: -1.0 }, 1.0);
    assert!((ed.scene.camera.roll + 30.0).abs() < 1e-3);
    assert!((ed.scene.camera.axes().2 - dir).length() < 1e-5);
    set_roll(&mut ed, 400.0);
    assert!((ed.scene.camera.roll - 40.0).abs() < 1e-3);
    set_roll(&mut ed, f32::NAN);
    assert!((ed.scene.camera.roll - 40.0).abs() < 1e-3);
}

#[test]
fn looking_turns_the_view_about_the_camera_not_about_the_target() {
    let mut ed = editor();
    let eye = pos(&ed);
    let dist = ed.orbit.distance;
    look(&mut ed.orbit, 30.0, -10.0);
    ed.orbit.apply(&mut ed.scene.camera);
    assert!((pos(&ed) - eye).length() < 1e-3, "the camera stays where it is: {:?}", pos(&ed));
    assert!((ed.orbit.distance - dist).abs() < 1e-6);
    assert!(Vec3::from(ed.scene.camera.target).distance(Vec3::new(0.0, 1.0, 0.0)) > 1.0, "the view turned");
    let before = ed.orbit.clone();
    look(&mut ed.orbit, f32::NAN, 1.0);
    assert_eq!(ed.orbit, before);
    look(&mut ed.orbit, 0.0, 1000.0);
    assert!(ed.orbit.pitch <= 89.0);
}

#[test]
fn ctrl_number_saves_the_camera_and_the_number_goes_back_to_it() {
    let mut ed = editor();
    let one = ed.scene.camera.clone();
    assert!(press(&mut ed, Key::Num1, Modifiers::COMMAND));
    assert_eq!(ed.scene.view(0), Some(&one));
    // Fly somewhere else, bank, save as 2.
    ed.fly.active = true;
    step(&mut ed, FlyInput { local: Vec3::new(1.0, 1.0, 1.0), roll: 0.0 }, 1.0);
    set_roll(&mut ed, 15.0);
    let two = ed.scene.camera.clone();
    assert!(press(&mut ed, Key::Num2, Modifiers::COMMAND));
    assert_eq!(ed.scene.view(1), Some(&two));
    // 1 goes back (a glide, then exactly the saved camera), 2 returns.
    assert!(press(&mut ed, Key::Num1, Modifiers::NONE));
    assert!(ed.tween.is_some());
    assert!(!advance_tween(&mut ed, 1.0), "the glide is over");
    assert!(ed.tween.is_none());
    assert_eq!(ed.scene.camera, one);
    assert_eq!(ed.orbit, Orbit::from_camera(&one), "the orbit follows, so the next drag continues from the view");
    assert!(press(&mut ed, Key::Num2, Modifiers::NONE));
    advance_tween(&mut ed, 1.0);
    assert!(same(&ed.scene.camera, &two));
}

#[test]
fn the_glide_goes_between_the_two_cameras_and_a_new_move_stops_it() {
    let mut ed = editor();
    let a = ed.scene.camera.clone();
    step(&mut ed, FlyInput { local: Vec3::new(1.0, 0.0, 0.0), roll: 0.0 }, 1.0);
    set_roll(&mut ed, 90.0);
    let b = ed.scene.camera.clone();
    ed.scene.save_view(3, &b);
    ed.scene.camera = a.clone();
    ed.orbit = Orbit::from_camera(&a);
    recall_view(&mut ed, 3);
    advance_tween(&mut ed, 0.125);
    let mid = ed.scene.camera.clone();
    assert!(mid.roll > 0.0 && mid.roll < 90.0, "{}", mid.roll);
    assert!(pos(&ed).x > a.position[0] && pos(&ed).x < b.position[0]);
    assert!(ed.tween.is_some());
    // The user moves: the glide gives way.
    step(&mut ed, FlyInput { local: Vec3::Z, roll: 0.0 }, 0.1);
    assert!(ed.tween.is_none());
    assert!(!advance_tween(&mut ed, 1.0));
}

#[test]
fn an_empty_slot_changes_nothing_and_says_so() {
    let mut ed = editor();
    let start = ed.scene.camera.clone();
    assert!(press(&mut ed, Key::Num5, Modifiers::NONE));
    assert!(ed.tween.is_none());
    assert_eq!(ed.scene.camera, start);
    assert!(ed.status.contains('5'), "{}", ed.status);
    // A saved view with hostile numbers glides without producing NaN.
    ed.scene.save_view(0, &Camera { position: [f32::NAN, 0.0, 0.0], target: [0.0; 3], fov: f32::INFINITY, roll: f32::NAN, ..Camera::default() });
    recall_view(&mut ed, 0);
    advance_tween(&mut ed, 0.1);
    assert!(ed.scene.camera.position.iter().all(|c| c.is_finite()) && ed.scene.camera.target.iter().all(|c| c.is_finite()));
    assert!(ed.scene.camera.roll.is_finite());
}

#[test]
fn views_and_banking_are_not_undo_steps() {
    let mut ed = editor();
    save_view(&mut ed, 0);
    set_roll(&mut ed, 30.0);
    crate::scene3d_canvas::step_history(&mut ed, true);
    assert_eq!(ed.scene.camera.roll, 30.0);
    assert!(ed.scene.view(0).is_some(), "an undo does not take a saved view away");
}
