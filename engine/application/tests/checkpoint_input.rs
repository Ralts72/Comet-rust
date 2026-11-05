use comet_application::{CheckpointAction, CheckpointInput};

fn key(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers: egui::Modifiers::NONE,
    }
}

fn sample(
    ctx: &egui::Context,
    gate: &mut CheckpointInput,
    events: Vec<egui::Event>,
    active: bool,
    activity_changed: bool,
    text_focus: bool,
    focused: bool,
) -> Option<CheckpointAction> {
    let mut action = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            focused,
            events,
            ..Default::default()
        },
        |ui| {
            if text_focus {
                ui.add(egui::TextEdit::singleline(&mut String::new()))
                    .request_focus();
            }
            let response = ui.allocate_rect(
                ui.available_rect_before_wrap(),
                egui::Sense::click_and_drag(),
            );
            action = gate.sample(&response, active, activity_changed);
        },
    );
    output.textures_delta.clear();
    action
}

#[test]
fn fast_taps_and_competing_shortcuts_publish_only_the_first_fresh_action() {
    let ctx = egui::Context::default();
    let mut gate = CheckpointInput::default();
    let save = sample(
        &ctx,
        &mut gate,
        vec![
            key(egui::Key::F5, true, false),
            key(egui::Key::F5, false, false),
        ],
        true,
        false,
        false,
        true,
    );
    assert_eq!(save, Some(CheckpointAction::Save));
    assert_eq!(
        sample(
            &ctx,
            &mut gate,
            vec![
                key(egui::Key::F9, true, false),
                key(egui::Key::F5, true, false)
            ],
            true,
            false,
            false,
            true,
        ),
        Some(CheckpointAction::Continue)
    );
    for events in [
        vec![],
        vec![key(egui::Key::F5, true, true)],
        vec![key(egui::Key::F9, true, true)],
    ] {
        assert_eq!(
            sample(&ctx, &mut gate, events, true, false, false, true),
            None
        );
    }
}

#[test]
fn modal_and_activity_transitions_block_held_keys_until_release() {
    for activity_changed in [false, true] {
        let ctx = egui::Context::default();
        let mut gate = CheckpointInput::default();
        assert_eq!(
            sample(
                &ctx,
                &mut gate,
                vec![key(egui::Key::F5, true, false)],
                activity_changed,
                activity_changed,
                false,
                true,
            ),
            None
        );
        // Re-enabling while held cannot turn a repeat or a duplicate press into a save.
        for events in [
            vec![],
            vec![key(egui::Key::F5, true, true)],
            vec![key(egui::Key::F5, true, false)],
        ] {
            assert_eq!(
                sample(&ctx, &mut gate, events, true, false, false, true),
                None
            );
        }
        assert_eq!(
            sample(
                &ctx,
                &mut gate,
                vec![
                    key(egui::Key::F5, false, false),
                    key(egui::Key::F5, true, false)
                ],
                true,
                false,
                false,
                true,
            ),
            Some(CheckpointAction::Save)
        );
    }
}

#[test]
fn text_focus_and_unfocused_windows_do_not_capture_shortcuts() {
    for (text_focus, focused) in [(true, true), (false, false)] {
        let ctx = egui::Context::default();
        let mut gate = CheckpointInput::default();
        assert_eq!(
            sample(
                &ctx,
                &mut gate,
                vec![key(egui::Key::F9, true, false)],
                true,
                false,
                text_focus,
                focused,
            ),
            None
        );
    }
}

#[test]
fn repeated_ui_passes_do_not_duplicate_a_fast_tap() {
    let ctx = egui::Context::default();
    let mut gate = CheckpointInput::default();
    let mut samples = vec![];
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![
                key(egui::Key::F5, true, false),
                key(egui::Key::F5, false, false),
            ],
            ..Default::default()
        },
        |ui| {
            let response = ui.allocate_rect(ui.max_rect(), egui::Sense::click_and_drag());
            samples.push(gate.sample(&response, true, false));
            samples.push(gate.sample(&response, true, false));
        },
    );
    output.textures_delta.clear();
    assert_eq!(samples, vec![Some(CheckpointAction::Save), None]);
}
