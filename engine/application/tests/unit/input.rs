use super::*;

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
            sampled = Some(sample_actions(&response, active, crate::Camera::default()));
        },
    );
    output.textures_delta.clear();
    sampled.unwrap()
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
fn following_preserves_camera_offset_and_initially_focuses_the_character() {
    let mut camera = crate::Camera::default();
    let offset = camera.eye - camera.target;
    follow_target(&mut camera, None, Some([2., 1., 3.]));
    assert_eq!(camera.target, comet_core::math::Vec3::new(2., 1., 3.));
    follow_target(&mut camera, Some([2., 1., 3.]), Some([4., 2., 5.]));
    assert_eq!(camera.target, comet_core::math::Vec3::new(4., 2., 5.));
    assert!((camera.eye - camera.target - offset).length() < 1e-6);
}
