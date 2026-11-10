use comet_application::{Camera, follow_target, sample_actions};
use comet_core::{Action, ActionEvent, ActionInput};

fn key(pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Space,
        physical_key: Some(egui::Key::Space),
        pressed,
        repeat,
        modifiers: egui::Modifiers::NONE,
    }
}
fn frame(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    text_focus: bool,
    active: bool,
) -> ActionInput {
    let mut sampled = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            if text_focus {
                let mut value = String::new();
                ui.add(egui::TextEdit::singleline(&mut value))
                    .request_focus();
            }
            let response = ui.allocate_rect(
                ui.available_rect_before_wrap(),
                egui::Sense::click_and_drag(),
            );
            sampled = Some(sample_actions(&response, active, Camera::default()));
        },
    );
    output.textures_delta.clear();
    sampled.unwrap()
}
#[test]
fn primary_aliases_emit_one_logical_edge_while_either_keyboard_or_mouse_remains_held() {
    let key = |pressed| egui::Event::Key {
        key: egui::Key::J,
        physical_key: Some(egui::Key::J),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let pos = egui::pos2(100., 100.);
    let mouse = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for mouse_first in [false, true] {
        let ctx = egui::Context::default();
        frame(&ctx, vec![], false, true);
        let first = if mouse_first { mouse(true) } else { key(true) };
        let second = if mouse_first { key(true) } else { mouse(true) };
        let released = if mouse_first {
            mouse(false)
        } else {
            key(false)
        };
        let last = if mouse_first {
            key(false)
        } else {
            mouse(false)
        };
        let pressed = frame(
            &ctx,
            vec![egui::Event::PointerMoved(pos), first],
            false,
            true,
        );
        assert_eq!(
            pressed.events,
            vec![ActionEvent {
                action: Action::Primary,
                pressed: true
            }]
        );
        for event in [second, released] {
            let sample = frame(&ctx, vec![event], false, true);
            assert!(sample.held.contains(Action::Primary));
            assert!(sample.events.is_empty());
        }
        let sample = frame(&ctx, vec![last], false, true);
        assert!(!sample.held.contains(Action::Primary));
        assert_eq!(
            sample.events,
            vec![ActionEvent {
                action: Action::Primary,
                pressed: false
            }]
        );
    }
    let ctx = egui::Context::default();
    let tap = frame(&ctx, vec![key(true), key(false)], false, true);
    assert_eq!(tap.events.len(), 2);
}

#[test]
fn sampling_preserves_fast_taps_and_skips_key_repeat_events() {
    let ctx = egui::Context::default();
    let input = frame(&ctx, vec![key(true, false), key(false, false)], false, true);
    assert!(input.enabled);
    assert_eq!(
        input.events,
        vec![
            ActionEvent {
                action: Action::Jump,
                pressed: true
            },
            ActionEvent {
                action: Action::Jump,
                pressed: false
            }
        ]
    );
    assert!(!input.held.contains(Action::Jump));
    frame(&ctx, vec![key(true, false)], false, true);
    let input = frame(&ctx, vec![key(true, true)], false, true);
    assert!(input.held.contains(Action::Jump));
    assert!(input.events.is_empty());
}

#[test]
fn restart_mapping_preserves_fresh_r_edges_and_obeys_input_authorization() {
    let r = |pressed, repeat| egui::Event::Key {
        key: egui::Key::R,
        physical_key: Some(egui::Key::R),
        pressed,
        repeat,
        modifiers: egui::Modifiers::NONE,
    };
    let ctx = egui::Context::default();
    let tap = frame(&ctx, vec![r(true, false), r(false, false)], false, true);
    assert_eq!(
        tap.events,
        vec![
            ActionEvent {
                action: Action::Restart,
                pressed: true
            },
            ActionEvent {
                action: Action::Restart,
                pressed: false
            }
        ]
    );
    assert!(!tap.held.contains(Action::Restart));
    frame(&ctx, vec![r(true, false)], false, true);
    let repeated = frame(&ctx, vec![r(true, true)], false, true);
    assert!(repeated.held.contains(Action::Restart));
    assert!(repeated.events.is_empty());
    assert!(!frame(&ctx, vec![r(true, false)], true, true).enabled);
    let ctx = egui::Context::default();
    assert!(!frame(&ctx, vec![r(true, false)], false, false).enabled);
}
#[test]
fn text_editing_and_inactive_windows_do_not_authorize_gameplay() {
    let ctx = egui::Context::default();
    assert!(!frame(&ctx, vec![key(true, false)], true, true).enabled);
    let ctx = egui::Context::default();
    assert!(!frame(&ctx, vec![key(true, false)], false, false).enabled);
}
#[test]
fn movement_uses_the_camera_heading_and_normalizes_diagonals() {
    let ctx = egui::Context::default();
    let press = |key| egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let forward = frame(&ctx, vec![press(egui::Key::W)], false, true);
    assert!(forward.movement[0] < 0. && forward.movement[1] > 0.);
    let diagonal = frame(&ctx, vec![press(egui::Key::D)], false, true);
    assert!((diagonal.movement.iter().map(|v| v * v).sum::<f32>() - 1.).abs() < 1e-6);
}

#[test]
fn primary_and_secondary_drags_sample_without_reentering_context_input() {
    for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        frame(&ctx, vec![], false, true);
        let start = egui::pos2(100., 100.);
        let end = egui::pos2(140., 120.);
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let press = frame(
            &ctx,
            vec![egui::Event::PointerMoved(start), pointer(start, true)],
            false,
            true,
        );
        let dragged = frame(&ctx, vec![egui::Event::PointerMoved(end)], false, true);
        assert!(press.enabled && dragged.enabled);
        if button == egui::PointerButton::Primary {
            assert_eq!(
                press.events,
                vec![ActionEvent {
                    action: Action::Primary,
                    pressed: true
                }]
            );
            assert!(dragged.held.contains(Action::Primary));
            assert_eq!(dragged.look_delta, [0.; 2]);
        } else {
            assert!(press.events.is_empty());
            assert!(!dragged.held.contains(Action::Primary));
            assert_eq!(dragged.look_delta, [40., 20.]);
        }
        let released = frame(&ctx, vec![pointer(end, false)], false, true);
        assert!(!released.held.contains(Action::Primary));
        assert_eq!(released.look_delta, [0.; 2]);
    }
}
#[test]
fn following_preserves_camera_offset_and_initially_focuses_the_character() {
    let mut camera = Camera::default();
    let offset = camera.eye - camera.target;
    follow_target(&mut camera, None, Some([2., 1., 3.]));
    assert_eq!(camera.target, comet_core::math::Vec3::new(2., 1., 3.));
    follow_target(&mut camera, Some([2., 1., 3.]), Some([4., 2., 5.]));
    assert_eq!(camera.target, comet_core::math::Vec3::new(4., 2., 5.));
    assert!((camera.eye - camera.target - offset).length() < 1e-6);
}
