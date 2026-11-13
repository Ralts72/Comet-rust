use super::*;
use crate::input::merge_device_actions as merge;

#[test]
fn reconnect_and_axis_rearming_preserve_taps_but_never_inherit_held_controls() {
    let mut state = State::default();
    state.synchronize((1 << 0) | (1 << 6), [[0.8, 0.2], [0.3, 0.4]]);
    let frame = state.take();
    assert_eq!(frame.actions.held, Actions::default());
    assert!(frame.actions.events.is_empty());
    assert_eq!(frame.actions.movement, [0.; 2]);
    assert_eq!(frame.look_rate, [0.; 2]);
    state.button(0, false);
    state.button(6, false);
    assert!(state.take().actions.events.is_empty());
    state.button(0, true);
    state.button(0, true);
    state.button(0, false);
    let frame = state.take();
    assert_eq!(
        frame.actions.events,
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
    state.axis(0, 0, 0.);
    state.axis(0, 1, 0.);
    state.axis(0, 0, 1.);
    state.axis(0, 1, 1.);
    let frame = state.take();
    let movement = frame.actions.movement;
    assert!((movement[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    assert!((movement[0].hypot(movement[1]) - 1.).abs() < 1e-6);
    state.axis(1, 0, 0.);
    state.axis(1, 1, 0.);
    state.axis(1, 0, 1.);
    let frame = state.take();
    assert_eq!(frame.look_delta(1. / 60.), [10., 0.]);
    assert_eq!(frame.look_delta(f64::NAN), [0.; 2]);
    assert_eq!(frame.look_delta(1.), [60., 0.]);
    state.synchronize(0, [[0.; 2]; 2]);
    assert!(state.take().actions.events.is_empty());
    assert_eq!(deadzone([0.1, 0.1]), [0.; 2]);
    assert_eq!(deadzone([f32::NAN, 0.]), [0.; 2]);
}

#[test]
fn overlapping_device_holds_emit_only_edges_of_the_combined_action() {
    let event = |pressed| ActionEvent {
        action: Action::Primary,
        pressed,
    };
    let mut keyboard = ActionInput {
        enabled: true,
        ..Default::default()
    };
    keyboard.held.set(Action::Primary, true);
    let mut pad = GamepadFrame::default();
    pad.actions.enabled = true;
    pad.actions.held.set(Action::Primary, true);
    pad.actions.events = vec![event(true)];
    merge(&mut keyboard, &pad.actions);
    assert!(keyboard.held.contains(Action::Primary));
    assert!(keyboard.events.is_empty());
    keyboard.events = vec![event(false)];
    keyboard.held.set(Action::Primary, false);
    pad.actions.events.clear();
    merge(&mut keyboard, &pad.actions);
    assert!(keyboard.held.contains(Action::Primary));
    assert!(keyboard.events.is_empty());
    keyboard.held = Actions::default();
    pad.actions.held = Actions::default();
    pad.actions.events = vec![event(false)];
    merge(&mut keyboard, &pad.actions);
    assert_eq!(keyboard.events, vec![event(false)]);
    keyboard.events.clear();
    pad.actions.events = vec![event(true), event(false)];
    merge(&mut keyboard, &pad.actions);
    assert_eq!(keyboard.events, pad.actions.events);
    keyboard.movement = [1., 0.];
    pad.actions.movement = [-0.5, 1.];
    merge(&mut keyboard, &pad.actions);
    assert_eq!(keyboard.movement, [0.5, 1.]);
}

#[test]
fn pause_shortcuts_observe_final_authorization_and_do_not_replay_or_repeat() {
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let mut latch = GamepadPauseInput::default();
    let mut frame = GamepadFrame {
        pause_down: true,
        pause_pressed: true,
        ..Default::default()
    };
    let sample = |latch: &mut GamepadPauseInput, frame: &GamepadFrame, authorized| {
        let mut result = false;
        let mut output = ctx.run_ui(Default::default(), |_| {
            result = latch.sample(&ctx, frame, authorized);
            assert!(!latch.sample(&ctx, frame, authorized));
        });
        output.textures_delta.clear();
        result
    };
    assert!(!sample(&mut latch, &frame, false));
    assert!(!sample(&mut latch, &frame, true));
    frame.pause_down = false;
    frame.pause_pressed = false;
    assert!(!sample(&mut latch, &frame, true));
    frame.pause_down = true;
    frame.pause_pressed = true;
    assert!(sample(&mut latch, &frame, true));
    frame.pause_pressed = false;
    assert!(!sample(&mut latch, &frame, true));
}
