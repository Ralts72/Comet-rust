use anyhow::{Result, ensure};
use comet_core::{Action, ActionEvent, ActionInput, MAX_INPUT_EVENTS};

pub(crate) struct InputBuffer {
    pending: ActionInput,
    needs_baseline: bool,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self {
            pending: ActionInput::default(),
            needs_baseline: true,
        }
    }
}
impl InputBuffer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Paused frames retain current holds, never actions from before a manual step.
    pub fn synchronize(&mut self, mut sample: ActionInput) -> Result<()> {
        sample.validate()?;
        if !sample.enabled {
            sample = ActionInput::default();
        }
        sample.events.clear();
        sample.look_delta = [0.; 2];
        self.pending = sample;
        self.needs_baseline = true;
        Ok(())
    }
    pub fn sample(&mut self, sample: ActionInput) -> Result<()> {
        sample.validate()?;
        if !sample.enabled || self.needs_baseline {
            let enabled = sample.enabled;
            self.synchronize(sample)?;
            self.needs_baseline = !enabled;
            return Ok(());
        }
        let mut next = self.pending.clone();
        next.movement = sample.movement;
        for (total, delta) in next.look_delta.iter_mut().zip(sample.look_delta) {
            *total += delta;
            ensure!(total.is_finite(), "accumulated look delta exhausted");
        }
        for event in sample.events {
            Self::transition(&mut next, event);
        }
        // Reconcile held state too: modifiers may have no key event, and a host can
        // lose a release event. Repeat key-down events never create extra presses.
        for action in Action::ALL {
            Self::transition(
                &mut next,
                ActionEvent {
                    action,
                    pressed: sample.held.contains(action),
                },
            );
        }
        ensure!(
            next.events.len() <= MAX_INPUT_EVENTS,
            "pending input event limit exceeded"
        );
        self.pending = next;
        Ok(())
    }
    fn transition(next: &mut ActionInput, event: ActionEvent) {
        if next.held.contains(event.action) != event.pressed {
            next.held.set(event.action, event.pressed);
            next.events.push(event);
        }
    }
    pub fn tick(&mut self) -> ActionInput {
        ActionInput {
            enabled: self.pending.enabled,
            movement: self.pending.movement,
            look_delta: std::mem::replace(&mut self.pending.look_delta, [0.; 2]),
            held: self.pending.held,
            events: std::mem::take(&mut self.pending.events),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use comet_core::Actions;

    fn sample() -> ActionInput {
        ActionInput {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn ordered_edges_and_mouse_delta_are_delivered_only_to_the_first_tick() -> Result<()> {
        let mut buffer = InputBuffer::default();
        buffer.sample(sample())?;
        let events = vec![
            ActionEvent {
                action: Action::Jump,
                pressed: true,
            },
            ActionEvent {
                action: Action::Jump,
                pressed: false,
            },
        ];
        buffer.sample(ActionInput {
            events: events.clone(),
            look_delta: [2., 3.],
            ..sample()
        })?;
        let mut held = Actions::default();
        held.set(Action::Run, true);
        buffer.sample(ActionInput {
            movement: [0., 1.],
            held,
            look_delta: [4., -1.],
            ..sample()
        })?;
        let tick = buffer.tick();
        assert_eq!(&tick.events[..2], events);
        assert_eq!(
            tick.events[2],
            ActionEvent {
                action: Action::Run,
                pressed: true
            }
        );
        assert_eq!(tick.look_delta, [6., 2.]);
        for _ in 0..3 {
            let tick = buffer.tick();
            assert!(tick.events.is_empty());
            assert_eq!(tick.look_delta, [0.; 2]);
            assert_eq!(tick.movement, [0., 1.]);
            assert!(tick.held.contains(Action::Run));
        }
        buffer.sample(ActionInput {
            held,
            events: vec![ActionEvent {
                action: Action::Run,
                pressed: true,
            }],
            ..sample()
        })?;
        assert!(buffer.tick().events.is_empty());
        Ok(())
    }

    #[test]
    fn queue_and_delta_overflow_keep_the_accepted_input() -> Result<()> {
        let mut buffer = InputBuffer::default();
        buffer.sample(sample())?;
        let events = vec![
            ActionEvent {
                action: Action::Interact,
                pressed: true,
            },
            ActionEvent {
                action: Action::Interact,
                pressed: false,
            },
        ];
        for _ in 0..MAX_INPUT_EVENTS / 2 {
            buffer.sample(ActionInput {
                events: events.clone(),
                ..sample()
            })?;
        }
        assert!(buffer.sample(ActionInput { events, ..sample() }).is_err());
        assert_eq!(buffer.tick().events.len(), MAX_INPUT_EVENTS);
        buffer.sample(ActionInput {
            look_delta: [f32::MAX, 0.],
            ..sample()
        })?;
        assert!(
            buffer
                .sample(ActionInput {
                    look_delta: [f32::MAX, 0.],
                    ..sample()
                })
                .is_err()
        );
        assert_eq!(buffer.tick().look_delta, [f32::MAX, 0.]);
        Ok(())
    }
}
