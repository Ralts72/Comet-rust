use anyhow::Result;
use comet_core::{Action, ActionEvent, ActionInput, Actions};
use comet_runtime::{FIXED_DELTA, MAX_FRAME_STEPS, PlaySession};
use comet_scene::{EntityData, SceneSnapshot, schema};

fn input() -> ActionInput {
    ActionInput {
        enabled: true,
        ..Default::default()
    }
}
fn session() -> Result<PlaySession> {
    PlaySession::start(SceneSnapshot::default())
}

fn restart_input(held: bool) -> ActionInput {
    let mut input = input();
    input.held.set(Action::Restart, held);
    input
}

#[test]
fn restart_restores_entry_snapshot_rebuilds_world_and_preserves_pause_and_suspension() -> Result<()>
{
    let entity = EntityData::new("entry");
    let id = entity.id;
    let entry = SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    };
    let mut play = PlaySession::start(entry.clone())?;
    let old_world = play.world_id();
    play.advance(1.)?;
    play.set_field(
        &[id],
        schema::TRANSLATION,
        schema::FieldValue::Vec3([4., 3., 2.]),
    )?;
    play.set_paused(true);
    play.request_single_step()?;
    play.set_suspended(true);
    play.restart()?;
    assert_ne!(play.world_id(), old_world);
    assert!(play.is_paused() && play.is_suspended());
    assert_eq!(play.snapshot(), entry);
    assert_eq!(play.steps(), 0);
    assert_eq!(play.elapsed(), 0.);
    assert_eq!(play.dropped_time(), 0.);
    assert_eq!(play.frame_steps(), 0);
    assert_eq!(play.interpolation_alpha(), 0.);
    assert_eq!(play.input(), &ActionInput::default());
    play.advance(100.)?;
    assert_eq!(play.steps(), 0);
    play.set_suspended(false);
    play.advance(100.)?;
    assert_eq!(play.steps(), 0);
    play.set_paused(false);
    play.advance(FIXED_DELTA / 2.)?;
    assert_eq!(play.steps(), 0);
    play.advance(FIXED_DELTA / 2.)?;
    assert_eq!(play.steps(), 1);
    Ok(())
}

#[test]
fn restart_actions_are_immediate_while_paused_and_require_a_fresh_authorized_edge() -> Result<()> {
    let mut play = session()?;
    play.advance_with_input(0., input())?;
    play.set_paused(true);
    play.advance_with_input(0., input())?;
    play.request_single_step()?;
    let previous = play.world_id();
    let mut tap = input();
    tap.events = vec![
        ActionEvent {
            action: Action::Restart,
            pressed: true,
        },
        ActionEvent {
            action: Action::Restart,
            pressed: false,
        },
    ];
    play.advance_with_input(0., tap.clone())?;
    assert_ne!(play.world_id(), previous);
    assert!(play.is_paused());
    assert_eq!(play.steps(), 0);
    let restarted = play.world_id();
    for _ in 0..3 {
        play.advance_with_input(1., restart_input(true))?;
        assert_eq!(play.world_id(), restarted);
    }
    play.advance_with_input(0., input())?;
    play.advance_with_input(0., tap.clone())?;
    assert_ne!(play.world_id(), restarted);
    let restarted = play.world_id();
    play.set_paused(false);
    let mut denied = restart_input(true);
    denied.enabled = false;
    play.advance_with_input(1., denied)?;
    play.advance_with_input(0., restart_input(true))?;
    assert_eq!(play.world_id(), restarted);
    play.synchronize_host(true, true);
    play.advance_with_input(0., tap.clone())?;
    assert_eq!(play.world_id(), restarted);
    play.advance_with_input(0., tap)?;
    assert_ne!(play.world_id(), restarted);
    assert_eq!(play.steps(), 0);
    Ok(())
}

#[test]
fn restart_discards_construction_time_old_inputs_and_invalid_frames() -> Result<()> {
    let mut play = session()?;
    play.advance_with_input(0., input())?;
    let previous = play.world_id();
    for dt in [-1., f64::INFINITY, f64::NAN] {
        assert!(play.advance_with_input(dt, restart_input(true)).is_err());
        assert_eq!(play.world_id(), previous);
    }
    play.advance_with_input(
        FIXED_DELTA / 2.,
        ActionInput {
            events: vec![ActionEvent {
                action: Action::Jump,
                pressed: true,
            }],
            look_delta: [3., 2.],
            ..input()
        },
    )?;
    play.advance_with_input(100., restart_input(true))?;
    assert_ne!(play.world_id(), previous);
    assert_eq!(play.steps(), 0);
    play.advance_with_input(0., restart_input(true))?;
    play.advance_with_input(100., restart_input(true))?;
    assert_eq!(play.steps(), 0);
    assert_eq!(play.dropped_time(), 0.);
    play.advance_with_input(FIXED_DELTA, restart_input(true))?;
    assert_eq!(play.steps(), 1);
    assert!(play.input().events.is_empty());
    assert_eq!(play.input().look_delta, [0.; 2]);
    Ok(())
}

#[test]
fn fixed_time_is_independent_of_render_rate() -> Result<()> {
    for fps in [30, 60, 120, 144, 240] {
        let mut play = session()?;
        let world = play.world_id();
        for _ in 0..fps {
            play.advance(1. / f64::from(fps))?;
        }
        assert_eq!(play.steps(), 60, "{fps} FPS");
        assert!((play.elapsed() - 1.).abs() < 1e-12);
        assert_eq!(play.world_id(), world);
        assert!(play.interpolation_alpha() < 1e-8);
    }
    Ok(())
}

#[test]
fn long_frames_have_bounded_catchup_and_account_for_dropped_time() -> Result<()> {
    let mut play = session()?;
    play.advance(1.)?;
    assert_eq!(play.frame_steps(), MAX_FRAME_STEPS);
    assert_eq!(play.steps(), u64::from(MAX_FRAME_STEPS));
    assert!((play.elapsed() + play.dropped_time() - 1.).abs() < 1e-12);
    play.advance(0.)?;
    assert_eq!(play.frame_steps(), 0);
    play.advance(FIXED_DELTA / 2.)?;
    assert_eq!(play.frame_steps(), 0);
    assert!((play.interpolation_alpha() - 0.5).abs() < 1e-8);
    play.advance(FIXED_DELTA / 2.)?;
    assert_eq!(play.frame_steps(), 1);
    Ok(())
}

#[test]
fn paused_single_step_and_host_suspension_do_not_accrue_debt() -> Result<()> {
    let mut play = session()?;
    assert!(play.single_step().is_err());
    play.advance(FIXED_DELTA / 2.)?;
    play.set_paused(true);
    play.request_single_step()?;
    play.advance(10.)?;
    assert_eq!(play.steps(), 1);
    play.advance(10.)?;
    assert_eq!(play.steps(), 1);
    assert_eq!(play.dropped_time(), 0.);
    play.set_paused(false);
    play.advance(FIXED_DELTA / 2.)?;
    assert_eq!(play.steps(), 1);
    play.set_suspended(true);
    play.advance(100.)?;
    assert_eq!(play.steps(), 1);
    play.set_paused(true);
    assert!(play.single_step().is_err());
    assert!(play.request_single_step().is_err());
    play.set_suspended(false);
    play.set_paused(false);
    play.advance(FIXED_DELTA / 2.)?;
    assert_eq!(play.steps(), 1);
    play.advance(FIXED_DELTA / 2.)?;
    assert_eq!(play.steps(), 2);
    assert_eq!(play.dropped_time(), 0.);
    Ok(())
}

#[test]
fn quick_taps_wait_for_a_tick_and_are_not_repeated_during_catchup() -> Result<()> {
    let mut play = session()?;
    play.advance_with_input(0., input())?;
    let tap = vec![
        ActionEvent {
            action: Action::Jump,
            pressed: true,
        },
        ActionEvent {
            action: Action::Jump,
            pressed: false,
        },
    ];
    play.advance_with_input(
        FIXED_DELTA / 2.,
        ActionInput {
            events: tap.clone(),
            look_delta: [3., -2.],
            ..input()
        },
    )?;
    assert_eq!(play.steps(), 0);
    play.advance_with_input(
        FIXED_DELTA / 2.,
        ActionInput {
            look_delta: [2., 1.],
            ..input()
        },
    )?;
    assert_eq!(play.input().events, tap);
    assert_eq!(play.input().look_delta, [5., -1.]);
    assert!(!play.input().held.contains(Action::Jump));
    play.advance_with_input(FIXED_DELTA * 3., input())?;
    assert_eq!(play.frame_steps(), 3);
    assert!(play.input().events.is_empty());
    assert_eq!(play.input().look_delta, [0.; 2]);
    Ok(())
}

#[test]
fn pause_input_authorization_and_new_worlds_rebuild_the_baseline() -> Result<()> {
    let mut play = session()?;
    play.advance_with_input(0., input())?;
    let mut held = Actions::default();
    held.set(Action::Jump, true);
    let pressed = ActionInput {
        held,
        events: vec![ActionEvent {
            action: Action::Jump,
            pressed: true,
        }],
        look_delta: [4., 2.],
        ..input()
    };
    play.advance_with_input(0., pressed.clone())?;
    play.advance_with_input(0., ActionInput::default())?;
    play.advance_with_input(FIXED_DELTA, pressed.clone())?;
    assert!(play.input().events.is_empty());
    assert_eq!(play.input().look_delta, [0.; 2]);
    assert!(play.input().held.contains(Action::Jump));
    play.set_paused(true);
    play.request_single_step()?;
    play.advance_with_input(5., pressed.clone())?;
    assert_eq!(play.frame_steps(), 1);
    assert!(play.input().events.is_empty());
    assert_eq!(play.input().look_delta, [0.; 2]);
    let mut fresh = session()?;
    fresh.advance_with_input(FIXED_DELTA, pressed)?;
    assert_ne!(play.world_id(), fresh.world_id());
    assert!(fresh.input().events.is_empty());
    assert_eq!(fresh.input().look_delta, [0.; 2]);
    Ok(())
}

#[test]
fn invalid_frames_preserve_time_and_pending_actions() -> Result<()> {
    let mut play = session()?;
    play.advance_with_input(0., input())?;
    let tap = vec![
        ActionEvent {
            action: Action::Interact,
            pressed: true,
        },
        ActionEvent {
            action: Action::Interact,
            pressed: false,
        },
    ];
    play.advance_with_input(
        FIXED_DELTA / 2.,
        ActionInput {
            events: tap.clone(),
            ..input()
        },
    )?;
    let alpha = play.interpolation_alpha();
    for dt in [-1., f64::NAN, f64::INFINITY] {
        assert!(play.advance_with_input(dt, input()).is_err());
        assert_eq!(play.steps(), 0);
        assert_eq!(play.interpolation_alpha(), alpha);
    }
    assert!(
        play.advance_with_input(
            1.,
            ActionInput {
                movement: [f32::NAN, 0.],
                ..input()
            }
        )
        .is_err()
    );
    assert_eq!(play.dropped_time(), 0.);
    play.advance_with_input(FIXED_DELTA / 2., input())?;
    assert_eq!(play.input().events, tap);
    Ok(())
}

#[test]
fn host_recovery_between_render_frames_discards_pending_input_and_partial_time() -> Result<()> {
    let mut play = session()?;
    play.advance_with_input(0., input())?;
    play.advance_with_input(
        FIXED_DELTA / 2.,
        ActionInput {
            events: vec![
                ActionEvent {
                    action: Action::Jump,
                    pressed: true,
                },
                ActionEvent {
                    action: Action::Jump,
                    pressed: false,
                },
            ],
            look_delta: [2., 3.],
            ..input()
        },
    )?;
    play.synchronize_host(true, true);
    play.advance_with_input(FIXED_DELTA / 2., input())?;
    assert_eq!(play.steps(), 0);
    play.advance_with_input(FIXED_DELTA / 2., input())?;
    assert_eq!(play.steps(), 1);
    assert!(play.input().events.is_empty());
    assert_eq!(play.input().look_delta, [0.; 2]);
    Ok(())
}

#[test]
fn running_world_edits_and_time_do_not_modify_the_authoring_snapshot() -> Result<()> {
    let entity = EntityData::new("editable");
    let id = entity.id;
    let source = SceneSnapshot {
        version: comet_scene::scene::SCENE_VERSION,
        entities: vec![entity],
    };
    let original = source.clone();
    let mut play = PlaySession::start(source.clone())?;
    play.set_field(
        &[id],
        schema::TRANSLATION,
        schema::FieldValue::Vec3([1., 2., 3.]),
    )?;
    play.advance(FIXED_DELTA * 2.)?;
    assert_eq!(source, original);
    assert_ne!(play.snapshot(), source);
    let fresh = PlaySession::start(source)?;
    assert_eq!(fresh.steps(), 0);
    assert_eq!(fresh.elapsed(), 0.);
    assert_ne!(play.world_id(), fresh.world_id());
    Ok(())
}
