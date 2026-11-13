//! Native devices stay in Application; consumers see only one ephemeral logical frame.
use comet_core::{Action, ActionEvent, ActionInput, Actions};

mod native;
pub(crate) use native::GamepadHost;

#[derive(Clone, Debug, Default)]
pub struct GamepadFrame {
    pub actions: ActionInput,
    /// Right stick in pointer-equivalent units per second; integrate with host dt once.
    pub look_rate: [f32; 2],
    pub pause_down: bool,
    pub pause_pressed: bool,
    pub pause_released: bool,
}

impl GamepadFrame {
    pub fn look_delta(&self, dt: f64) -> [f32; 2] {
        if !self.actions.enabled || !dt.is_finite() || dt <= 0. {
            return [0.; 2];
        }
        self.look_rate.map(|v| {
            if v.is_finite() {
                (f64::from(v) * dt.min(0.1)) as f32
            } else {
                0.
            }
        })
    }
}

/// Pause is a host command, independent of simulation ticks and the gameplay buffer.
#[derive(Default)]
pub struct GamepadPauseInput {
    blocked: bool,
    frame: Option<u64>,
}
impl GamepadPauseInput {
    pub fn sample(
        &mut self,
        ctx: &egui::Context,
        gamepad: &GamepadFrame,
        authorized: bool,
    ) -> bool {
        let duplicate =
            self.frame.replace(ctx.cumulative_frame_nr()) == Some(ctx.cumulative_frame_nr());
        if !authorized || !ctx.input(|input| input.focused) {
            self.blocked = gamepad.pause_down;
            return false;
        }
        if duplicate {
            return false;
        }
        if gamepad.pause_released || !gamepad.pause_down {
            self.blocked = false;
        }
        gamepad.pause_pressed && !self.blocked
    }
}

#[derive(Default)]
struct State {
    held: u8,
    blocked: u8,
    sticks: [[f32; 2]; 2],
    stick_blocked: [bool; 2],
    frame: GamepadFrame,
}
impl State {
    fn synchronize(&mut self, held: u8, sticks: [[f32; 2]; 2]) {
        // A connected/replaced device must release each held button and center each stick.
        self.held = held;
        self.blocked = held;
        self.sticks = sticks;
        self.stick_blocked = sticks.map(|stick| deadzone(stick) != [0.; 2]);
        self.frame = GamepadFrame::default();
    }
    fn button(&mut self, index: usize, pressed: bool) {
        let mask = 1 << index;
        let before = self.held & mask != 0 && self.blocked & mask == 0;
        if pressed {
            self.held |= mask;
        } else {
            self.held &= !mask;
            self.blocked &= !mask;
        }
        let pressed = pressed && self.blocked & mask == 0;
        if before == pressed {
            return;
        }
        if index == Action::ALL.len() {
            self.frame.pause_pressed |= pressed;
            self.frame.pause_released |= !pressed;
        } else {
            self.frame.actions.events.push(ActionEvent {
                action: Action::ALL[index],
                pressed,
            });
        }
    }
    fn axis(&mut self, stick: usize, axis: usize, value: f32) {
        self.sticks[stick][axis] = if value.is_finite() {
            value.clamp(-1., 1.)
        } else {
            0.
        };
        if deadzone(self.sticks[stick]) == [0.; 2] {
            self.stick_blocked[stick] = false;
        }
    }
    fn take(&mut self) -> GamepadFrame {
        let held = self.held & !self.blocked;
        let mut actions = Actions::default();
        for (index, action) in Action::ALL.into_iter().enumerate() {
            actions.set(action, held & (1 << index) != 0);
        }
        self.frame.actions.enabled = true;
        self.frame.actions.held = actions;
        self.frame.actions.movement = if self.stick_blocked[0] {
            [0.; 2]
        } else {
            deadzone(self.sticks[0])
        };
        let look = if self.stick_blocked[1] {
            [0.; 2]
        } else {
            deadzone(self.sticks[1])
        };
        self.frame.look_rate = [look[0] * 600., -look[1] * 600.];
        self.frame.pause_down = held & (1 << Action::ALL.len()) != 0;
        std::mem::take(&mut self.frame)
    }
}

fn deadzone(value: [f32; 2]) -> [f32; 2] {
    const DEADZONE: f32 = 0.15;
    let value = value.map(|v| if v.is_finite() { v.clamp(-1., 1.) } else { 0. });
    let length = value[0].hypot(value[1]);
    if length <= DEADZONE {
        return [0.; 2];
    }
    let scale = ((length - DEADZONE) / (1. - DEADZONE)).min(1.) / length;
    value.map(|v| v * scale)
}

#[cfg(test)]
mod tests;
