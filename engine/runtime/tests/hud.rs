use anyhow::Result;
use comet_asset_data::AudioData;
use comet_core::{AssetId, EntityId, HudFrame, MAX_HUD_LABEL_BYTES, MAX_HUD_LABELS};
use comet_runtime::{FIXED_DELTA, PlaySession, SceneResources, ScriptLibrary};
use comet_scene::{AudioSource, EntityData, SceneSnapshot, Script, ScriptValue};
use std::sync::Arc;

fn scripted(source: &str) -> (SceneSnapshot, ScriptLibrary, EntityId) {
    let mut entity = EntityData::new("HUD controller");
    let asset = AssetId::new();
    entity.script = Some(Script::new(asset));
    let id = entity.id;
    (
        SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        [(asset, source.into())].into(),
        id,
    )
}
fn text(frame: &HudFrame, owner: EntityId) -> &str {
    &frame
        .labels
        .iter()
        .find(|label| label.owner == owner)
        .unwrap()
        .text
}

#[test]
fn script_owned_labels_are_sorted_immutable_and_reuse_the_batch_until_content_changes() -> Result<()>
{
    assert!(
        PlaySession::start(SceneSnapshot::default())?
            .hud_frame()
            .is_none()
    );
    let (mut scene, mut scripts, _) = scripted(
        r#"
        fn on_start(ctx) { this.n = 0; ctx.set_hud("等待收集🌠"); }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            ctx.set_hud("临时文字");
            if this.n == 1 { ctx.set_hud("等待收集🌠"); }
            else if this.n == 2 { ctx.set_hud("收集完成"); }
            else { ctx.clear_hud(); }
        }
        "#,
    );
    scene.entities[0].id = "00000000-0000-4000-8000-000000000002".parse()?;
    let owner = scene.entities[0].id;
    let (other, other_scripts, _) = scripted(
        r#"
        fn on_start(ctx) { this.n = 0; ctx.set_hud("操作提示"); }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            if this.n == 5 { ctx.clear_hud(); }
        }
        "#,
    );
    let mut other = other.entities.into_iter().next().unwrap();
    other.id = "00000000-0000-4000-8000-000000000001".parse()?;
    let other_id = other.id;
    scene.entities.push(other);
    scripts.extend(other_scripts);
    let mut author = scene.clone();
    author.canonicalize();
    let mut play = PlaySession::start_with_scripts(scene, scripts)?;
    let initial = play.hud_frame().unwrap();
    initial.validate()?;
    assert_eq!(initial.world, play.world_id());
    assert_eq!(
        initial
            .labels
            .iter()
            .map(|label| label.owner)
            .collect::<Vec<_>>(),
        [other_id, owner]
    );
    assert_eq!(text(&initial, owner), "等待收集🌠");
    play.advance(FIXED_DELTA)?;
    assert!(Arc::ptr_eq(
        &initial.labels,
        &play.hud_frame().unwrap().labels
    ));
    play.advance(FIXED_DELTA)?;
    let changed = play.hud_frame().unwrap();
    assert!(!Arc::ptr_eq(&initial.labels, &changed.labels));
    assert_eq!(text(&changed, owner), "收集完成");
    assert_eq!(text(&initial, owner), "等待收集🌠");
    play.advance(FIXED_DELTA)?;
    let cleared = play.hud_frame().unwrap();
    assert_eq!(cleared.labels.len(), 1);
    assert_eq!(text(&cleared, other_id), "操作提示");
    play.advance(FIXED_DELTA)?;
    assert!(Arc::ptr_eq(
        &cleared.labels,
        &play.hud_frame().unwrap().labels
    ));
    play.advance(FIXED_DELTA)?;
    assert!(play.hud_frame().is_none());
    assert_eq!(text(&cleared, other_id), "操作提示");
    assert_eq!(
        play.snapshot(),
        author,
        "HUD intents do not edit author components"
    );
    Ok(())
}

#[test]
fn failed_mixed_callbacks_keep_hud_scene_sound_requests_and_ordinary_logs_unpublished() -> Result<()>
{
    let (mut scene, scripts, owner) = scripted(
        r#"
        fn on_start(ctx) { ctx.set_hud("原任务"); }
        fn on_fixed_update(ctx, dt) {
            ctx.set_hud("不应显示");
            ctx.translate_local(ctx.entity(), 1, 0, 0);
            ctx.play_sound(ctx.entity());
            ctx.request_scene(ctx.param("next"));
            ctx.log("不应提交");
            ctx.set_sound_volume(ctx.entity(), 2);
        }
        "#,
    );
    let clip = AssetId::new();
    scene.entities[0].audio_source = Some(AudioSource::new(clip));
    scene.entities[0]
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert("next".into(), ScriptValue::Text(AssetId::new().to_string()));
    let mut play = PlaySession::start_with_resources(
        scene,
        SceneResources {
            scripts,
            audio: [(
                clip,
                Arc::new(AudioData {
                    version: 1,
                    sample_rate: 48_000,
                    channels: 1,
                    samples: vec![1; 16].into(),
                }),
            )]
            .into(),
            ..Default::default()
        },
    )?;
    let hud = play.hud_frame().unwrap();
    let snapshot = play.shared_snapshot();
    let sound = play.audio_frame()?.unwrap();
    play.advance(FIXED_DELTA)?;
    assert!(Arc::ptr_eq(&hud.labels, &play.hud_frame().unwrap().labels));
    assert!(Arc::ptr_eq(&snapshot, &play.shared_snapshot()));
    assert!(Arc::ptr_eq(
        &sound.voices,
        &play.audio_frame()?.unwrap().voices
    ));
    assert!(play.take_scene_request().is_none());
    assert_eq!(text(&hud, owner), "原任务");
    assert_eq!(play.script_diagnostics().len(), 1);
    assert!(play.script_diagnostics()[0].failed);
    assert!(
        play.script_diagnostics()[0]
            .message
            .contains("sound volume")
    );
    play.advance(FIXED_DELTA)?;
    assert_eq!(
        play.script_diagnostics().len(),
        1,
        "the failed binding is disabled"
    );
    assert!(Arc::ptr_eq(&snapshot, &play.shared_snapshot()));
    Ok(())
}

#[test]
fn hud_uses_the_shared_command_budget_and_validates_every_queued_text_before_publishing()
-> Result<()> {
    for accepted in [true, false] {
        let (scene, scripts, owner) = scripted(&format!(
            r#"
            fn on_start(ctx) {{ ctx.set_hud("基线"); }}
            fn on_fixed_update(ctx, dt) {{
                ctx.translate_local(ctx.entity(), 1, 0, 0);
                for n in 0..{} {{ ctx.set_hud(n.to_string()); }}
                ctx.log("完成");
            }}
        "#,
            if accepted { 255 } else { 256 }
        ));
        let mut play = PlaySession::start_with_scripts(scene, scripts)?;
        let initial = play.hud_frame().unwrap();
        play.advance(FIXED_DELTA)?;
        let current = play.hud_frame().unwrap();
        assert_eq!(text(&current, owner), if accepted { "254" } else { "基线" });
        assert_eq!(Arc::ptr_eq(&initial.labels, &current.labels), !accepted);
        assert_eq!(
            play.snapshot().entities[0].transform.translation[0],
            if accepted { 1. } else { 0. }
        );
        assert_eq!(play.script_diagnostics().len(), 1);
        assert_eq!(play.script_diagnostics()[0].failed, !accepted);
        assert!(play.script_diagnostics()[0].message.contains(if accepted {
            "完成"
        } else {
            "command budget"
        }));
    }
    let (scene, scripts, _) = scripted(&format!(
        r#"
        fn on_start(ctx) {{ ctx.set_hud("原任务"); }}
        fn on_fixed_update(ctx, dt) {{
            ctx.set_hud("{}");
            ctx.clear_hud();
            ctx.translate_local(ctx.entity(), 1, 0, 0);
            ctx.log("不应提交");
        }}
    "#,
        "a".repeat(MAX_HUD_LABEL_BYTES + 1)
    ));
    let mut play = PlaySession::start_with_scripts(scene, scripts)?;
    let initial = play.hud_frame().unwrap();
    play.advance(FIXED_DELTA)?;
    assert!(Arc::ptr_eq(
        &initial.labels,
        &play.hud_frame().unwrap().labels
    ));
    assert_eq!(play.snapshot().entities[0].transform.translation[0], 0.);
    assert_eq!(play.script_diagnostics().len(), 1);
    assert!(play.script_diagnostics()[0].failed);
    assert!(
        play.script_diagnostics()[0]
            .message
            .contains("text byte budget")
    );
    Ok(())
}

#[test]
fn per_world_limits_preserve_previous_bindings_when_a_later_binding_exceeds_the_budget()
-> Result<()> {
    for (count, text_size, accepted) in [
        (MAX_HUD_LABELS + 1, 1, MAX_HUD_LABELS),
        (5, MAX_HUD_LABEL_BYTES, 4),
    ] {
        let source = format!(
            "fn on_start(ctx) {{ ctx.set_hud(\"{}\"); ctx.translate_local(ctx.entity(), 1, 0, 0); }}",
            "a".repeat(text_size)
        );
        let mut snapshot = SceneSnapshot::default();
        let mut sources = ScriptLibrary::new();
        for _ in 0..count {
            let (scene, scripts, _) = scripted(&source);
            snapshot.entities.extend(scene.entities);
            sources.extend(scripts);
        }
        let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
        let frame = play.hud_frame().unwrap();
        frame.validate()?;
        assert_eq!(frame.labels.len(), accepted);
        let current = play.snapshot();
        assert!(
            current.entities[..accepted]
                .iter()
                .all(|entity| entity.transform.translation[0] == 1.)
        );
        assert!(
            current.entities[accepted..]
                .iter()
                .all(|entity| entity.transform.translation[0] == 0.)
        );
        assert_eq!(play.script_diagnostics().len(), count - accepted);
        assert!(
            play.script_diagnostics()
                .iter()
                .all(|entry| entry.failed && entry.phase == "on_start")
        );
    }
    Ok(())
}

#[test]
fn pause_restart_stop_and_candidate_acceptance_keep_worlds_and_cached_frames_isolated() -> Result<()>
{
    let source = r#"
        fn on_start(ctx) { this.n = 0; ctx.set_hud("入关任务"); }
        fn on_fixed_update(ctx, dt) { this.n += 1; ctx.set_hud(this.n.to_string()); }
        fn on_stop(ctx) { ctx.log("不应提交"); ctx.clear_hud(); }
    "#;
    let (scene, scripts, owner) = scripted(source);
    let mut play = PlaySession::start_with_scripts(scene, scripts)?;
    let initial = play.hud_frame().unwrap();
    play.set_paused(true);
    play.advance(1.)?;
    assert!(Arc::ptr_eq(
        &initial.labels,
        &play.hud_frame().unwrap().labels
    ));
    play.single_step()?;
    let stepped = play.hud_frame().unwrap();
    assert_eq!(text(&stepped, owner), "1");
    play.set_suspended(true);
    play.advance(1.)?;
    assert!(play.single_step().is_err());
    assert!(Arc::ptr_eq(
        &stepped.labels,
        &play.hud_frame().unwrap().labels
    ));

    let (scene, scripts, _) = scripted("fn on_start( {");
    assert!(PlaySession::start_with_scripts(scene, scripts).is_err());
    let (scene, scripts, _) = scripted("fn on_start(ctx) { ctx.set_hud(\"被拒候选\"); }");
    let mut rejected = PlaySession::start_with_scripts(scene, scripts)?;
    rejected.stop();
    assert!(play.accept_candidate(rejected).is_err());
    assert!(Arc::ptr_eq(
        &stepped.labels,
        &play.hud_frame().unwrap().labels
    ));

    let (scene, scripts, next_owner) = scripted(source);
    let candidate = PlaySession::start_with_scripts(scene, scripts)?;
    let prepared = candidate.hud_frame().unwrap();
    assert_eq!(text(&play.hud_frame().unwrap(), owner), "1");
    play.accept_candidate(candidate)?;
    let accepted = play.hud_frame().unwrap();
    assert_eq!(accepted.world, prepared.world);
    assert_ne!(accepted.world, stepped.world);
    assert!(Arc::ptr_eq(&accepted.labels, &prepared.labels));
    assert_eq!(accepted.labels.len(), 1);
    assert_eq!(text(&accepted, next_owner), "入关任务");
    assert!(play.is_paused() && play.is_suspended());
    assert_eq!(play.script_diagnostics().len(), 1);
    assert!(play.script_diagnostics()[0].failed && play.script_diagnostics()[0].phase == "on_stop");
    play.set_suspended(false);
    play.single_step()?;
    assert_eq!(text(&play.hud_frame().unwrap(), next_owner), "1");
    play.restart()?;
    let restarted = play.hud_frame().unwrap();
    assert_ne!(restarted.world, accepted.world);
    assert_eq!(text(&restarted, next_owner), "入关任务");
    assert_eq!(text(&stepped, owner), "1");
    play.stop();
    assert!(play.hud_frame().is_none());
    let count = play.script_diagnostics().len();
    play.stop();
    assert_eq!(play.script_diagnostics().len(), count);
    Ok(())
}
