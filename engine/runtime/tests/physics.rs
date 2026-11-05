use anyhow::Result;
use comet_core::{Action, ActionEvent, ActionInput};
use comet_runtime::{FIXED_DELTA, PlaySession};
use comet_scene::{
    BodyMotion, Character, ColliderShape, EntityData, PhysicsBody, PoseUpdate, SceneSnapshot,
    schema,
};

fn body(name: &str, motion: BodyMotion, position: [f32; 3], half: [f32; 3]) -> EntityData {
    let mut e = EntityData::new(name);
    e.transform.translation = position;
    e.physics = Some(PhysicsBody {
        motion,
        shape: ColliderShape::Box { half_extents: half },
        ..Default::default()
    });
    e
}
fn ground() -> EntityData {
    body("ground", BodyMotion::Fixed, [0., -0.5, 0.], [20., 0.5, 20.])
}
fn character() -> EntityData {
    let mut e = body("character", BodyMotion::Kinematic, [0., 1., 0.], [0.5; 3]);
    e.physics.as_mut().unwrap().shape = ColliderShape::Capsule {
        half_height: 0.5,
        radius: 0.3,
    };
    e.character = Some(Character::default());
    e
}
fn scene(entities: Vec<EntityData>) -> SceneSnapshot {
    SceneSnapshot {
        entities,
        ..Default::default()
    }
}
fn input() -> ActionInput {
    ActionInput {
        enabled: true,
        ..Default::default()
    }
}
fn ticks(play: &mut PlaySession, count: usize, input: ActionInput) -> Result<()> {
    for _ in 0..count {
        play.advance_with_input(FIXED_DELTA, input.clone())?;
    }
    Ok(())
}
fn position(play: &mut PlaySession, entity: comet_core::EntityId) -> [f32; 3] {
    play.snapshot()
        .entities
        .iter()
        .find(|e| e.id == entity)
        .unwrap()
        .transform
        .translation
}
#[test]
fn dynamics_land_pause_step_and_restart_without_touching_authoring() -> Result<()> {
    let cube = body("falling", BodyMotion::Dynamic, [0., 4., 0.], [0.5; 3]);
    let id = cube.id;
    let author = scene(vec![ground(), cube]);
    let before = author.clone();
    let mut play = PlaySession::start(author.clone())?;
    let world = play.world_id();
    ticks(&mut play, 120, input())?;
    assert!((position(&mut play, id)[1] - 0.5).abs() < 0.025);
    play.apply_impulse(id, [0., 3., 0.])?;
    play.set_paused(true);
    let paused = play.snapshot();
    play.advance(10.)?;
    assert_eq!(play.snapshot(), paused);
    play.single_step()?;
    assert!(position(&mut play, id)[1] > 0.5);
    play.set_suspended(true);
    assert!(play.single_step().is_err());
    assert_eq!(play.world_id(), world);
    assert_eq!(author, before);
    let restarted = PlaySession::start(author)?;
    assert_ne!(restarted.world_id(), world);
    Ok(())
}
#[test]
fn layer_masks_keep_all_32_bits_and_filter_contacts() -> Result<()> {
    for collide in [false, true] {
        let mut floor = ground();
        let config = floor.physics.as_mut().unwrap();
        config.layer = 1 << 31;
        config.mask = 2;
        let mut cube = body("falling", BodyMotion::Dynamic, [0., 2., 0.], [0.5; 3]);
        let id = cube.id;
        let config = cube.physics.as_mut().unwrap();
        config.layer = 2;
        config.mask = if collide { 1 << 31 } else { 1 };
        let mut play = PlaySession::start(scene(vec![floor, cube]))?;
        ticks(&mut play, 90, input())?;
        let y = position(&mut play, id)[1];
        assert!(
            if collide {
                (y - 0.5).abs() < 0.025
            } else {
                y < -3.
            },
            "collide={collide} y={y}"
        );
    }
    Ok(())
}
#[test]
fn triggers_report_enter_exit_once_across_fixed_ticks() -> Result<()> {
    let mut sensor = body("trigger", BodyMotion::Fixed, [0.; 3], [1.; 3]);
    sensor.physics.as_mut().unwrap().sensor = true;
    let cube = body("traveler", BodyMotion::Kinematic, [-3., 0., 0.], [0.2; 3]);
    let (trigger, other) = (sensor.id, cube.id);
    let mut play = PlaySession::start(scene(vec![sensor, cube]))?;
    play.set_linear_velocity(other, [3., 0., 0.])?;
    let mut events = vec![];
    for _ in 0..120 {
        play.advance(FIXED_DELTA * 2.)?;
        events.extend_from_slice(play.trigger_events());
    }
    assert_eq!(events.len(), 2);
    assert_eq!(
        (events[0].trigger, events[0].other, events[0].entered),
        (trigger, other, true)
    );
    assert!(!events[1].entered && events[1].tick > events[0].tick);
    play.advance(0.)?;
    assert!(play.trigger_events().is_empty());
    Ok(())
}

#[test]
fn a_sensor_crossed_within_one_tick_reports_ordered_transitions_once() -> Result<()> {
    let mut sensor = body("thin sensor", BodyMotion::Fixed, [0.; 3], [0.04, 1., 1.]);
    sensor.physics.as_mut().unwrap().sensor = true;
    let traveler = body("traveler", BodyMotion::Kinematic, [-0.5, 0., 0.], [0.01; 3]);
    let (trigger, other) = (sensor.id, traveler.id);
    let mut play = PlaySession::start(scene(vec![sensor, traveler]))?;
    play.set_linear_velocity(other, [120., 0., 0.])?;
    play.advance(FIXED_DELTA)?;
    assert_eq!(play.steps(), 1);
    assert!((position(&mut play, other)[0] - 1.5).abs() < 1e-5);
    let events = play.trigger_events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(events[0].entered && !events[1].entered);
    for event in events {
        assert_eq!(
            (event.trigger, event.other, event.tick),
            (trigger, other, 1)
        );
    }
    play.advance(FIXED_DELTA)?;
    assert!(play.trigger_events().is_empty());
    play.set_paused(true);
    play.single_step()?;
    assert_eq!(play.steps(), 3);
    assert!(play.trigger_events().is_empty());
    Ok(())
}
#[test]
fn character_walks_runs_jumps_once_and_lands() -> Result<()> {
    let character = character();
    let id = character.id;
    let mut play = PlaySession::start(scene(vec![ground(), character]))?;
    ticks(&mut play, 60, input())?;
    assert!(play.character(id).unwrap().grounded);
    let start = position(&mut play, id);
    ticks(
        &mut play,
        30,
        ActionInput {
            movement: [1., 0.],
            ..input()
        },
    )?;
    let walked = position(&mut play, id);
    assert!((walked[0] - start[0] - 2.).abs() < 0.05);
    let mut run = ActionInput {
        movement: [1., 0.],
        ..input()
    };
    run.held.set(Action::Run, true);
    ticks(&mut play, 30, run)?;
    let ran = position(&mut play, id);
    assert!(
        (ran[0] - walked[0] - 3.5).abs() < 0.05,
        "walked={walked:?} ran={ran:?}"
    );
    let mut jump = input();
    jump.held.set(Action::Jump, true);
    jump.events.push(ActionEvent {
        action: Action::Jump,
        pressed: true,
    });
    play.advance_with_input(FIXED_DELTA, jump.clone())?;
    assert!(play.character(id).unwrap().vertical_velocity > 0.);
    jump.events.clear();
    ticks(&mut play, 10, jump.clone())?;
    assert!(position(&mut play, id)[1] > start[1] + 0.5);
    ticks(&mut play, 150, jump)?;
    assert!(play.character(id).unwrap().grounded);
    assert!((position(&mut play, id)[1] - start[1]).abs() < 0.025);
    Ok(())
}

#[test]
fn crouching_keeps_feet_in_place_uses_its_speed_and_resets_with_the_world() -> Result<()> {
    let character = character();
    let id = character.id;
    let authoring = scene(vec![ground(), character]);
    let saved = authoring.clone();
    let mut play = PlaySession::start(authoring.clone())?;
    ticks(&mut play, 60, input())?;
    let standing = position(&mut play, id);
    let mut crouch = input();
    crouch.held.set(Action::Crouch, true);
    crouch.held.set(Action::Run, true);
    ticks(&mut play, 1, crouch.clone())?;
    assert!(play.character(id).unwrap().crouched);
    let lower = position(&mut play, id);
    // Standing half-height is 0.8, crouched half-height is 0.48.
    assert!((standing[1] - lower[1] - 0.32).abs() < 0.02);
    assert!(play.character(id).unwrap().grounded);
    play.set_paused(true);
    ticks(&mut play, 2, input())?;
    assert!(play.character(id).unwrap().crouched);
    assert_eq!(position(&mut play, id), lower);
    play.set_paused(false);
    crouch.movement = [1., 0.];
    ticks(&mut play, 30, crouch)?;
    let walked = position(&mut play, id);
    assert!((walked[0] - 1.).abs() < 0.04, "walked={walked:?}");
    ticks(&mut play, 1, input())?;
    assert!(!play.character(id).unwrap().crouched);
    assert!((position(&mut play, id)[1] - standing[1]).abs() < 0.02);
    assert_eq!(authoring, saved);
    let restarted = PlaySession::start(authoring)?;
    assert!(!restarted.character(id).unwrap().crouched);
    assert_eq!(restarted.character(id).unwrap().position, [0., 1., 0.]);
    Ok(())
}

#[test]
fn restart_reconstructs_physics_character_posture_and_trigger_baselines() -> Result<()> {
    let character = character();
    let id = character.id;
    let falling = body("falling", BodyMotion::Dynamic, [3., 4., 0.], [0.5; 3]);
    let cube = falling.id;
    let mut trigger = body("trigger", BodyMotion::Fixed, [0., 1., 0.], [2.; 3]);
    trigger.physics.as_mut().unwrap().sensor = true;
    let authoring = scene(vec![ground(), character, falling, trigger]);
    let mut play = PlaySession::start(authoring.clone())?;
    ticks(&mut play, 60, input())?;
    let mut crouch = input();
    crouch.held.set(Action::Crouch, true);
    ticks(&mut play, 1, crouch)?;
    assert!(play.character(id).unwrap().crouched);
    assert!(position(&mut play, cube)[1] < 1.);
    let old = play.world_id();
    play.restart()?;
    assert_ne!(play.world_id(), old);
    assert_eq!(play.character(id).unwrap().position, [0., 1., 0.]);
    assert!(!play.character(id).unwrap().grounded);
    assert!(!play.character(id).unwrap().crouched);
    assert_eq!(play.character(id).unwrap().vertical_velocity, 0.);
    assert_eq!(position(&mut play, cube), [3., 4., 0.]);
    assert!(play.trigger_events().is_empty());
    assert_eq!(play.snapshot().to_json()?, authoring.to_json()?);
    play.advance_with_input(100., input())?;
    assert_eq!(play.steps(), 0);
    play.advance_with_input(FIXED_DELTA, input())?;
    assert!(
        play.trigger_events()
            .iter()
            .any(|e| e.other == id && e.entered && e.tick == 1)
    );
    Ok(())
}

#[test]
fn low_ceiling_blocks_standing_until_the_character_leaves_the_tunnel() -> Result<()> {
    let character = character();
    let id = character.id;
    let ceiling = body(
        "tunnel",
        BodyMotion::Fixed,
        [2.5, 1.35, 0.],
        [1.5, 0.25, 2.],
    );
    let mut play = PlaySession::start(scene(vec![ground(), character, ceiling]))?;
    ticks(&mut play, 60, input())?;
    let mut crouch = ActionInput {
        movement: [1., 0.],
        ..input()
    };
    crouch.held.set(Action::Crouch, true);
    ticks(&mut play, 60, crouch)?;
    let walked = position(&mut play, id);
    assert!((walked[0] - 2.).abs() < 0.04, "walked={walked:?}");
    ticks(&mut play, 1, input())?;
    assert!(play.character(id).unwrap().crouched);
    assert!(position(&mut play, id)[1] < 0.52);
    let mut denied = input();
    denied.enabled = false;
    ticks(&mut play, 5, denied)?;
    assert!(play.character(id).unwrap().crouched);
    assert!(position(&mut play, id)[1] < 0.52);
    let crouched_pose = position(&mut play, id);
    play.teleport(PoseUpdate {
        entity: id,
        translation: [crouched_pose[0], crouched_pose[1], 0.],
        rotation: [0., 0., 0., 1.],
    })?;
    assert!(play.character(id).unwrap().crouched);
    ticks(&mut play, 1, input())?;
    assert!(play.character(id).unwrap().crouched);
    ticks(
        &mut play,
        90,
        ActionInput {
            movement: [1., 0.],
            ..input()
        },
    )?;
    assert!(position(&mut play, id)[0] > 4.5);
    assert!(!play.character(id).unwrap().crouched);
    assert!((position(&mut play, id)[1] - 0.81).abs() < 0.025);
    Ok(())
}

#[test]
fn crouch_motion_is_stable_at_different_world_positions_and_frame_rates() -> Result<()> {
    for start in [-5., 0., 3., 9.] {
        let mut character = character();
        character.transform.translation[0] = start;
        let id = character.id;
        let source = scene(vec![ground(), character]);
        let mut expected = None;
        for fps in [30, 60, 120] {
            let mut play = PlaySession::start(source.clone())?;
            ticks(&mut play, 60, input())?;
            let mut crouch = ActionInput {
                movement: [1., 0.],
                ..input()
            };
            crouch.held.set(Action::Crouch, true);
            for _ in 0..fps {
                play.advance_with_input(1. / f64::from(fps), crouch.clone())?;
            }
            let actual = position(&mut play, id);
            assert!(
                (actual[0] - start - 2.).abs() < 0.05,
                "start={start} fps={fps} actual={actual:?}"
            );
            if let Some(expected) = expected {
                assert_eq!(actual, expected);
            } else {
                expected = Some(actual);
            }
        }
    }
    Ok(())
}

#[test]
fn standing_clearance_respects_sensors_and_collision_layers() -> Result<()> {
    for sensor in [true, false] {
        let character = character();
        let id = character.id;
        let mut ceiling = body(
            "ignored ceiling",
            BodyMotion::Fixed,
            [0., 1.35, 0.],
            [2., 0.25, 2.],
        );
        let config = ceiling.physics.as_mut().unwrap();
        config.sensor = sensor;
        if !sensor {
            config.mask = 0;
        }
        let mut play = PlaySession::start(scene(vec![ground(), character, ceiling]))?;
        ticks(&mut play, 60, input())?;
        let mut crouch = input();
        crouch.held.set(Action::Crouch, true);
        ticks(&mut play, 1, crouch)?;
        assert!(play.character(id).unwrap().crouched);
        ticks(&mut play, 1, input())?;
        assert!(!play.character(id).unwrap().crouched);
    }
    Ok(())
}

#[test]
fn short_crouch_capsules_keep_their_radius_and_ignore_unauthorized_input() -> Result<()> {
    let mut character = character();
    character.transform.scale = [2.; 3];
    character.transform.translation[1] = 2.;
    character.character.as_mut().unwrap().crouch_height_ratio = 0.1;
    let id = character.id;
    let mut play = PlaySession::start(scene(vec![ground(), character]))?;
    ticks(&mut play, 60, input())?;
    let mut crouch = input();
    crouch.held.set(Action::Crouch, true);
    ticks(&mut play, 1, crouch.clone())?;
    assert!(play.character(id).unwrap().crouched);
    assert!((position(&mut play, id)[1] - 0.61).abs() < 0.025);
    crouch.enabled = false;
    crouch.movement = [1., 0.];
    ticks(&mut play, 30, crouch)?;
    assert!(!play.character(id).unwrap().crouched);
    assert!((position(&mut play, id)[1] - 1.61).abs() < 0.025);
    assert!(position(&mut play, id)[0].abs() < 1e-5);
    Ok(())
}
#[test]
fn character_stops_at_walls_and_climbs_small_steps() -> Result<()> {
    for height in [0.2, 2.] {
        let character = character();
        let id = character.id;
        let obstacle = body(
            "obstacle",
            BodyMotion::Fixed,
            [1.5, height / 2., 0.],
            [0.5, height / 2., 2.],
        );
        let mut play = PlaySession::start(scene(vec![ground(), character, obstacle]))?;
        ticks(&mut play, 60, input())?;
        ticks(
            &mut play,
            45,
            ActionInput {
                movement: [1., 0.],
                ..input()
            },
        )?;
        let x = position(&mut play, id)[0];
        assert!(
            if height < 1. { x > 2.5 } else { x < 0.75 },
            "height={height} x={x}"
        );
    }
    Ok(())
}
#[test]
fn character_rides_a_moving_kinematic_platform() -> Result<()> {
    let character = character();
    let id = character.id;
    let platform = body(
        "platform",
        BodyMotion::Kinematic,
        [0., -0.5, 0.],
        [3., 0.5, 3.],
    );
    let platform_id = platform.id;
    let mut play = PlaySession::start(scene(vec![platform, character]))?;
    ticks(&mut play, 60, input())?;
    play.set_linear_velocity(platform_id, [1., 0., 0.])?;
    ticks(&mut play, 60, input())?;
    assert!(play.character(id).unwrap().grounded);
    assert!(position(&mut play, id)[0] > 0.8);
    Ok(())
}
#[test]
fn explicit_teleport_keeps_backend_and_scene_in_sync_and_rejects_invalid_commands() -> Result<()> {
    let character = character();
    let id = character.id;
    let mut play = PlaySession::start(scene(vec![ground(), character]))?;
    assert!(
        play.set_field(
            &[id],
            schema::TRANSLATION,
            comet_scene::schema::FieldValue::Vec3([3.; 3])
        )
        .is_err()
    );
    assert!(play.set_linear_velocity(id, [1.; 3]).is_err());
    assert!(play.apply_impulse(id, [1.; 3]).is_err());
    let before = play.snapshot();
    assert!(
        play.teleport(PoseUpdate {
            entity: id,
            translation: [f32::NAN, 0., 0.],
            rotation: [0., 0., 0., 1.]
        })
        .is_err()
    );
    assert_eq!(play.snapshot(), before);
    play.teleport(PoseUpdate {
        entity: id,
        translation: [3., 5., 0.],
        rotation: [0., 0., 0., 1.],
    })?;
    assert_eq!(play.character(id).unwrap().position, [3., 5., 0.]);
    ticks(&mut play, 120, input())?;
    let p = position(&mut play, id);
    assert!((p[0] - 3.).abs() < 0.01 && (p[1] - 0.81).abs() < 0.04);
    Ok(())
}

#[test]
fn slope_limit_accepts_a_ramp_and_blocks_a_steep_face() -> Result<()> {
    for degrees in [30_f32, 65.] {
        let angle = degrees.to_radians();
        let mut character = character();
        let id = character.id;
        character.transform.translation = [-2., 1., 0.];
        let mut ramp = body(
            "ramp",
            BodyMotion::Fixed,
            [2. * angle.cos(), 2. * angle.sin(), 0.],
            [2., 0.1, 2.],
        );
        ramp.transform.rotation = [0., 0., (angle / 2.).sin(), (angle / 2.).cos()];
        let mut play = PlaySession::start(scene(vec![ground(), character, ramp]))?;
        ticks(&mut play, 60, input())?;
        ticks(
            &mut play,
            60,
            ActionInput {
                movement: [1., 0.],
                ..input()
            },
        )?;
        let p = position(&mut play, id);
        assert!(
            if degrees < 45. {
                p[0] > 1.2 && p[1] > 1.5
            } else {
                p[0] < 0.2
            },
            "slope={degrees} position={p:?}"
        );
    }
    Ok(())
}

#[test]
fn physics_results_match_across_render_rates_and_source_entity_order() -> Result<()> {
    let mut c = character();
    c.transform.translation = [-5., 1., 0.];
    let cube = body("falling", BodyMotion::Dynamic, [0., 3., 0.], [0.5; 3]);
    let id = cube.id;
    let source = scene(vec![ground(), cube, c]);
    let mut expected = None;
    for fps in [30, 60, 120] {
        let mut copy = source.clone();
        if fps == 120 {
            copy.entities.reverse();
        }
        let mut play = PlaySession::start(copy)?;
        for _ in 0..fps {
            play.advance_with_input(
                1. / f64::from(fps),
                ActionInput {
                    movement: [1., 0.],
                    ..input()
                },
            )?;
        }
        let actual = (
            position(&mut play, id),
            play.player_character().unwrap().position,
        );
        if let Some(expected) = expected {
            assert_eq!(actual, expected);
        } else {
            expected = Some(actual);
        }
    }
    Ok(())
}
