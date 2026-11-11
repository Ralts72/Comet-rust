use anyhow::Result;
use comet_asset_data::AudioData;
use comet_audio::{AudioFrame, AudioLibrary, AudioVoice};
use comet_core::{AssetId, EntityId};
use comet_runtime::{PlaySession, SceneResources, validate_audio};
use comet_scene::{
    AudioSource, EntityData, SceneSnapshot, Script,
    schema::{self, FieldValue},
};
use std::sync::Arc;

#[test]
fn spatial_frames_lazily_follow_mirrored_nonuniform_parent_world_origins_without_decomposing_shear()
-> Result<()> {
    use comet_core::math::{Quat, Vec3};
    let (mut snapshot, resources, id) = fixture(false);
    let mut parent = EntityData::new("parent");
    let parent_id = parent.id;
    parent.transform.translation = [3., 4., 5.];
    parent.transform.scale = [-2., 3., 4.];
    parent.transform.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
    let child = &mut snapshot.entities[0];
    child.parent = Some(parent.id);
    child.transform.translation = [1., 2., 3.];
    child.transform.rotation = Quat::from_rotation_y(0.4).to_array();
    child.transform.scale = [2., 1., 3.];
    child.audio_source.as_mut().unwrap().spatial = true;
    snapshot.entities.push(parent);
    let mut play = PlaySession::start_with_resources(snapshot, resources)?;
    let initial = play.audio_frame()?.unwrap();
    assert!(
        initial.listener.is_none(),
        "the Runtime does not own a camera listener"
    );
    assert!(
        Vec3::from_array(voice(&initial, id).spatial.unwrap().position)
            .distance(Vec3::new(-3., 2., 17.))
            < 1e-5
    );
    assert!(Arc::ptr_eq(
        &initial.voices,
        &play.audio_frame()?.unwrap().voices
    ));
    play.play_sound(id)?;
    play.set_paused(true);
    play.set_field(
        &[parent_id],
        schema::TRANSLATION,
        FieldValue::Vec3([4., 5., 6.]),
    )?;
    let moved = play.audio_frame()?.unwrap();
    assert!(moved.paused);
    assert!(
        Vec3::from_array(voice(&moved, id).spatial.unwrap().position)
            .distance(Vec3::new(-2., 3., 18.))
            < 1e-5
    );
    assert_eq!(voice(&moved, id).generation, 1);
    assert!(Arc::ptr_eq(
        &voice(&initial, id).data,
        &voice(&moved, id).data
    ));
    assert!(
        Vec3::from_array(voice(&initial, id).spatial.unwrap().position)
            .distance(Vec3::new(-3., 2., 17.))
            < 1e-5
    );
    play.set_field(
        &[id],
        schema::ROTATION,
        FieldValue::Quat(Quat::from_rotation_y(0.8).to_array()),
    )?;
    assert!(
        Arc::ptr_eq(&moved.voices, &play.audio_frame()?.unwrap().voices),
        "a changed scene revision with unchanged sound positions keeps the batch"
    );
    assert!(
        play.set_field(&[id], schema::AUDIO_SPATIAL, FieldValue::Bool(false))
            .is_err()
    );
    assert!(
        play.set_field(&[parent_id], schema::SCALE, FieldValue::Vec3([f32::MAX; 3]))
            .is_err()
    );
    assert!(Arc::ptr_eq(
        &moved.voices,
        &play.audio_frame()?.unwrap().voices
    ));
    play.restart()?;
    let restarted = play.audio_frame()?.unwrap();
    assert_ne!(restarted.world, moved.world);
    assert!(restarted.paused);
    assert!(
        Vec3::from_array(voice(&restarted, id).spatial.unwrap().position)
            .distance(Vec3::new(-3., 2., 17.))
            < 1e-5
    );
    assert_eq!(voice(&restarted, id).generation, 0);
    assert!(Arc::ptr_eq(
        &voice(&initial, id).data,
        &voice(&restarted, id).data
    ));
    Ok(())
}

#[test]
fn spatial_getter_observes_start_fixed_trigger_and_public_teleport_writes_even_while_paused()
-> Result<()> {
    use comet_runtime::FIXED_DELTA;
    use comet_scene::{BodyMotion, EntityRef, PhysicsBody, PoseUpdate, ScriptValue};
    let (mut snapshot, mut resources, id) = fixture(false);
    let mut body = EntityData::new("kinematic parent");
    let body_id = body.id;
    body.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        ..Default::default()
    });
    body.transform.translation = [1., 2., 0.];
    snapshot.entities[0].parent = Some(body_id);
    snapshot.entities[0].transform.translation = [1., 0., 0.];
    snapshot.entities[0].audio_source.as_mut().unwrap().spatial = true;
    let mut sensor = EntityData::new("trigger");
    sensor.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        sensor: true,
        ..Default::default()
    });
    let mut other = EntityData::new("overlapping body");
    other.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    for (entity, text) in [
        (
            &mut snapshot.entities[0],
            r#"
        fn on_start(ctx) { ctx.teleport(ctx.param("body"), 2, 2, 0); }
        fn on_fixed_update(ctx, dt) { ctx.teleport(ctx.param("body"), 3, 3, 0); }
    "#,
        ),
        (
            &mut sensor,
            r#"fn on_trigger(ctx, other, entered) { if entered { ctx.teleport(ctx.param("body"), 4, 4, 0); ctx.log("moved by trigger"); } }"#,
        ),
    ] {
        let asset = AssetId::new();
        let mut script = Script::new(asset);
        script.parameters.insert(
            "body".into(),
            ScriptValue::Entity(Some(EntityRef::local(body_id))),
        );
        entity.script = Some(script);
        resources.scripts.insert(asset, text.into());
    }
    snapshot.entities.extend([body, sensor, other]);
    let mut play = PlaySession::start_with_resources(snapshot, resources)?;
    assert_eq!(
        voice(&play.audio_frame()?.unwrap(), id)
            .spatial
            .unwrap()
            .position,
        [3., 2., 0.]
    );
    play.advance(FIXED_DELTA)?;
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|entry| !entry.failed && entry.message == "moved by trigger")
    );
    assert_eq!(
        voice(&play.audio_frame()?.unwrap(), id)
            .spatial
            .unwrap()
            .position,
        [5., 4., 0.]
    );
    play.set_paused(true);
    play.teleport(PoseUpdate {
        entity: body_id,
        translation: [6., 6., 0.],
        rotation: [0., 0., 0., 1.],
    })?;
    let paused = play.audio_frame()?.unwrap();
    assert!(paused.paused);
    assert_eq!(voice(&paused, id).spatial.unwrap().position, [7., 6., 0.]);
    play.single_step()?;
    let stepped = play.audio_frame()?.unwrap();
    assert!(stepped.paused);
    assert_eq!(voice(&stepped, id).spatial.unwrap().position, [4., 3., 0.]);
    play.set_suspended(true);
    assert!(Arc::ptr_eq(
        &stepped.voices,
        &play.audio_frame()?.unwrap().voices
    ));
    assert!(play.play_sound(EntityId::new()).is_err());
    Ok(())
}

fn pcm(samples: usize) -> Arc<AudioData> {
    Arc::new(AudioData {
        version: 1,
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0; samples].into(),
    })
}
fn fixture(autoplay: bool) -> (SceneSnapshot, SceneResources, EntityId) {
    let clip = AssetId::new();
    let mut entity = EntityData::new("sound");
    entity.audio_source = Some(AudioSource {
        clip,
        gain: 0.7,
        looping: true,
        autoplay,
        ..AudioSource::new(clip)
    });
    let id = entity.id;
    (
        SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        SceneResources {
            audio: AudioLibrary::from([(clip, pcm(16))]),
            ..Default::default()
        },
        id,
    )
}
fn voice(frame: &AudioFrame, id: EntityId) -> &AudioVoice {
    frame
        .voices
        .iter()
        .find(|voice| voice.entity == id)
        .unwrap()
}

#[test]
fn voice_intents_share_immutable_pcm_and_only_changed_content_rebuilds_the_batch() -> Result<()> {
    assert!(
        PlaySession::start(SceneSnapshot::default())?
            .audio_frame()?
            .is_none()
    );
    let (mut snapshot, resources, id) = fixture(false);
    let mut second = snapshot.entities[0].clone();
    second.id = EntityId::new();
    second.audio_source.as_mut().unwrap().autoplay = true;
    let other = second.id;
    snapshot.entities.push(second);
    let author = snapshot.clone();
    let mut play = PlaySession::start_with_resources(snapshot, resources)?;
    let initial = play.audio_frame()?.unwrap();
    initial.validate()?;
    assert_eq!(voice(&initial, id).generation, 0);
    assert!(!voice(&initial, id).playing);
    assert_eq!(voice(&initial, other).generation, 1);
    assert!(voice(&initial, other).playing);
    assert!(Arc::ptr_eq(
        &voice(&initial, id).data,
        &voice(&initial, other).data
    ));
    play.stop_sound(id)?;
    play.set_sound_volume(id, 0.7)?;
    play.advance(0.)?;
    assert!(Arc::ptr_eq(
        &initial.voices,
        &play.audio_frame()?.unwrap().voices
    ));
    play.play_sound(id)?;
    play.play_sound(id)?;
    play.set_sound_volume(id, 0.25)?;
    play.stop_sound(id)?;
    let changed = play.audio_frame()?.unwrap();
    assert_eq!(voice(&changed, id).generation, 2);
    assert!(!voice(&changed, id).playing);
    assert_eq!(voice(&changed, id).gain, 0.25);
    assert_eq!(voice(&initial, id).generation, 0);
    assert_eq!(voice(&initial, id).gain, 0.7);
    assert!(Arc::ptr_eq(
        &voice(&initial, id).data,
        &voice(&changed, id).data
    ));
    for gain in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        assert!(play.set_sound_volume(id, gain).is_err());
    }
    assert!(play.play_sound(EntityId::new()).is_err());
    assert!(
        play.set_field(&[id], schema::AUDIO_GAIN, FieldValue::Scalar(0.1))
            .is_err()
    );
    assert!(Arc::ptr_eq(
        &changed.voices,
        &play.audio_frame()?.unwrap().voices
    ));
    let mut current = play.snapshot();
    current.canonicalize();
    let mut author = author;
    author.canonicalize();
    assert_eq!(
        current, author,
        "runtime sound intents never edit author components"
    );
    play.set_paused(true);
    assert!(play.audio_frame()?.unwrap().paused);
    play.restart()?;
    let restarted = play.audio_frame()?.unwrap();
    assert_ne!(initial.world, restarted.world);
    assert!(restarted.paused);
    assert_eq!(voice(&restarted, id).generation, 0);
    assert_eq!(voice(&restarted, id).gain, 0.7);
    assert!(Arc::ptr_eq(
        &voice(&initial, id).data,
        &voice(&restarted, id).data
    ));
    play.stop();
    assert!(play.audio_frame()?.is_none());
    assert!(play.play_sound(id).is_err());
    assert!(play.stop_sound(id).is_err());
    assert!(play.set_sound_volume(id, 0.).is_err());
    Ok(())
}

#[test]
fn initialized_candidates_single_steps_and_world_acceptance_only_publish_paused_memory_intents()
-> Result<()> {
    let (mut snapshot, mut resources, id) = fixture(false);
    let script = AssetId::new();
    snapshot.entities[0].script = Some(Script::new(script));
    resources.scripts.insert(
        script,
        r#"
        fn on_start(ctx) { ctx.play_sound(ctx.entity()); ctx.set_sound_volume(ctx.entity(), 0.3); }
        fn on_fixed_update(ctx, dt) { ctx.play_sound(ctx.entity()); }
        fn on_stop(ctx) { ctx.play_sound(ctx.entity()); }
    "#
        .into(),
    );
    let mut play = PlaySession::start_with_resources(snapshot, resources)?;
    let first = play.audio_frame()?.unwrap();
    assert_eq!(voice(&first, id).generation, 1);
    assert_eq!(voice(&first, id).gain, 0.3);
    play.set_paused(true);
    play.single_step()?;
    let stepped = play.audio_frame()?.unwrap();
    assert!(stepped.paused);
    assert_eq!(voice(&stepped, id).generation, 2);
    play.set_suspended(true);
    assert!(play.single_step().is_err());
    let (snapshot, resources, new_id) = fixture(true);
    let mut candidate = PlaySession::start_with_resources(snapshot, resources)?;
    let candidate_frame = candidate.audio_frame()?.unwrap();
    play.accept_candidate(candidate)?;
    let accepted = play.audio_frame()?.unwrap();
    assert_eq!(accepted.world, candidate_frame.world);
    assert_ne!(accepted.world, first.world);
    assert!(accepted.paused);
    assert!(Arc::ptr_eq(&accepted.voices, &candidate_frame.voices));
    assert_eq!(accepted.voices.len(), 1);
    assert_eq!(voice(&accepted, new_id).generation, 1);
    assert!(play.play_sound(id).is_err());
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|entry| entry.phase == "on_stop"
                && entry.failed
                && entry.message.contains("unavailable during on_stop"))
    );
    play.set_suspended(false);
    assert!(play.audio_frame()?.unwrap().paused);
    play.set_paused(false);
    assert!(!play.audio_frame()?.unwrap().paused);
    assert_eq!(
        voice(&stepped, id).generation,
        2,
        "the old world's submitted batch remains immutable"
    );
    Ok(())
}

#[test]
fn audio_preflight_rejects_missing_invalid_and_overbudget_resources_and_counts_unique_references()
-> Result<()> {
    let (mut snapshot, mut resources, _) = fixture(false);
    let clip = snapshot.entities[0].audio_source.as_ref().unwrap().clip;
    assert!(validate_audio(&snapshot, &AudioLibrary::new()).is_err());
    let mut invalid = (*pcm(1)).clone();
    invalid.sample_rate = 0;
    resources.audio.insert(clip, Arc::new(invalid));
    assert!(validate_audio(&snapshot, &resources.audio).is_err());
    resources
        .audio
        .insert(clip, pcm(comet_asset_data::audio::MAX_AUDIO_SAMPLES));
    let mut duplicate = snapshot.entities[0].clone();
    duplicate.id = EntityId::new();
    snapshot.entities.push(duplicate);
    validate_audio(&snapshot, &resources.audio)?;
    let extra = AssetId::new();
    resources
        .audio
        .insert(extra, resources.audio[&clip].clone());
    snapshot.entities[1].audio_source.as_mut().unwrap().clip = extra;
    validate_audio(&snapshot, &resources.audio)?;
    let third = AssetId::new();
    resources.audio.insert(third, pcm(1));
    let mut third_source = snapshot.entities[0].clone();
    third_source.id = EntityId::new();
    third_source.audio_source.as_mut().unwrap().clip = third;
    snapshot.entities.push(third_source);
    assert!(validate_audio(&snapshot, &resources.audio).is_err());
    snapshot.entities.pop();
    // Unreferenced library content belongs to other scenes and does not consume this world's budget.
    validate_audio(&snapshot, &resources.audio)?;
    let source = snapshot.entities[0].clone();
    while snapshot.entities.len() <= comet_audio::MAX_VOICES {
        let mut entity = source.clone();
        entity.id = EntityId::new();
        snapshot.entities.push(entity);
    }
    assert!(validate_audio(&snapshot, &resources.audio).is_err());
    assert!(PlaySession::start_with_resources(snapshot, resources).is_err());
    Ok(())
}

#[test]
fn rhai_sound_commands_share_the_existing_budget_and_stop_callbacks_cannot_publish_them()
-> Result<()> {
    for accepted in [true, false] {
        let (mut snapshot, mut resources, id) = fixture(false);
        let script = AssetId::new();
        snapshot.entities[0].script = Some(Script::new(script));
        resources.scripts.insert(
            script,
            format!(
                r#"
            fn on_start(ctx) {{
                ctx.translate_local(ctx.entity(), 1, 0, 0);
                for n in 0..{} {{ ctx.play_sound(ctx.entity()); }}
                ctx.log("committed sound");
            }}
        "#,
                if accepted { 255 } else { 256 }
            ),
        );
        let mut play = PlaySession::start_with_resources(snapshot, resources)?;
        let frame = play.audio_frame()?.unwrap();
        assert_eq!(voice(&frame, id).generation, if accepted { 255 } else { 0 });
        assert_eq!(
            play.shared_snapshot().entities[0].transform.translation[0],
            if accepted { 1. } else { 0. }
        );
        assert_eq!(play.script_diagnostics().len(), 1);
        let diagnostic = &play.script_diagnostics()[0];
        assert_eq!(diagnostic.failed, !accepted);
        assert!(diagnostic.message.contains(if accepted {
            "committed sound"
        } else {
            "command budget exceeded"
        }));
    }
    for command in [
        "ctx.play_sound(ctx.entity());",
        "ctx.stop_sound(ctx.entity());",
        "ctx.set_sound_volume(ctx.entity(), 0.1);",
    ] {
        let (mut snapshot, mut resources, id) = fixture(true);
        let script = AssetId::new();
        snapshot.entities[0].script = Some(Script::new(script));
        resources.scripts.insert(
            script,
            format!("fn on_stop(ctx) {{ ctx.log(\"uncommitted stop\"); {command} }}"),
        );
        let mut play = PlaySession::start_with_resources(snapshot, resources)?;
        let frame = play.audio_frame()?.unwrap();
        play.stop();
        assert!(play.audio_frame()?.is_none());
        assert_eq!(voice(&frame, id).generation, 1);
        assert!(voice(&frame, id).playing);
        assert_eq!(play.script_diagnostics().len(), 1);
        assert!(play.script_diagnostics()[0].failed);
        assert!(
            play.script_diagnostics()[0]
                .message
                .contains("unavailable during on_stop")
        );
    }
    Ok(())
}
