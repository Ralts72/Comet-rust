use comet_core::{Action, ActionEvent, ActionInput, Actions};

/// Default keyboard/mouse mapping shared by Editor Play and App.
pub fn sample_actions(
    response: &egui::Response,
    active: bool,
    camera: crate::Camera,
) -> ActionInput {
    let ctx = &response.ctx;
    if !active || (!response.has_focus() && ctx.egui_wants_keyboard_input()) {
        return ActionInput::default();
    }
    let mut actions = ctx.input(|input| {
        if !input.focused {
            return ActionInput::default();
        }
        let mut held = Actions::default();
        held.set(Action::Jump, input.key_down(egui::Key::Space));
        held.set(Action::Run, input.modifiers.shift);
        held.set(Action::Crouch, input.modifiers.ctrl);
        held.set(Action::Interact, input.key_down(egui::Key::E));
        held.set(
            Action::Primary,
            input.pointer.primary_down()
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|p| response.rect.contains(p)),
        );
        let events = input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed,
                    repeat: false,
                    ..
                } => {
                    let action = match key {
                        egui::Key::Space => Action::Jump,
                        egui::Key::E => Action::Interact,
                        _ => return None,
                    };
                    Some(ActionEvent {
                        action,
                        pressed: *pressed,
                    })
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } if response.rect.contains(*pos) => Some(ActionEvent {
                    action: Action::Primary,
                    pressed: *pressed,
                }),
                _ => None,
            })
            .collect();
        let axis = |positive, negative| {
            f32::from(input.key_down(positive)) - f32::from(input.key_down(negative))
        };
        let look_delta = if response.dragged_by(egui::PointerButton::Secondary) {
            let delta = input.pointer.delta();
            [delta.x, delta.y]
        } else {
            [0.; 2]
        };
        ActionInput {
            enabled: true,
            movement: [
                axis(egui::Key::D, egui::Key::A),
                axis(egui::Key::W, egui::Key::S),
            ],
            look_delta,
            held,
            events,
        }
    });
    // Runtime receives movement in world X / negative Z, independent of any camera API.
    let forward = (camera.target - camera.eye) * comet_core::math::Vec3::new(1., 0., 1.);
    let forward = forward
        .try_normalize()
        .unwrap_or(comet_core::math::Vec3::NEG_Z);
    let right = forward.cross(comet_core::math::Vec3::Y);
    let world = (right * actions.movement[0] + forward * actions.movement[1]).clamp_length_max(1.);
    actions.movement = [world.x, -world.z];
    actions
}

/// Follow simulation movement while preserving the user's orbit and zoom.
/// None as the previous target focuses a newly started Play world.
pub fn follow_target(
    camera: &mut crate::Camera,
    previous: Option<[f32; 3]>,
    current: Option<[f32; 3]>,
) {
    if let Some(current) = current {
        let current = comet_core::math::Vec3::from_array(current);
        let previous = previous.map_or(camera.target, comet_core::math::Vec3::from_array);
        let delta = current - previous;
        camera.eye += delta;
        camera.target += delta;
    }
}

#[cfg(test)]
mod tests {
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
}
