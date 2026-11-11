use anyhow::Result;
use comet_core::{
    AssetId, Checkpoint, EntityId, GameId, ProgressEntry, ProgressValue, ProgressValues,
};
use comet_runtime::{FIXED_DELTA, PlaySession, SceneResources};
use comet_scene::{EntityData, SceneSnapshot, Script};
use std::sync::Arc;

fn fixture(source: &str) -> (SceneSnapshot, SceneResources, EntityId) {
    let mut entity = EntityData::new("progress owner");
    let asset = AssetId::new();
    entity.script = Some(Script::new(asset));
    let owner = entity.id;
    (
        SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        SceneResources {
            scripts: [(asset, source.into())].into(),
            ..Default::default()
        },
        owner,
    )
}
fn value(play: &PlaySession, scene: AssetId, owner: EntityId, key: &str) -> Option<ProgressValue> {
    play.progress_snapshot().get(scene, owner, key).cloned()
}
fn start(source: &str) -> Result<(PlaySession, AssetId, EntityId)> {
    let (snapshot, resources, owner) = fixture(source);
    let scene = AssetId::new();
    Ok((
        PlaySession::start_with_progress(snapshot, resources, scene, Arc::new(Default::default()))?,
        scene,
        owner,
    ))
}

#[test]
fn callbacks_read_their_entry_snapshot_and_publish_only_explicit_typed_values() -> Result<()> {
    let (mut play, scene, owner) = start(
        r#"
        fn on_start(ctx) {
            if ctx.progress("missing", 42) != 42 { throw "default"; }
            ctx.set_progress("flag", true);
            if ctx.progress("flag", false) { throw "write leaked into read snapshot"; }
            ctx.set_progress("count", 9223372036854775807);
            ctx.set_progress("amount", 1.25);
            ctx.set_progress("large_number", 1e300);
            ctx.set_progress("message", "中文🌠");
        }
        fn on_fixed_update(ctx, dt) {
            if !ctx.progress("flag", false) { throw "previous callback did not commit"; }
            ctx.set_progress("count", 2);
            ctx.remove_progress("message");
        }
        "#,
    )?;
    let initial = play.progress_snapshot();
    assert!(initial.get(scene, owner, "missing").is_none());
    assert_eq!(
        initial.get(scene, owner, "count"),
        Some(&ProgressValue::Integer(i64::MAX))
    );
    assert_eq!(
        initial.get(scene, owner, "amount"),
        Some(&ProgressValue::Number(1.25))
    );
    assert_eq!(
        initial.get(scene, owner, "large_number"),
        Some(&ProgressValue::Number(1e300))
    );
    assert_eq!(
        initial.get(scene, owner, "message"),
        Some(&ProgressValue::Text("中文🌠".into()))
    );
    play.advance(FIXED_DELTA)?;
    let changed = play.progress_snapshot();
    assert!(!Arc::ptr_eq(&initial, &changed));
    assert_eq!(
        changed.get(scene, owner, "count"),
        Some(&ProgressValue::Integer(2))
    );
    assert!(changed.get(scene, owner, "message").is_none());
    play.advance(FIXED_DELTA)?;
    assert!(
        Arc::ptr_eq(&changed, &play.progress_snapshot()),
        "equal writes and absent removals are no-ops"
    );
    assert!(play.script_diagnostics().is_empty());
    assert_eq!(
        initial.get(scene, owner, "count"),
        Some(&ProgressValue::Integer(i64::MAX))
    );
    Ok(())
}

#[test]
fn unscoped_sessions_reject_progress_without_changing_old_gameplay() -> Result<()> {
    let (snapshot, resources, owner) = fixture(
        r#"fn on_start(ctx) {
            ctx.set_hud("must not publish");
            ctx.translate_local(ctx.entity(), 2, 0, 0);
            ctx.set_progress("flag", true);
        }"#,
    );
    let mut play = PlaySession::start_with_resources(snapshot, resources)?;
    assert_eq!(play.scene_asset(), None);
    assert!(play.progress_snapshot().entries.is_empty());
    assert!(play.hud_frame().is_none());
    assert_eq!(play.snapshot().entities[0].transform.translation, [0.; 3]);
    assert_eq!(play.script_diagnostics()[0].entity, owner);
    assert!(
        play.script_diagnostics()[0]
            .message
            .contains("scene asset scope")
    );
    for expression in [
        "ctx.progress(\"flag\", false)",
        "ctx.remove_progress(\"flag\")",
    ] {
        let (snapshot, resources, _) = fixture(&format!("fn on_start(ctx) {{ {expression}; }}"));
        let play = PlaySession::start_with_resources(snapshot, resources)?;
        assert!(
            play.script_diagnostics()[0]
                .message
                .contains("scene asset scope")
        );
    }
    let (snapshot, resources, _) = fixture("fn on_start(ctx) { ctx.set_hud(\"old gameplay\"); }");
    assert!(
        PlaySession::start_with_resources(snapshot, resources)?
            .hud_frame()
            .is_some()
    );
    Ok(())
}

#[test]
fn invalid_progress_or_later_domain_commands_leave_the_entire_callback_unpublished() -> Result<()> {
    let target = AssetId::new();
    for command in [
        "ctx.set_progress(\"flag\", 1);",
        "ctx.progress(\"flag\", 0);",
        "ctx.set_progress(\"array\", [1, 2]);",
        "ctx.set_progress(\"overflow\", 1e300 * 1e300);",
        "ctx.set_progress(\"bad\", ctx.entity());",
        "ctx.set_position(ctx.entity(), 1e300, 0, 0);",
        "ctx.play_sound(ctx.entity());",
        "for n in 0..257 { ctx.set_progress(\"same\", n); }",
        "for n in 0..120 { ctx.set_progress(\"k\" + n, true); }",
    ] {
        let source = format!(
            r#"
            fn on_start(ctx) {{ ctx.set_progress("flag", false); }}
            fn on_fixed_update(ctx, dt) {{
                ctx.set_progress("flag", true);
                ctx.set_hud("must not publish");
                ctx.log("must not publish");
                ctx.request_scene("{target}");
                ctx.translate_local(ctx.entity(), 3, 0, 0);
                {command}
            }}"#
        );
        let (mut play, scene, owner) = start(&source)?;
        let accepted = play.progress_snapshot();
        play.advance(FIXED_DELTA)?;
        assert!(
            Arc::ptr_eq(&accepted, &play.progress_snapshot()),
            "{command}"
        );
        assert_eq!(
            value(&play, scene, owner, "flag"),
            Some(ProgressValue::Bool(false))
        );
        assert!(play.hud_frame().is_none(), "{command}");
        assert!(play.take_scene_request().is_none(), "{command}");
        assert_eq!(play.snapshot().entities[0].transform.translation, [0.; 3]);
        assert_eq!(play.script_diagnostics().len(), 1, "{command}");
        assert!(play.script_diagnostics()[0].failed);
        play.advance(FIXED_DELTA)?;
        assert_eq!(
            play.script_diagnostics().len(),
            1,
            "failed binding is disabled"
        );
    }
    Ok(())
}

#[test]
fn entry_baseline_cross_scene_candidates_and_checkpoint_continue_are_isolated() -> Result<()> {
    let (snapshot_a, resources_a, owner_a) =
        fixture(r#"fn on_fixed_update(ctx, dt) { ctx.set_progress("collected", true); }"#);
    let scene_a = AssetId::new();
    let mut a = PlaySession::start_with_progress(
        snapshot_a,
        resources_a,
        scene_a,
        Arc::new(Default::default()),
    )?;
    a.advance(FIXED_DELTA)?;
    let current_a = a.progress_snapshot();
    let (snapshot_b, resources_b, owner_b) = fixture(
        r#"
        fn on_start(ctx) { ctx.set_progress("entries", ctx.progress("entries", 0) + 1); }
        fn on_fixed_update(ctx, dt) { ctx.set_progress("collected", true); }
        "#,
    );
    let scene_b = AssetId::new();
    let candidate = a.prepare_scene(snapshot_b.clone(), resources_b.clone(), scene_b)?;
    assert!(Arc::ptr_eq(&current_a, &a.progress_snapshot()));
    assert_eq!(
        value(&candidate, scene_b, owner_b, "entries"),
        Some(ProgressValue::Integer(1))
    );
    assert!(current_a.get(scene_b, owner_b, "entries").is_none());
    drop(candidate);
    assert!(
        Arc::ptr_eq(&current_a, &a.progress_snapshot()),
        "abandoned candidate cannot publish initialization"
    );
    let candidate = a.prepare_scene(snapshot_b.clone(), resources_b.clone(), scene_b)?;
    a.set_paused(true);
    a.set_suspended(true);
    a.accept_candidate(candidate)?;
    assert!(a.is_paused() && a.is_suspended());
    assert_eq!(a.scene_asset(), Some(scene_b));
    assert_eq!(
        value(&a, scene_a, owner_a, "collected"),
        Some(ProgressValue::Bool(true))
    );
    a.set_suspended(false);
    a.single_step()?;
    assert_eq!(
        value(&a, scene_b, owner_b, "collected"),
        Some(ProgressValue::Bool(true))
    );
    let world = a.world_id();
    a.restart()?;
    assert_ne!(world, a.world_id());
    assert!(a.is_paused());
    assert_eq!(
        value(&a, scene_b, owner_b, "entries"),
        Some(ProgressValue::Integer(1)),
        "entry is before on_start"
    );
    assert_eq!(value(&a, scene_b, owner_b, "collected"), None);
    assert_eq!(
        value(&a, scene_a, owner_a, "collected"),
        Some(ProgressValue::Bool(true))
    );
    a.single_step()?;
    let checkpoint = Checkpoint::new(GameId::new(), scene_b, (*a.progress_snapshot()).clone())?;
    let checkpoint = Checkpoint::from_bytes(&checkpoint.to_bytes()?)?;
    let mut resumed = PlaySession::start_with_progress(
        snapshot_b,
        resources_b,
        checkpoint.scene,
        Arc::new(checkpoint.progress),
    )?;
    assert_eq!(
        value(&resumed, scene_b, owner_b, "entries"),
        Some(ProgressValue::Integer(2))
    );
    resumed.restart()?;
    assert_eq!(
        value(&resumed, scene_b, owner_b, "entries"),
        Some(ProgressValue::Integer(2))
    );
    assert_eq!(
        value(&resumed, scene_b, owner_b, "collected"),
        Some(ProgressValue::Bool(true))
    );
    assert!(
        current_a.get(scene_b, owner_b, "entries").is_none(),
        "accepted initialization never mutates retained snapshots"
    );
    Ok(())
}

#[test]
fn scoped_owner_keys_do_not_alias_and_progress_survives_only_accepted_candidates() -> Result<()> {
    let (mut snapshot, mut resources, owner) =
        fixture("fn on_start(ctx) { ctx.set_progress(\"same\", true); }");
    let (second, more, other_owner) =
        fixture("fn on_start(ctx) { ctx.set_progress(\"same\", 7); }");
    snapshot.entities.extend(second.entities);
    resources.scripts.extend(more.scripts);
    let scene = AssetId::new();
    let foreign_scene = AssetId::new();
    let seed = Arc::new(ProgressValues {
        entries: vec![ProgressEntry {
            scene: foreign_scene,
            owner,
            key: "same".into(),
            value: ProgressValue::Text("other scene".into()),
        }],
    });
    let mut play = PlaySession::start_with_progress(snapshot, resources, scene, seed.clone())?;
    assert_eq!(
        value(&play, scene, owner, "same"),
        Some(ProgressValue::Bool(true))
    );
    assert_eq!(
        value(&play, scene, other_owner, "same"),
        Some(ProgressValue::Integer(7))
    );
    assert_eq!(
        value(&play, foreign_scene, owner, "same"),
        Some(ProgressValue::Text("other scene".into()))
    );
    assert_eq!(seed.entries.len(), 1);
    let accepted = play.progress_snapshot();
    let (bad_snapshot, _, _) = fixture("fn on_start(ctx) {} ");
    assert!(
        play.prepare_scene(bad_snapshot, SceneResources::default(), AssetId::new())
            .is_err()
    );
    assert!(Arc::ptr_eq(&accepted, &play.progress_snapshot()));
    let mut candidate = play.prepare_scene(
        SceneSnapshot::default(),
        SceneResources::default(),
        AssetId::new(),
    )?;
    candidate.stop();
    assert!(play.accept_candidate(candidate).is_err());
    assert!(Arc::ptr_eq(&accepted, &play.progress_snapshot()));
    play.stop();
    assert!(
        play.prepare_scene(
            SceneSnapshot::default(),
            SceneResources::default(),
            AssetId::new()
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn pause_single_step_and_stop_keep_progress_read_permissions_and_restore_entry_values() -> Result<()>
{
    let (mut play, scene, owner) = start(
        r#"
        fn on_start(ctx) { ctx.set_progress("count", ctx.progress("count", 0) + 1); }
        fn on_fixed_update(ctx, dt) { ctx.set_progress("count", ctx.progress("count", 0) + 10); }
        fn on_stop(ctx) {
            ctx.log("stopping " + ctx.progress("count", 0));
            ctx.set_progress("count", 999);
        }
        "#,
    )?;
    let initial = play.progress_snapshot();
    play.set_paused(true);
    play.advance(FIXED_DELTA * 3.)?;
    assert!(Arc::ptr_eq(&initial, &play.progress_snapshot()));
    play.single_step()?;
    assert_eq!(
        value(&play, scene, owner, "count"),
        Some(ProgressValue::Integer(11))
    );
    let changed = play.progress_snapshot();
    play.stop();
    assert!(Arc::ptr_eq(&changed, &play.progress_snapshot()));
    assert_eq!(
        play.script_diagnostics().len(),
        1,
        "failed stop callback discards its ordinary logs"
    );
    assert!(play.script_diagnostics()[0].message.contains("on_stop"));
    assert!(play.restart().is_err());
    Ok(())
}
