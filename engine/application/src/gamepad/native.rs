use super::{GamepadFrame, State};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs, GilrsBuilder};

const BUTTONS: [Button; 7] = [
    Button::South,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::North,
    Button::West,
    Button::Select,
    Button::Start,
];
const AXES: [[Axis; 2]; 2] = [
    [Axis::LeftStickX, Axis::LeftStickY],
    [Axis::RightStickX, Axis::RightStickY],
];
const EVENT_BUDGET: usize = 512;

#[derive(Default)]
pub(crate) struct GamepadHost {
    gilrs: Option<Gilrs>,
    active: Option<GamepadId>,
    state: State,
    synchronize: bool,
    initialized: bool,
    error: Option<String>,
    connected: usize,
    name: Option<String>,
    overflows: u64,
}
impl GamepadHost {
    pub fn open() -> Self {
        match GilrsBuilder::new()
            .with_default_filters(false)
            .with_force_feedback(false)
            .build()
        {
            Ok(gilrs) => Self {
                gilrs: Some(gilrs),
                initialized: true,
                synchronize: true,
                ..Default::default()
            },
            Err(error) => {
                eprintln!("手柄输入不可用：{error}");
                Self {
                    error: Some(error.to_string()),
                    ..Default::default()
                }
            }
        }
    }
    pub fn sample(&mut self, active: bool, changed: bool, smoke: bool) -> GamepadFrame {
        let Some(gilrs) = &mut self.gilrs else {
            return GamepadFrame::default();
        };
        let mut drained = false;
        let mut overflowed = false;
        for _ in 0..EVENT_BUDGET {
            let Some(event) = gilrs.next_event() else {
                drained = true;
                break;
            };
            if event.event == EventType::Connected && self.active.is_none() {
                self.active = Some(event.id);
                self.synchronize = true;
            }
            if Some(event.id) != self.active {
                continue;
            }
            match event.event {
                EventType::Disconnected => {
                    self.active = None;
                    self.synchronize = true;
                }
                EventType::ButtonPressed(button, _) | EventType::ButtonReleased(button, _) => {
                    if let Some(index) = BUTTONS.iter().position(|candidate| *candidate == button) {
                        self.state
                            .button(index, matches!(event.event, EventType::ButtonPressed(..)));
                    }
                }
                EventType::AxisChanged(axis, value, _) => {
                    for (stick, axes) in AXES.into_iter().enumerate() {
                        if let Some(index) = axes.iter().position(|candidate| *candidate == axis) {
                            self.state.axis(stick, index, value);
                        }
                    }
                }
                EventType::Dropped => overflowed = true,
                _ => {}
            }
            if self.state.frame.actions.events.len() > comet_core::MAX_INPUT_EVENTS / 2 {
                self.synchronize = true;
                overflowed = true;
                self.state.frame = GamepadFrame::default();
            }
        }
        if self.active.is_none() {
            self.active = gilrs.gamepads().next().map(|(id, _)| id);
            self.synchronize = true;
        }
        self.connected = gilrs.gamepads().count();
        self.name = self
            .active
            .map(|id| gilrs.gamepad(id).name().chars().take(128).collect());
        gilrs.inc();
        if !drained || overflowed {
            self.overflows += 1;
            self.synchronize = true;
        }
        if !active || changed || smoke || self.synchronize {
            let (held, sticks) = self
                .active
                .map(|id| {
                    let pad = gilrs.gamepad(id);
                    let held = BUTTONS
                        .into_iter()
                        .enumerate()
                        .fold(0, |bits, (index, button)| {
                            bits | (u8::from(pad.is_pressed(button)) << index)
                        });
                    let sticks = AXES.map(|axes| axes.map(|axis| pad.value(axis)));
                    (held, sticks)
                })
                .unwrap_or_default();
            self.state.synchronize(held, sticks);
            self.synchronize = !drained;
            return GamepadFrame::default();
        }
        if self.active.is_none() {
            return GamepadFrame::default();
        }
        self.state.take()
    }
    pub fn close(&mut self) {
        self.gilrs.take();
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"status":if self.initialized {"ready"} else if self.error.is_some(){"unavailable"} else {"disabled"},"error":self.error,"connected_devices":self.connected,"active_device":self.name,"overflow_frames":self.overflows,"closed":self.gilrs.is_none()})
    }
}
