use anyhow::{Result, ensure};

/// Standard gameplay actions. Window keys and UI widgets belong to the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Jump,
    Run,
    Crouch,
    Interact,
    Primary,
}
impl Action {
    pub const ALL: [Self; 5] = [
        Self::Jump,
        Self::Run,
        Self::Crouch,
        Self::Interact,
        Self::Primary,
    ];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions(u8);
impl Actions {
    pub fn contains(self, action: Action) -> bool {
        self.0 & (1 << action as u8) != 0
    }
    pub fn set(&mut self, action: Action, held: bool) {
        let bit = 1 << action as u8;
        if held {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionEvent {
    pub action: Action,
    pub pressed: bool,
}

/// Ephemeral host input, not an authoring or asset format. Events keep their order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionInput {
    pub enabled: bool,
    pub movement: [f32; 2],
    pub look_delta: [f32; 2],
    pub held: Actions,
    pub events: Vec<ActionEvent>,
}
pub const MAX_INPUT_EVENTS: usize = 256;
impl ActionInput {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.movement.iter().all(|v| v.is_finite() && v.abs() <= 1.),
            "movement must be finite and within [-1, 1]"
        );
        ensure!(
            self.look_delta.iter().all(|v| v.is_finite()),
            "look delta must be finite"
        );
        ensure!(
            self.events.len() <= MAX_INPUT_EVENTS,
            "too many input events"
        );
        Ok(())
    }
}
