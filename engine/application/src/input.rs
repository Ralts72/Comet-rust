use comet_core::{Action, ActionEvent, ActionInput, Actions};

fn keyboard_authorized(response: &egui::Response, active: bool) -> bool {
    active
        && (response.has_focus() || !response.ctx.egui_wants_keyboard_input())
        && response.ctx.input(|input| input.focused)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointAction {
    Save,
    Continue,
}

/// Host shortcuts run independently of simulation ticks, including while paused.
#[derive(Default)]
pub struct CheckpointInput {
    held: [bool; 2],
    blocked: [bool; 2],
    frame: Option<u64>,
}
impl CheckpointInput {
    /// `active` includes the host's final modal/busy authorization for this frame.
    pub fn sample(
        &mut self,
        response: &egui::Response,
        active: bool,
        activity_changed: bool,
    ) -> Option<CheckpointAction> {
        let authorized = keyboard_authorized(response, active) && !activity_changed;
        let frame = response.ctx.cumulative_frame_nr();
        let duplicate = self.frame.replace(frame) == Some(frame);
        response.ctx.input(|input| {
            let keys = [egui::Key::F5, egui::Key::F9];
            if !authorized {
                self.held = keys.map(|key| input.key_down(key));
                self.blocked = self.held;
                return None;
            }
            if duplicate {
                return None;
            }
            let mut action = None;
            for event in &input.events {
                let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    ..
                } = event
                else {
                    continue;
                };
                let Some(index) = keys.iter().position(|candidate| candidate == key) else {
                    continue;
                };
                if *pressed {
                    if !repeat && !self.held[index] && !self.blocked[index] && action.is_none() {
                        action = Some([CheckpointAction::Save, CheckpointAction::Continue][index]);
                    }
                    self.held[index] = true;
                } else {
                    self.held[index] = false;
                    self.blocked[index] = false;
                }
            }
            for (index, key) in keys.into_iter().enumerate() {
                if !input.key_down(key) {
                    self.held[index] = false;
                    self.blocked[index] = false;
                } else if !self.held[index] {
                    // A key restored without a fresh event must be released before use.
                    self.held[index] = true;
                    self.blocked[index] = true;
                }
            }
            action
        })
    }
}

/// Default keyboard/mouse mapping shared by Editor Play and App.
pub fn sample_actions(
    response: &egui::Response,
    active: bool,
    camera: crate::Camera,
) -> ActionInput {
    let ctx = &response.ctx;
    if !keyboard_authorized(response, active) {
        return ActionInput::default();
    }
    // Response queries may lock Context too; complete them before borrowing InputState.
    let looking = response.dragged_by(egui::PointerButton::Secondary);
    let mut actions = ctx.input(|input| {
        let mut held = Actions::default();
        held.set(Action::Jump, input.key_down(egui::Key::Space));
        held.set(Action::Run, input.modifiers.shift);
        held.set(Action::Crouch, input.modifiers.ctrl);
        held.set(Action::Interact, input.key_down(egui::Key::E));
        held.set(Action::Restart, input.key_down(egui::Key::R));
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
                        egui::Key::R => Action::Restart,
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
        let look_delta = if looking {
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
