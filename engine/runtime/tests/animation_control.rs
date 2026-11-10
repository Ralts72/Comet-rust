use anyhow::Result;
use comet_asset_data::{
    AnimationChannel, AnimationClipData, AnimationInterpolation, AnimationValues, AudioData,
    SkeletonData, SkeletonNode,
};
use comet_core::{AssetId, EntityId};
use comet_runtime::{AnimationState, FIXED_DELTA, PlaySession, RuntimeResources};
use comet_scene::{
    Animator, AudioSource, EntityData, EntityRef, SceneSnapshot, Script, ScriptValue, Transform,
    schema,
};
use std::sync::Arc;

struct Fixture {
    scene: SceneSnapshot,
    resources: RuntimeResources,
    owner: EntityId,
    bone: EntityId,
}
fn bind(f: &mut Fixture, entity: EntityId, source: &str) {
    let asset = AssetId::new();
    let mut script = Script::new(asset);
    script.parameters.insert(
        "animation".into(),
        ScriptValue::Entity(Some(EntityRef::local(f.owner))),
    );
    script.parameters.insert(
        "bone".into(),
        ScriptValue::Entity(Some(EntityRef::local(f.bone))),
    );
    script
        .parameters
        .insert("next".into(), ScriptValue::Text(AssetId::new().to_string()));
    f.scene
        .entities
        .iter_mut()
        .find(|e| e.id == entity)
        .unwrap()
        .script = Some(script);
    f.resources.scripts.insert(asset, source.into());
}
fn fixture(source: &str) -> Fixture {
    let skeleton = AssetId::new();
    let clip = AssetId::new();
    let sound = AssetId::new();
    let node = EntityId::new();
    let mut owner = EntityData::new("animated actor");
    let mut bone = EntityData::new("bone");
    bone.parent = Some(owner.id);
    owner.audio_source = Some(AudioSource::new(sound));
    owner.animator = Some(Animator {
        controller: None,
        skeleton,
        clip: Some(clip),
        bindings: [(node, Some(EntityRef::local(bone.id)))].into(),
        looping: false,
        speed: 1.,
    });
    let (owner_id, bone_id) = (owner.id, bone.id);
    let mut f = Fixture {
        scene: SceneSnapshot {
            entities: vec![owner, bone],
            ..Default::default()
        },
        resources: RuntimeResources {
            skeletons: [(
                skeleton,
                Arc::new(SkeletonData {
                    version: 1,
                    nodes: vec![SkeletonNode {
                        id: node,
                        name: "bone".into(),
                        parent: None,
                        rest: Transform::default(),
                    }],
                }),
            )]
            .into(),
            animations: [(
                clip,
                Arc::new(AnimationClipData {
                    events: vec![],
                    version: 1,
                    skeleton,
                    duration: 1.,
                    channels: vec![AnimationChannel {
                        node,
                        interpolation: AnimationInterpolation::Linear,
                        times: vec![0., 1.],
                        values: AnimationValues::Translation(vec![[0.; 3], [1., 0., 0.]]),
                    }],
                }),
            )]
            .into(),
            audio: [(
                sound,
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
        owner: owner_id,
        bone: bone_id,
    };
    if !source.is_empty() {
        bind(&mut f, owner_id, source);
    }
    f
}
fn advance(play: &mut PlaySession, ticks: usize) -> Result<()> {
    for _ in 0..ticks {
        play.advance(FIXED_DELTA)?;
    }
    Ok(())
}

#[test]
fn animation_markers_cross_whole_loops_once_and_follow_pause_step_replay_and_world_identity()
-> Result<()> {
    let mut f = fixture(
        r#"
        fn on_start(ctx) { this.ticks = 0; }
        fn on_animation_event(ctx, animator, name) {
            if animator.id() != ctx.entity().id() { throw "wrong animator"; }
            let sample = ctx.animation_state(animator);
            if sample.time < 0.0 { throw "unpublished sample"; }
            ctx.log(name);
        }
        fn on_fixed_update(ctx, dt) {
            this.ticks += 1;
            if this.ticks == 3 { ctx.play_animation(ctx.entity()); }
        }
    "#,
    );
    let animator = f
        .scene
        .entities
        .iter_mut()
        .find_map(|e| e.animator.as_mut())
        .unwrap();
    animator.looping = true;
    animator.speed = 150.;
    let clip = Arc::make_mut(f.resources.animations.values_mut().next().unwrap());
    clip.version = 3;
    clip.events = [(0., "entry"), (0.5, "middle"), (1., "end")]
        .map(|(phase, name)| comet_asset_data::AnimationEvent {
            phase,
            name: name.into(),
        })
        .into();
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    assert!(
        play.script_diagnostics().is_empty(),
        "initial pose does not emit gameplay events"
    );
    advance(&mut play, 1)?;
    let messages = |p: &PlaySession| {
        p.script_diagnostics()
            .iter()
            .filter(|e| e.world == p.world_id())
            .map(|e| e.message.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        messages(&play),
        [
            "entry", "middle", "end", "entry", "middle", "end", "entry", "middle"
        ]
    );
    assert_time(&mut play, f.owner, f.bone, 0.5);
    play.set_paused(true);
    advance(&mut play, 3)?;
    assert_eq!(messages(&play).len(), 8);
    play.single_step()?;
    assert_eq!(messages(&play).len(), 16);
    assert_time(&mut play, f.owner, f.bone, 0.);
    play.set_suspended(true);
    assert!(play.single_step().is_err());
    advance(&mut play, 3)?;
    assert_eq!(messages(&play).len(), 16);
    play.set_suspended(false);
    play.set_paused(false);
    play.advance(FIXED_DELTA)?;
    assert_eq!(
        messages(&play).len(),
        16,
        "replay first publishes a zero pose"
    );
    advance(&mut play, 1)?;
    assert_eq!(
        &messages(&play)[16..],
        [
            "entry", "middle", "end", "entry", "middle", "end", "entry", "middle"
        ]
    );
    let old_world = play.world_id();
    play.restart()?;
    assert_ne!(play.world_id(), old_world);
    assert!(messages(&play).is_empty());
    advance(&mut play, 2)?;
    assert_eq!(messages(&play).len(), 8);
    Ok(())
}

#[test]
fn event_overflow_keeps_the_sample_atomic_and_failed_callbacks_do_not_publish_world_commands()
-> Result<()> {
    for overflow in [true, false] {
        let mut f = fixture(
            r#"
            fn on_animation_event(ctx, animator, name) {
                ctx.set_scale(ctx.entity(), 2, 2, 2);
                ctx.play_sound(ctx.entity());
                ctx.set_hud("must not publish");
                ctx.request_scene(ctx.param("next"));
                ctx.log("must not publish");
                ctx.play_animation(ctx.param("bone"));
            }
        "#,
        );
        let clip = Arc::make_mut(f.resources.animations.values_mut().next().unwrap());
        clip.version = 3;
        clip.events = vec![comet_asset_data::AnimationEvent {
            phase: 0.,
            name: "hit".into(),
        }];
        if overflow {
            let animator = f
                .scene
                .entities
                .iter_mut()
                .find_map(|e| e.animator.as_mut())
                .unwrap();
            animator.looping = true;
            animator.speed = 1e8;
        }
        let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
        let before = play.shared_snapshot();
        let initial = state(&play, f.owner);
        let result = play.advance(FIXED_DELTA);
        if overflow {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("event step budget")
            );
            assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
            assert_eq!(state(&play, f.owner), initial);
            assert!(play.script_diagnostics().is_empty());
        } else {
            result?;
            assert_time(&mut play, f.owner, f.bone, FIXED_DELTA);
            assert_eq!(play.script_diagnostics().len(), 1);
            assert!(play.script_diagnostics()[0].failed);
            assert_eq!(play.script_diagnostics()[0].phase, "on_animation_event");
            advance(&mut play, 3)?;
            assert_eq!(play.script_diagnostics().len(), 1);
        }
        assert_eq!(
            play.shared_snapshot()
                .entities
                .iter()
                .find(|e| e.id == f.owner)
                .unwrap()
                .transform
                .scale,
            [1.; 3]
        );
        assert_eq!(play.audio_frame()?.unwrap().voices[0].generation, 0);
        assert!(play.hud_frame().is_none());
        assert!(play.take_scene_request().is_none());
    }
    Ok(())
}
fn state(play: &PlaySession, owner: EntityId) -> AnimationState {
    play.animation_state(owner).unwrap()
}
fn assert_time(play: &mut PlaySession, owner: EntityId, bone: EntityId, expected: f64) {
    assert!((state(play, owner).time - expected).abs() < 1e-12);
    let snapshot = play.shared_snapshot();
    let x = snapshot
        .entities
        .iter()
        .find(|e| e.id == bone)
        .unwrap()
        .transform
        .translation[0];
    assert!(
        (f64::from(x) - expected).abs() < 1e-6,
        "the reported clock must match the published scene pose"
    );
}

#[test]
fn on_start_reads_the_initial_sample_then_publishes_controls_at_zero_without_losing_a_default_tick()
-> Result<()> {
    let f = fixture(
        r#"
        fn on_start(ctx) {
            let before = ctx.animation_state(ctx.entity());
            if before.time != 0.0 || !before.playing || before.speed != 1.0 { throw "initial sample"; }
            ctx.set_animation_speed(ctx.entity(), 2);
            ctx.pause_animation(ctx.entity());
            let after = ctx.animation_state(ctx.entity());
            if after.time != before.time || after.speed != before.speed || after.playing != before.playing { throw "intent leaked"; }
            ctx.log("initial snapshot");
        }
    "#,
    );
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    assert_eq!(
        state(&play, f.owner),
        AnimationState {
            clip: state(&play, f.owner).clip,
            state: None,
            blend: 1.,
            parameters: Arc::new(Default::default()),
            time: 0.,
            duration: 1.,
            playing: false,
            speed: 2.
        }
    );
    let paused = play.shared_snapshot();
    advance(&mut play, 3)?;
    assert!(Arc::ptr_eq(&paused, &play.shared_snapshot()));
    assert!(play.is_animation_controlled(f.bone, schema::TRANSLATION));
    assert_eq!(play.script_diagnostics().len(), 1);
    assert!(!play.script_diagnostics()[0].failed);
    assert_eq!(
        play.shared_snapshot()
            .entities
            .iter()
            .find(|e| e.id == f.owner)
            .unwrap()
            .animator
            .as_ref()
            .unwrap()
            .speed,
        1.
    );
    for source in ["", "fn on_start(ctx) { ctx.play_animation(ctx.entity()); }"] {
        let f = fixture(source);
        let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
        advance(&mut play, 7)?;
        assert_time(&mut play, f.owner, f.bone, FIXED_DELTA * 7.);
    }
    Ok(())
}

#[test]
fn local_speed_pause_zero_and_replay_follow_command_order_and_queries_keep_the_old_sample()
-> Result<()> {
    let f = fixture(
        r#"
        fn on_start(ctx) { this.n = 0; }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            let old = ctx.animation_state(ctx.entity());
            if this.n == 1 { ctx.set_animation_speed(ctx.entity(), 2); }
            if this.n == 2 { ctx.pause_animation(ctx.entity()); }
            if this.n == 3 { ctx.set_animation_speed(ctx.entity(), 0); ctx.resume_animation(ctx.entity()); }
            if this.n == 4 { ctx.set_animation_speed(ctx.entity(), 3); }
            if this.n == 5 { ctx.play_animation(ctx.entity()); ctx.pause_animation(ctx.entity()); }
            if this.n == 6 { ctx.resume_animation(ctx.entity()); }
            if this.n == 7 { ctx.pause_animation(ctx.entity()); ctx.play_animation(ctx.entity()); }
            let after = ctx.animation_state(ctx.entity());
            if after.time != old.time || after.speed != old.speed || after.playing != old.playing { throw "early state publication"; }
        }
    "#,
    );
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    for (time, playing, speed) in [
        (2., true, 2.),
        (2., false, 2.),
        (2., true, 0.),
        (5., true, 3.),
        (0., false, 3.),
        (3., true, 3.),
        (0., true, 3.),
    ] {
        play.advance(FIXED_DELTA)?;
        assert_time(&mut play, f.owner, f.bone, time * FIXED_DELTA);
        let current = state(&play, f.owner);
        assert_eq!((current.playing, current.speed), (playing, speed));
        assert!(play.is_animation_controlled(f.bone, schema::TRANSLATION));
    }
    assert!(play.script_diagnostics().is_empty());
    Ok(())
}

#[test]
fn invalid_animation_commands_abort_scene_sound_hud_requests_and_logs_and_share_the_exact_budget()
-> Result<()> {
    for invalid in [
        "ctx.set_animation_speed(ctx.entity(), -1);",
        "ctx.play_animation(ctx.param(\"bone\"));",
    ] {
        let f = fixture(&format!(
            r#"
            fn on_start(ctx) {{ ctx.pause_animation(ctx.entity()); ctx.set_hud("原任务"); }}
            fn on_fixed_update(ctx, dt) {{
                ctx.play_animation(ctx.entity()); ctx.set_animation_speed(ctx.entity(), 2);
                ctx.translate_local(ctx.entity(), 1, 0, 0); ctx.play_sound(ctx.entity());
                ctx.set_hud("不应发布"); ctx.request_scene(ctx.param("next")); ctx.log("不应提交");
                {invalid}
            }}
        "#
        ));
        let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
        let before = play.shared_snapshot();
        let animation = state(&play, f.owner);
        let audio = play.audio_frame()?.unwrap();
        let hud = play.hud_frame().unwrap();
        play.advance(FIXED_DELTA)?;
        assert_eq!(state(&play, f.owner), animation);
        assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
        assert!(Arc::ptr_eq(
            &audio.voices,
            &play.audio_frame()?.unwrap().voices
        ));
        assert!(Arc::ptr_eq(&hud.labels, &play.hud_frame().unwrap().labels));
        assert!(play.take_scene_request().is_none());
        assert_eq!(play.script_diagnostics().len(), 1);
        assert!(play.script_diagnostics()[0].failed);
        play.advance(FIXED_DELTA)?;
        assert_eq!(play.script_diagnostics().len(), 1);
    }
    for accepted in [true, false] {
        let f = fixture(&format!(
            r#"
            fn on_start(ctx) {{ ctx.pause_animation(ctx.entity()); }}
            fn on_fixed_update(ctx, dt) {{
                ctx.translate_local(ctx.entity(), 1, 0, 0);
                for n in 0..{} {{ ctx.pause_animation(ctx.entity()); }}
                ctx.play_animation(ctx.entity()); ctx.log("committed");
            }}
        "#,
            if accepted { 254 } else { 255 }
        ));
        let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
        play.advance(FIXED_DELTA)?;
        assert_eq!(state(&play, f.owner).playing, accepted);
        assert_eq!(
            play.shared_snapshot()
                .entities
                .iter()
                .find(|e| e.id == f.owner)
                .unwrap()
                .transform
                .translation[0],
            if accepted { 1. } else { 0. }
        );
        assert_eq!(play.script_diagnostics()[0].failed, !accepted);
    }
    Ok(())
}

#[test]
fn finished_nonlooping_clips_resume_without_rewinding_and_replay_starts_a_new_zero_sample()
-> Result<()> {
    let f = fixture(
        r#"
        fn on_start(ctx) { this.n = 0; }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            if this.n == 61 { ctx.resume_animation(ctx.entity()); }
            if this.n == 62 { ctx.play_animation(ctx.entity()); }
        }
    "#,
    );
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    advance(&mut play, 60)?;
    assert_time(&mut play, f.owner, f.bone, 1.);
    assert!(!state(&play, f.owner).playing);
    let finished = play.shared_snapshot();
    play.advance(FIXED_DELTA)?;
    assert!(Arc::ptr_eq(&finished, &play.shared_snapshot()));
    assert!(!state(&play, f.owner).playing);
    play.advance(FIXED_DELTA)?;
    assert_time(&mut play, f.owner, f.bone, 0.);
    assert!(state(&play, f.owner).playing);
    play.advance(FIXED_DELTA)?;
    assert_time(&mut play, f.owner, f.bone, FIXED_DELTA);
    assert!(play.script_diagnostics().is_empty());
    Ok(())
}

#[test]
fn paused_worlds_step_controls_once_and_restart_and_candidate_acceptance_reset_runtime_overrides()
-> Result<()> {
    let source = r#"
        fn on_start(ctx) { this.n = 0; ctx.set_animation_speed(ctx.entity(), 2); ctx.pause_animation(ctx.entity()); }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            if this.n == 1 { ctx.play_animation(ctx.entity()); }
            if this.n == 2 { ctx.pause_animation(ctx.entity()); }
            if this.n == 3 { ctx.resume_animation(ctx.entity()); }
        }
    "#;
    let f = fixture(source);
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    play.set_paused(true);
    let initial = state(&play, f.owner);
    play.advance(1.)?;
    assert_eq!(state(&play, f.owner), initial);
    play.single_step()?;
    assert_eq!(state(&play, f.owner).time, 0.);
    assert!(state(&play, f.owner).playing && play.is_paused());
    play.single_step()?;
    assert!(!state(&play, f.owner).playing);
    play.single_step()?;
    assert_time(&mut play, f.owner, f.bone, FIXED_DELTA * 2.);
    play.set_suspended(true);
    let frozen = state(&play, f.owner);
    play.advance(1.)?;
    assert_eq!(state(&play, f.owner), frozen);
    assert!(play.single_step().is_err());
    let world = play.world_id();
    play.restart()?;
    assert_ne!(play.world_id(), world);
    assert_eq!(state(&play, f.owner), initial);
    let next = fixture("");
    let candidate = PlaySession::start_with_resources(next.scene, next.resources)?;
    play.accept_candidate(candidate)?;
    assert!(play.animation_state(f.owner).is_none());
    assert_eq!(
        state(&play, next.owner),
        AnimationState {
            clip: state(&play, next.owner).clip,
            state: None,
            blend: 1.,
            parameters: Arc::new(Default::default()),
            time: 0.,
            duration: 1.,
            playing: true,
            speed: 1.
        }
    );
    assert!(play.is_paused() && play.is_suspended());
    play.stop();
    assert!(play.animation_state(next.owner).is_none());
    assert!(!play.is_animation_controlled(next.bone, schema::TRANSLATION));
    Ok(())
}

#[test]
fn trigger_queries_observe_the_completed_sample_and_queued_replay_waits_for_the_next_phase()
-> Result<()> {
    use comet_scene::{BodyMotion, PhysicsBody};
    let mut f = fixture("");
    let mut sensor = EntityData::new("sensor");
    sensor.transform.translation = [0., 2., 0.];
    sensor.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        sensor: true,
        ..Default::default()
    });
    let sensor_id = sensor.id;
    let mut other = EntityData::new("entering body");
    other.transform.translation = sensor.transform.translation;
    other.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    f.scene.entities.extend([sensor, other]);
    bind(
        &mut f,
        sensor_id,
        r#"
        fn on_start(ctx) { this.n = 0; this.entered = false; ctx.pause_animation(ctx.param("animation")); }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            if this.n == 1 {
                let old = ctx.animation_state(ctx.param("animation"));
                if old.playing || old.time != 0.0 { throw "fixed snapshot"; }
                ctx.resume_animation(ctx.param("animation"));
            }
        }
        fn on_trigger(ctx, other, entered) {
            if entered && !this.entered {
                this.entered = true;
                let old = ctx.animation_state(ctx.param("animation"));
                if !old.playing || old.time <= 0.0 || old.speed != 1.0 { throw "trigger must read post-sample state"; }
                ctx.play_animation(ctx.param("animation")); ctx.set_animation_speed(ctx.param("animation"), 2);
                let after = ctx.animation_state(ctx.param("animation"));
                if after.time != old.time || after.speed != old.speed { throw "trigger intent leaked"; }
                ctx.log("trigger snapshot");
            }
        }
    "#,
    );
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    play.advance(FIXED_DELTA)?;
    assert_time(&mut play, f.owner, f.bone, FIXED_DELTA);
    assert_eq!(state(&play, f.owner).speed, 1.);
    assert_eq!(play.script_diagnostics().len(), 1);
    assert_eq!(play.script_diagnostics()[0].phase, "on_trigger");
    assert!(!play.script_diagnostics()[0].failed);
    play.advance(FIXED_DELTA)?;
    assert_time(&mut play, f.owner, f.bone, 0.);
    assert_eq!(state(&play, f.owner).speed, 2.);
    advance(&mut play, 5)?;
    assert_time(&mut play, f.owner, f.bone, FIXED_DELTA * 10.);
    Ok(())
}

#[test]
fn stopped_callbacks_only_query_the_last_sample_and_missing_clips_reject_controls() -> Result<()> {
    for command in [
        "ctx.play_animation(ctx.entity());",
        "ctx.pause_animation(ctx.entity());",
        "ctx.resume_animation(ctx.entity());",
        "ctx.set_animation_speed(ctx.entity(), 2);",
    ] {
        let f = fixture(&format!(
            r#"
            fn on_stop(ctx) {{
                let old = ctx.animation_state(ctx.entity());
                if old.time <= 0.0 {{ throw "last sample missing"; }}
                ctx.log("discarded stop log"); {command}
            }}
        "#
        ));
        let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
        advance(&mut play, 3)?;
        play.stop();
        assert_eq!(play.script_diagnostics().len(), 1);
        assert!(play.script_diagnostics()[0].failed);
        assert!(
            play.script_diagnostics()[0]
                .message
                .contains("unavailable during on_stop")
        );
    }
    let mut f = fixture(
        "fn on_start(ctx) { ctx.translate_local(ctx.entity(), 1, 0, 0); ctx.play_animation(ctx.entity()); }",
    );
    f.scene.entities[0].animator.as_mut().unwrap().clip = None;
    f.resources.animations.clear();
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    assert!(play.animation_state(f.owner).is_none());
    assert_eq!(
        play.shared_snapshot()
            .entities
            .iter()
            .find(|e| e.id == f.owner)
            .unwrap()
            .transform
            .translation,
        [0.; 3]
    );
    assert!(
        play.script_diagnostics()[0]
            .message
            .contains("no selected animation")
    );
    Ok(())
}
