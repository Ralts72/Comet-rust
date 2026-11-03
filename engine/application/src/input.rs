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
#[path = "../tests/unit/input.rs"]
mod tests;
