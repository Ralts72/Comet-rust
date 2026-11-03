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
mod tests;
