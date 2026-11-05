use anyhow::Result;
use comet_core::{Action, ActionEvent, ActionInput, AssetId, EntityId};
use comet_runtime::{FIXED_DELTA, PlaySession, ScriptLibrary, validate_scripts};
use comet_scene::{EntityData, SceneSnapshot, Script, ScriptValue};

fn scripted(source: &str) -> (SceneSnapshot, ScriptLibrary, EntityId) {
    let asset = AssetId::new();
    let mut entity = EntityData::new("scripted");
    entity.script = Some(Script::new(asset));
    let id = entity.id;
    (
        SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        [(asset, source.to_owned())].into(),
        id,
    )
}
fn x(play: &mut PlaySession, id: EntityId) -> f32 {
    play.snapshot()
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .transform
        .translation[0]
}
fn input() -> ActionInput {
    ActionInput {
        enabled: true,
        ..Default::default()
    }
}

#[test]
fn lifecycle_has_private_per_binding_state_and_restarts_from_author_baseline() -> Result<()> {
    let (mut snapshot, sources, id) = scripted(
        r#"
        fn on_start(ctx) { this.n = 0; ctx.log("start"); }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            ctx.translate_local(ctx.entity(), ctx.param("speed") * dt, 0, 0);
            ctx.log(this.n.to_string());
        }
        fn on_stop(ctx) { ctx.log("stop"); }
    "#,
    );
    snapshot.entities[0]
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert("speed".into(), ScriptValue::Number(60.));
    let mut second = snapshot.entities[0].clone();
    second.id = EntityId::new();
    second
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert("speed".into(), ScriptValue::Number(120.));
    let other = second.id;
    snapshot.entities.push(second);
    let author = snapshot.clone();
    let mut play = PlaySession::start_with_scripts(snapshot.clone(), sources)?;
    assert_eq!(play.script_diagnostics().len(), 2);
    play.advance(FIXED_DELTA * 2.)?;
    assert_eq!(x(&mut play, id), 2.);
    assert_eq!(x(&mut play, other), 4.);
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|d| d.message == "2")
            .count(),
        2
    );
    let previous = play.world_id();
    play.set_paused(true);
    play.restart()?;
    assert_ne!(previous, play.world_id());
    assert!(play.is_paused());
    assert_eq!(x(&mut play, id), 0.);
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|d| d.message == "stop" && d.world == previous)
            .count(),
        2
    );
    assert!(
        play.script_diagnostics()
            .iter()
            .zip(play.script_diagnostics().iter().skip(1))
            .all(|(a, b)| a.sequence < b.sequence)
    );
    play.single_step()?;
    assert_eq!(x(&mut play, id), 1.);
    play.stop();
    let count = play.script_diagnostics().len();
    play.stop();
    assert_eq!(play.script_diagnostics().len(), count);
    assert!(play.advance(FIXED_DELTA).is_err());
    assert_eq!(snapshot, author);
    Ok(())
}

#[test]
fn script_simulation_is_independent_of_frame_rate_and_respects_pause_and_suspend() -> Result<()> {
    let (snapshot, sources, id) =
        scripted("fn on_fixed_update(ctx, dt) { ctx.translate_local(ctx.entity(), dt, 0, 0); }");
    for fps in [30, 60, 144, 240] {
        let mut play = PlaySession::start_with_scripts(snapshot.clone(), sources.clone())?;
        for _ in 0..fps {
            play.advance(1. / f64::from(fps))?;
        }
        assert!((x(&mut play, id) - 1.).abs() < 1e-5);
        play.set_paused(true);
        play.advance(1.)?;
        play.set_suspended(true);
        assert!(play.single_step().is_err());
        play.set_suspended(false);
        play.single_step()?;
        assert!((x(&mut play, id) - (1. + FIXED_DELTA as f32)).abs() < 1e-5);
        assert!(play.script_diagnostics().is_empty());
    }
    Ok(())
}

#[test]
fn action_edges_are_delivered_once_across_catchup_ticks() -> Result<()> {
    let (snapshot, sources, id) = scripted(
        r#"fn on_fixed_update(ctx, dt) {
        if ctx.pressed("interact") { ctx.translate_local(ctx.entity(), 1, 0, 0); }
    }"#,
    );
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    play.advance_with_input(0., input())?;
    let mut tap = input();
    tap.events = vec![
        ActionEvent {
            action: Action::Interact,
            pressed: true,
        },
        ActionEvent {
            action: Action::Interact,
            pressed: false,
        },
    ];
    play.advance_with_input(FIXED_DELTA * 3., tap)?;
    assert_eq!(x(&mut play, id), 1.);
    Ok(())
}

#[test]
fn failed_callbacks_discard_all_world_commands_and_are_disabled_once() -> Result<()> {
    for source in [
        "fn on_fixed_update(ctx, dt) { ctx.translate_local(ctx.entity(), 3, 0, 0); throw \"failed\"; }",
        "fn on_fixed_update(ctx, dt) { ctx.translate_local(ctx.entity(), 3, 0, 0); ctx.set_velocity(ctx.entity(), 1, 0, 0); }",
        "fn on_fixed_update(ctx, dt) { ctx.translate_local(ctx.entity(), 3, 0, 0); this.saved = ctx; }",
        "fn on_fixed_update(ctx, dt) { ctx.translate_local(ctx.entity(), 3, 0, 0); loop {} }",
    ] {
        let (snapshot, sources, id) = scripted(source);
        let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
        play.advance(FIXED_DELTA * 3.)?;
        assert_eq!(x(&mut play, id), 0., "{source}");
        assert_eq!(
            play.script_diagnostics()
                .iter()
                .filter(|d| d.failed)
                .count(),
            1,
            "{source}"
        );
        play.stop();
        assert_eq!(
            play.script_diagnostics()
                .iter()
                .filter(|d| d.failed)
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn invalid_programs_are_rejected_before_start_and_validation_does_not_execute_them() -> Result<()> {
    for source in [
        "fn on_start( {",
        "fn on_start() {}",
        "fn helper() {}",
        "import \"outside.rhai\" as outside; fn on_start(ctx) {}",
        "fn on_start(ctx) { eval(\"1\"); }",
    ] {
        let (snapshot, sources, _) = scripted(source);
        assert!(validate_scripts(&snapshot, &sources).is_err(), "{source}");
    }
    let (snapshot, sources, _) =
        scripted("throw \"top level\"; fn on_start(ctx) { ctx.log(\"unreachable\"); }");
    validate_scripts(&snapshot, &sources)?;
    assert!(PlaySession::start(snapshot.clone()).is_err());
    let play = PlaySession::start_with_scripts(snapshot, sources)?;
    assert_eq!(play.script_diagnostics().len(), 1);
    assert!(play.script_diagnostics()[0].failed);
    Ok(())
}

#[test]
fn one_failing_binding_does_not_stop_other_entities_and_stop_is_read_only() -> Result<()> {
    let (mut snapshot, mut sources, id) = scripted(
        "fn on_fixed_update(ctx, dt) { ctx.translate_local(ctx.entity(), 1, 0, 0); } fn on_stop(ctx) { ctx.translate_local(ctx.entity(), 100, 0, 0); }",
    );
    let (bad, other_sources, _) = scripted("fn on_fixed_update(ctx, dt) { loop {} }");
    snapshot.entities.extend(bad.entities);
    sources.extend(other_sources);
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    play.advance(FIXED_DELTA * 2.)?;
    assert_eq!(x(&mut play, id), 2.);
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|d| d.failed)
            .count(),
        1
    );
    play.stop();
    assert_eq!(x(&mut play, id), 2.);
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|d| d.failed && d.phase == "on_stop")
    );
    Ok(())
}

#[test]
fn trigger_callbacks_belong_to_the_sensor_and_entity_parameters_target_the_current_world()
-> Result<()> {
    use comet_scene::{BodyMotion, EntityRef, PhysicsBody, PoseUpdate};
    let (mut snapshot, sources, sensor_id) = scripted(
        r#"
        fn on_trigger(ctx, other, entered) {
            ctx.log(other.id());
            if entered { ctx.translate_local(ctx.param("target"), 2, 0, 0); }
        }
    "#,
    );
    snapshot.entities[0].physics = Some(PhysicsBody {
        sensor: true,
        ..Default::default()
    });
    let marker = EntityData::new("marker");
    let marker_id = marker.id;
    snapshot.entities[0]
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert(
            "target".into(),
            ScriptValue::Entity(Some(EntityRef::Local { entity: marker_id })),
        );
    let mut body = EntityData::new("body");
    body.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    let body_id = body.id;
    snapshot.entities.extend([marker, body]);
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    play.advance(FIXED_DELTA * 3.)?;
    assert_eq!(x(&mut play, marker_id), 2.);
    assert_eq!(play.script_diagnostics().len(), 1);
    assert_eq!(play.script_diagnostics()[0].message, body_id.to_string());
    assert_eq!(play.script_diagnostics()[0].entity, sensor_id);
    play.teleport(PoseUpdate {
        entity: body_id,
        translation: [10., 0., 0.],
        rotation: [0., 0., 0., 1.],
    })?;
    play.advance(FIXED_DELTA)?;
    assert_eq!(x(&mut play, marker_id), 2.);
    assert_eq!(play.script_diagnostics().len(), 2);
    Ok(())
}

#[test]
fn physical_commands_validate_the_entire_batch_before_writing_velocity_or_scene() -> Result<()> {
    use comet_scene::{BodyMotion, EntityRef, PhysicsBody};
    let (mut snapshot, sources, id) = scripted(
        r#"
        fn on_start(ctx) {
            ctx.translate_local(ctx.entity(), 9, 0, 0);
            ctx.impulse(ctx.param("body"), 5, 0, 0);
            ctx.set_velocity(ctx.entity(), 1, 0, 0);
        }
    "#,
    );
    let mut body = EntityData::new("body");
    body.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    let body_id = body.id;
    snapshot.entities[0]
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert(
            "body".into(),
            ScriptValue::Entity(Some(EntityRef::Local { entity: body_id })),
        );
    snapshot.entities.push(body);
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    assert_eq!(x(&mut play, id), 0.);
    assert!(play.script_diagnostics()[0].failed);
    play.advance(FIXED_DELTA)?;
    assert_eq!(
        x(&mut play, body_id),
        0.,
        "earlier impulse must not escape a failed batch"
    );
    Ok(())
}

#[test]
fn persistent_state_and_total_phase_budgets_bound_long_running_scripts() -> Result<()> {
    let source = r#"
        fn on_start(ctx) {
            this.data = #{};
            for n in 0..40 { this.data["KEY" + n.to_string()] = n; }
        }
    "#
    .replace("KEY", &"x".repeat(4096));
    let (snapshot, sources, _) = scripted(&source);
    let play = PlaySession::start_with_scripts(snapshot, sources)?;
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|d| d.failed && d.message.contains("byte budget")),
        "{:?}",
        play.script_diagnostics()
    );
    let (snapshot, sources, _) = scripted(
        r#"
        fn on_start(ctx) { this.data = []; }
        fn on_fixed_update(ctx, dt) { this.data.push(1); }
    "#,
    );
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    for _ in 0..1200 {
        play.advance(FIXED_DELTA)?;
    }
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|d| d.failed)
            .count(),
        1
    );
    let (mut snapshot, sources, _) = scripted(
        "fn on_start(ctx) { this.done = false; } fn on_fixed_update(ctx, dt) { if !this.done { for n in 0..1000 { let x = n + 1; } this.done = true; } ctx.translate_local(ctx.entity(), 1, 0, 0); }",
    );
    let template = snapshot.entities[0].clone();
    for _ in 1..64 {
        let mut entity = template.clone();
        entity.id = EntityId::new();
        snapshot.entities.push(entity);
    }
    let mut tail = EntityData::new("after the exhausted phase");
    tail.id = "ffffffff-ffff-4fff-bfff-ffffffffffff".parse()?;
    // Give all heavy bindings deterministic earlier identities.
    for (index, entity) in snapshot.entities.iter_mut().enumerate() {
        entity.id = format!("00000000-0000-4000-8000-{:012x}", index + 1).parse()?;
    }
    let asset = AssetId::new();
    tail.script = Some(Script::new(asset));
    let tail_id = tail.id;
    snapshot.entities.push(tail);
    let mut sources = sources;
    sources.insert(
        asset,
        "fn on_fixed_update(ctx, dt) { ctx.translate_local(ctx.entity(), 1, 0, 0); }".into(),
    );
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    play.advance(FIXED_DELTA)?;
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|d| d.failed && d.message.contains("phase operation budget")),
        "{:?}",
        play.script_diagnostics()
    );
    assert!(
        play.snapshot()
            .entities
            .iter()
            .any(|e| e.transform.translation[0] == 1.)
    );
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|d| d.failed)
            .count(),
        1
    );
    assert_eq!(x(&mut play, tail_id), 0.);
    play.advance(FIXED_DELTA)?;
    assert_eq!(
        x(&mut play, tail_id),
        1.,
        "a skipped binding remains live on the following tick"
    );
    Ok(())
}

#[test]
fn initialization_budget_failure_rejects_the_candidate_instead_of_starting_uninitialized_bindings()
{
    let (mut snapshot, sources, _) =
        scripted("fn on_start(ctx) { for n in 0..1000 { let x = n + 1; } }");
    let template = snapshot.entities[0].clone();
    for _ in 1..128 {
        let mut entity = template.clone();
        entity.id = EntityId::new();
        snapshot.entities.push(entity);
    }
    assert!(
        PlaySession::start_with_scripts(snapshot, sources)
            .err()
            .is_some_and(|error| error.to_string().contains("total phase operation budget"))
    );
}

#[test]
fn queued_writes_preserve_order_read_the_entry_snapshot_and_validate_intermediate_values()
-> Result<()> {
    let (snapshot, sources, id) = scripted(
        r#"
        fn on_start(ctx) {
            ctx.set_position(ctx.entity(), 2, 0, 0);
            ctx.translate_local(ctx.entity(), 3, 0, 0);
            if ctx.position(ctx.entity())[0] != 0.0 { throw "query saw uncommitted writes"; }
            ctx.log("validated");
        }
    "#,
    );
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    assert_eq!(x(&mut play, id), 5.);
    assert_eq!(play.script_diagnostics()[0].message, "validated");
    assert!(!play.script_diagnostics()[0].failed);
    let (snapshot, sources, id) = scripted(
        r#"
        fn on_start(ctx) {
            ctx.set_position(ctx.entity(), 3.4e38, 0, 0);
            ctx.translate_local(ctx.entity(), 3.4e38, 0, 0);
            ctx.set_position(ctx.entity(), 0, 0, 0);
        }
    "#,
    );
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    assert_eq!(x(&mut play, id), 0.);
    assert!(
        play.script_diagnostics()[0].failed,
        "invalid intermediate writes cannot be hidden by a later valid value"
    );
    Ok(())
}
