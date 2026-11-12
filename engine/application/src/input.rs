use comet_core::{Action, ActionEvent, ActionInput, Actions};

/// Keep a paste intent even when the system clipboard has no text. Text widgets still consume
/// the original platform Paste event; a dedicated key pair never inserts empty text.
pub(crate) fn preserve_paste_shortcut(
    raw: &mut egui::RawInput,
    logical: &winit::keyboard::Key,
    physical: winit::keyboard::PhysicalKey,
    fallback: egui::Modifiers,
    windows: bool,
) -> bool {
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
    let modifiers = raw
        .events
        .iter()
        .rev()
        .find_map(|event| match event {
            egui::Event::ModifiersChanged(modifiers) => Some(*modifiers),
            _ => None,
        })
        .unwrap_or(fallback);
    let key = match logical {
        Key::Named(NamedKey::Paste) => Some(egui::Key::Paste),
        Key::Named(NamedKey::Insert) => Some(egui::Key::Insert),
        Key::Character(text) => egui::Key::from_name(text),
        _ => None,
    }
    .or(match physical {
        PhysicalKey::Code(KeyCode::KeyV) => Some(egui::Key::V),
        PhysicalKey::Code(KeyCode::Paste) => Some(egui::Key::Paste),
        PhysicalKey::Code(KeyCode::Insert) => Some(egui::Key::Insert),
        _ => None,
    });
    if key == Some(egui::Key::Paste)
        || (key == Some(egui::Key::V) && modifiers.command_only())
        || (windows && key == Some(egui::Key::Insert) && modifiers.shift)
    {
        // This is an intent, not a held gameplay key. Finish it in the same event batch.
        raw.events
            .extend([true, false].map(|pressed| egui::Event::Key {
                key: egui::Key::Paste,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }));
        true
    } else {
        false
    }
}

/// Replace only the current native key's window-local fallback, preserving earlier paste events.
pub(crate) fn replace_paste_text(raw: &mut egui::RawInput, before: usize, text: Option<String>) {
    for index in (before..raw.events.len()).rev() {
        if matches!(raw.events[index], egui::Event::Paste(_)) {
            raw.events.remove(index);
        }
    }
    if let Some(text) = text.filter(|text| !text.is_empty()) {
        raw.events
            .push(egui::Event::Paste(text.replace("\r\n", "\n")));
    }
}

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

fn primary_event(event: &egui::Event, rect: egui::Rect) -> Option<(usize, bool)> {
    match event {
        egui::Event::Key {
            key: egui::Key::J,
            pressed,
            repeat: false,
            ..
        } => Some((0, *pressed)),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            ..
        } if rect.contains(*pos) => Some((1, *pressed)),
        _ => None,
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
        let mut primary_sources = [
            input.key_down(egui::Key::J),
            input.pointer.primary_down()
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|p| response.rect.contains(p)),
        ];
        held.set(Action::Primary, primary_sources[0] || primary_sources[1]);
        // Recover the beginning of this event batch, then emit edges of the combined action.
        for event in input.events.iter().rev() {
            if let Some((source, pressed)) = primary_event(event, response.rect) {
                primary_sources[source] = !pressed;
            }
        }
        let events = input
            .events
            .iter()
            .filter_map(|event| {
                if let Some((source, pressed)) = primary_event(event, response.rect) {
                    let before = primary_sources[0] || primary_sources[1];
                    primary_sources[source] = pressed;
                    let after = primary_sources[0] || primary_sources[1];
                    return (before != after).then_some(ActionEvent {
                        action: Action::Primary,
                        pressed: after,
                    });
                }
                match event {
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
                    _ => None,
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

    #[test]
    fn paste_intents_follow_event_modifiers_and_layout_without_replacing_platform_text() {
        let physical = PhysicalKey::Code(KeyCode::KeyV);
        for (logical, key, modifiers, windows, expected) in [
            (
                Key::Character("v".into()),
                physical,
                egui::Modifiers::COMMAND,
                false,
                true,
            ),
            (
                Key::Character("V".into()),
                physical,
                egui::Modifiers::COMMAND,
                true,
                true,
            ),
            (
                Key::Character("м".into()),
                physical,
                egui::Modifiers::COMMAND,
                false,
                true,
            ),
            (
                Key::Character("f".into()),
                physical,
                egui::Modifiers::COMMAND,
                false,
                false,
            ),
            (
                Key::Character("v".into()),
                physical,
                egui::Modifiers::NONE,
                false,
                false,
            ),
            (
                Key::Character("v".into()),
                physical,
                egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT),
                false,
                false,
            ),
            (
                Key::Named(NamedKey::Paste),
                PhysicalKey::Code(KeyCode::Paste),
                egui::Modifiers::NONE,
                false,
                true,
            ),
            (
                Key::Named(NamedKey::Insert),
                PhysicalKey::Code(KeyCode::Insert),
                egui::Modifiers::SHIFT,
                true,
                true,
            ),
            (
                Key::Named(NamedKey::Insert),
                PhysicalKey::Code(KeyCode::Insert),
                egui::Modifiers::SHIFT,
                false,
                false,
            ),
        ] {
            for platform in [None, Some(egui::Event::Paste("text".into()))] {
                let mut raw = egui::RawInput::default();
                raw.events.push(egui::Event::ModifiersChanged(modifiers));
                raw.events.extend(platform.clone());
                let before = raw.events.clone();
                let matched =
                    preserve_paste_shortcut(&mut raw, &logical, key, egui::Modifiers::ALT, windows);
                assert_eq!(matched, expected);
                assert_eq!(&raw.events[..before.len()], before);
                assert_eq!(
                    raw.events.len(),
                    before.len() + if expected { 2 } else { 0 }
                );
                if expected {
                    for (event, pressed) in raw.events[before.len()..].iter().zip([true, false]) {
                        assert!(
                            matches!(event, egui::Event::Key { key: egui::Key::Paste, pressed: p, modifiers: m, .. } if *p == pressed && *m == modifiers)
                        );
                    }
                }
            }
        }
        let mut raw = egui::RawInput::default();
        preserve_paste_shortcut(
            &mut raw,
            &Key::Character("v".into()),
            physical,
            egui::Modifiers::COMMAND,
            false,
        );
        assert_eq!(
            raw.events.len(),
            2,
            "retain modifiers when the frame has no modifier changes"
        );
        let intent = raw.events.clone();
        raw.events.insert(0, egui::Event::Paste("earlier".into()));
        raw.events
            .insert(1, egui::Event::Paste("stale fallback".into()));
        replace_paste_text(&mut raw, 1, Some(String::new()));
        assert_eq!(&raw.events[1..], intent);
        assert_eq!(raw.events[0], egui::Event::Paste("earlier".into()));
        replace_paste_text(&mut raw, 1, Some("a\r\nb".into()));
        assert_eq!(raw.events.last(), Some(&egui::Event::Paste("a\nb".into())));
        replace_paste_text(&mut raw, 1, None);
        assert_eq!(&raw.events[1..], intent);
    }
}
