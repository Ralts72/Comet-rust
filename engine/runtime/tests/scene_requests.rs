use anyhow::Result;
use comet_core::{Action, ActionEvent, ActionInput, AssetId, EntityId};
use comet_runtime::{FIXED_DELTA, PlaySession, ScriptLibrary};
use comet_scene::{
    BodyMotion, EntityData, PhysicsBody, SceneSnapshot, Script, ScriptValue,
    schema::{self, FieldValue},
};
use std::sync::Arc;

fn scripted(source: &str, target: AssetId) -> (SceneSnapshot, ScriptLibrary, EntityId, AssetId) {
    let asset = AssetId::new();
    let mut entity = EntityData::new("traveler");
    let mut script = Script::new(asset);
    script
        .parameters
        .insert("next_scene".into(), ScriptValue::Text(target.to_string()));
    entity.script = Some(script);
    let id = entity.id;
    (
        SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        [(asset, source.into())].into(),
        id,
        asset,
    )
}

fn x(play: &mut PlaySession, id: EntityId) -> f32 {
    play.shared_snapshot()
        .entities
        .iter()
        .find(|entity| entity.id == id)
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
fn committed_requests_are_idempotent_and_first_binding_retains_its_context() -> Result<()> {
    let target = AssetId::new();
    let other = AssetId::new();
    let (mut snapshot, mut sources, _, asset) = scripted(
        r#"
        fn on_start(ctx) {
            if !ctx.request_scene(ctx.param("next_scene")) { throw "first request"; }
            if !ctx.request_scene(ctx.param("next_scene")) { throw "duplicate request"; }
            if ctx.request_scene(ctx.param("other_scene")) { throw "conflicting request"; }
            ctx.translate_local(ctx.entity(), 2, 0, 0);
        }
        "#,
        target,
    );
    snapshot.entities[0].id = "00000000-0000-4000-8000-000000000001".parse()?;
    let first = snapshot.entities[0].id;
    snapshot.entities[0]
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert("other_scene".into(), ScriptValue::Text(other.to_string()));
    let (mut second, second_sources, _, _) = scripted(
        r#"
        fn on_start(ctx) {
            if ctx.request_scene(ctx.param("next_scene")) { throw "later request replaced first"; }
            ctx.translate_local(ctx.entity(), 3, 0, 0);
        }
        "#,
        other,
    );
    second.entities[0].id = "00000000-0000-4000-8000-000000000002".parse()?;
    let second_id = second.entities[0].id;
    snapshot.entities.extend(second.entities);
    snapshot.entities.reverse();
    sources.extend(second_sources);
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    let world = play.world_id();
    assert_eq!(x(&mut play, first), 2.);
    assert_eq!(x(&mut play, second_id), 3.);
    assert!(play.script_diagnostics().is_empty());
    let request = play.take_scene_request().unwrap();
    assert_eq!(request.target(), target);
    assert!(play.take_scene_request().is_none());
    assert_eq!(play.world_id(), world, "requests never load a scene inline");
    assert!(play.reject_scene_request(request, "target is not in the exported scene list"));
    let diagnostic = &play.script_diagnostics()[0];
    assert_eq!(diagnostic.world, world);
    assert_eq!(diagnostic.entity, first);
    assert_eq!(diagnostic.asset, asset);
    assert_eq!(diagnostic.tick, 0);
    assert_eq!(diagnostic.phase, "on_start");
    assert!(diagnostic.failed && diagnostic.message.contains(&target.to_string()));
    Ok(())
}

#[test]
fn failed_callbacks_discard_scene_requests_with_all_other_world_commands() -> Result<()> {
    for callback in ["on_start(ctx)", "on_fixed_update(ctx, dt)"] {
        for failure in [
            "throw \"callback failed\";",
            "ctx.set_velocity(ctx.entity(), 1, 0, 0);",
            "this.retained = ctx;",
            "ctx.request_scene(\"not-a-uuid\");",
            "loop {}",
        ] {
            let source = format!(
                "fn {callback} {{ ctx.request_scene(ctx.param(\"next_scene\")); ctx.translate_local(ctx.entity(), 3, 0, 0); {failure} }}"
            );
            let (snapshot, sources, id, _) = scripted(&source, AssetId::new());
            let author = snapshot.clone();
            let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
            let before = play.shared_snapshot();
            play.advance(FIXED_DELTA * 3.)?;
            assert!(play.take_scene_request().is_none(), "{source}");
            assert_eq!(x(&mut play, id), 0., "{source}");
            assert!(Arc::ptr_eq(&before, &play.shared_snapshot()), "{source}");
            assert_eq!(*play.shared_snapshot(), author, "{source}");
            assert_eq!(play.script_diagnostics().len(), 1, "{source}");
            assert!(play.script_diagnostics()[0].failed, "{source}");
        }
    }
    Ok(())
}

#[test]
fn trigger_requests_wait_for_the_host_and_stop_cannot_publish_a_transition() -> Result<()> {
    let target = AssetId::new();
    let (mut snapshot, sources, sensor, asset) = scripted(
        r#"
        fn on_trigger(ctx, other, entered) {
            if entered { ctx.request_scene(ctx.param("next_scene")); }
        }
        fn on_stop(ctx) { ctx.request_scene(ctx.param("next_scene")); }
        "#,
        target,
    );
    snapshot.entities[0].physics = Some(PhysicsBody {
        sensor: true,
        ..Default::default()
    });
    let mut body = EntityData::new("body");
    body.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    snapshot.entities.push(body);
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    let world = play.world_id();
    play.advance(FIXED_DELTA * 3.)?;
    assert_eq!(play.world_id(), world);
    let request = play.take_scene_request().unwrap();
    assert_eq!(request.target(), target);
    assert!(play.reject_scene_request(request, "missing product"));
    let diagnostic = &play.script_diagnostics()[0];
    assert_eq!(diagnostic.entity, sensor);
    assert_eq!(diagnostic.asset, asset);
    assert_eq!(diagnostic.phase, "on_trigger");
    assert_eq!(diagnostic.tick, 1);
    play.stop();
    assert!(play.take_scene_request().is_none());
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|entry| entry.failed && entry.phase == "on_stop")
    );

    let (snapshot, sources, _, _) = scripted(
        "fn on_start(ctx) { ctx.request_scene(ctx.param(\"next_scene\")); }",
        target,
    );
    let mut stopped = PlaySession::start_with_scripts(snapshot, sources)?;
    stopped.stop();
    assert!(stopped.take_scene_request().is_none());
    Ok(())
}

#[test]
fn rejected_loads_keep_the_binding_alive_and_diagnostics_bounded() -> Result<()> {
    let target = AssetId::new();
    let (snapshot, sources, id, asset) = scripted(
        r#"
        fn on_fixed_update(ctx, dt) {
            ctx.request_scene(ctx.param("next_scene"));
            ctx.translate_local(ctx.entity(), 1, 0, 0);
        }
        "#,
        target,
    );
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    let world = play.world_id();
    let detail = "无法读取资源🌠".repeat(1000);
    for attempt in 1..=300 {
        play.advance(FIXED_DELTA)?;
        let request = play.take_scene_request().unwrap();
        assert!(play.reject_scene_request(request, format!("尝试{attempt}：{detail}")));
    }
    assert_eq!(play.world_id(), world);
    assert_eq!(x(&mut play, id), 300.);
    let diagnostics = play.script_diagnostics();
    assert_eq!(diagnostics.len(), 256);
    assert_eq!(diagnostics.front().unwrap().sequence, 45);
    assert_eq!(diagnostics.back().unwrap().sequence, 300);
    assert!(diagnostics.iter().all(|entry| {
        entry.world == world
            && entry.entity == id
            && entry.asset == asset
            && entry.phase == "on_fixed_update"
            && entry.tick == entry.sequence
            && entry.failed
            && entry.message.len() <= 4096
            && entry.message.len() >= 4093
            && format!("请求切换场景 {target} 失败：尝试{}：{detail}", entry.tick)
                .starts_with(&entry.message)
    }));
    play.advance(FIXED_DELTA)?;
    assert_eq!(x(&mut play, id), 301.);
    assert!(play.take_scene_request().is_some());
    Ok(())
}

#[test]
fn acceptance_preserves_host_state_new_requests_and_ordered_world_diagnostics() -> Result<()> {
    let old_target = AssetId::new();
    let (snapshot, sources, _, _) = scripted(
        r#"
        fn on_start(ctx) { ctx.log("old start"); ctx.request_scene(ctx.param("next_scene")); }
        fn on_fixed_update(ctx, dt) { ctx.request_scene(ctx.param("next_scene")); }
        fn on_stop(ctx) { ctx.log("old stop"); }
        "#,
        old_target,
    );
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    let old_world = play.world_id();
    let stale = play.take_scene_request().unwrap();
    play.advance(FIXED_DELTA)?;
    play.set_paused(true);
    play.set_suspended(true);
    let target = AssetId::new();
    let (snapshot, sources, _, _) = scripted(
        r#"
        fn on_start(ctx) { ctx.log("new start"); ctx.request_scene(ctx.param("next_scene")); }
        fn on_stop(ctx) { ctx.log("new stop"); }
        "#,
        target,
    );
    let candidate = PlaySession::start_with_scripts(snapshot, sources)?;
    let new_world = candidate.world_id();
    play.accept_candidate(candidate)?;
    assert_eq!(play.world_id(), new_world);
    assert_ne!(new_world, old_world);
    assert!(play.is_paused() && play.is_suspended());
    assert_eq!(play.steps(), 0);
    let entries: Vec<_> = play
        .script_diagnostics()
        .iter()
        .map(|entry| (entry.world, entry.sequence, entry.message.as_str()))
        .collect();
    assert_eq!(
        entries,
        [
            (old_world, 1, "old start"),
            (old_world, 2, "old stop"),
            (new_world, 3, "new start")
        ]
    );
    assert!(!play.reject_scene_request(stale, "late completion from the old world"));
    assert_eq!(play.script_diagnostics().len(), 3);
    let request = play.take_scene_request().unwrap();
    assert_eq!(request.target(), target);
    assert!(play.take_scene_request().is_none());
    play.stop();
    let count = play.script_diagnostics().len();
    play.stop();
    assert_eq!(play.script_diagnostics().len(), count);
    assert!(!play.reject_scene_request(request, "completion after Stop"));
    Ok(())
}

#[test]
fn failed_preparation_and_stopped_candidates_leave_the_current_world_running() -> Result<()> {
    let target = AssetId::new();
    let (snapshot, sources, id, _) = scripted(
        r#"
        fn on_fixed_update(ctx, dt) {
            ctx.request_scene(ctx.param("next_scene"));
            ctx.translate_local(ctx.entity(), 1, 0, 0);
        }
        fn on_stop(ctx) { ctx.log("old stop"); }
        "#,
        target,
    );
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    play.advance(FIXED_DELTA)?;
    let world = play.world_id();
    let before = play.shared_snapshot();
    let request = play.take_scene_request().unwrap();
    let (snapshot, sources, _, _) = scripted("fn on_start( {", target);
    let error = PlaySession::start_with_scripts(snapshot, sources)
        .err()
        .unwrap();
    assert!(play.reject_scene_request(request, error.to_string()));
    assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
    let mut candidate = PlaySession::start(SceneSnapshot::default())?;
    candidate.stop();
    assert!(play.accept_candidate(candidate).is_err());
    assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
    assert_eq!(play.world_id(), world);
    assert_eq!(play.steps(), 1);
    assert!(
        !play
            .script_diagnostics()
            .iter()
            .any(|entry| entry.message == "old stop")
    );
    play.advance(FIXED_DELTA)?;
    assert_eq!(x(&mut play, id), 2.);
    assert!(play.take_scene_request().is_some());
    play.stop();
    let diagnostics = play.script_diagnostics().len();
    let candidate = PlaySession::start(SceneSnapshot::default())?;
    assert!(play.accept_candidate(candidate).is_err());
    assert_eq!(play.script_diagnostics().len(), diagnostics);
    Ok(())
}

#[test]
fn restart_after_a_transition_uses_the_new_entry_snapshot_sources_and_input_baseline() -> Result<()>
{
    let target = AssetId::new();
    let (author, sources, _, _) = scripted("fn on_stop(ctx) { ctx.log(\"old stop\"); }", target);
    let original_author = author.clone();
    let mut play = PlaySession::start_with_scripts(author.clone(), sources)?;
    let (mut entry, sources, id, _) = scripted(
        r#"
        fn on_start(ctx) {
            this.n = 0;
            ctx.log("new start");
            ctx.translate_local(ctx.entity(), 2, 0, 0);
            ctx.request_scene(ctx.param("next_scene"));
        }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            ctx.log(this.n.to_string());
            ctx.translate_local(ctx.entity(), 1, 0, 0);
            if ctx.pressed("interact") { ctx.translate_local(ctx.entity(), 100, 0, 0); }
        }
        fn on_stop(ctx) { ctx.log("new stop"); }
        "#,
        target,
    );
    entry.entities[0].transform.translation[0] = 10.;
    let original_entry = entry.clone();
    let candidate = PlaySession::start_with_scripts(entry.clone(), sources)?;
    play.accept_candidate(candidate)?;
    assert_eq!(x(&mut play, id), 12.);
    assert_eq!(play.take_scene_request().unwrap().target(), target);
    let mut held = input();
    held.held.set(Action::Interact, true);
    held.events.push(ActionEvent {
        action: Action::Interact,
        pressed: true,
    });
    play.advance_with_input(0., held.clone())?;
    play.advance_with_input(FIXED_DELTA, held.clone())?;
    assert_eq!(play.steps(), 0, "preparation time belongs to the host");
    play.advance_with_input(FIXED_DELTA * 2., held)?;
    assert_eq!(x(&mut play, id), 14., "entry input edges must not replay");
    play.set_field(&[id], schema::TRANSLATION, FieldValue::Vec3([50., 0., 0.]))?;
    let previous = play.world_id();
    let mut restart = input();
    restart.held.set(Action::Restart, true);
    restart.events.push(ActionEvent {
        action: Action::Restart,
        pressed: true,
    });
    play.advance_with_input(0., restart.clone())?;
    assert_ne!(play.world_id(), previous);
    assert_eq!(play.steps(), 0);
    assert_eq!(x(&mut play, id), 12.);
    assert_eq!(play.take_scene_request().unwrap().target(), target);
    play.advance_with_input(0., restart.clone())?;
    play.advance_with_input(FIXED_DELTA, restart.clone())?;
    play.advance_with_input(FIXED_DELTA, restart)?;
    assert_eq!(play.steps(), 1, "holding R must not restart repeatedly");
    assert_eq!(x(&mut play, id), 13.);
    assert_eq!(play.script_diagnostics().back().unwrap().message, "1");
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|entry| entry.message == "old stop")
            .count(),
        1
    );
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|entry| entry.message == "new stop")
            .count(),
        1
    );
    assert_eq!(author, original_author);
    assert_eq!(entry, original_entry);
    Ok(())
}
