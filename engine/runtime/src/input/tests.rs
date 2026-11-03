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
