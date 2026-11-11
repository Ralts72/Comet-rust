use anyhow::{Context, Result};
use comet_asset_data::AudioData;
use comet_core::{Action, ActionEvent, ActionInput, AssetId};
use comet_runtime::{FIXED_DELTA, PlaySession, SceneResources, ScriptLibrary};
use comet_scene::{ContentSource, SceneSnapshot};
use std::{collections::BTreeMap, sync::Arc};

// WAV decoding and actual mixing have separate regressions; Runtime consumes ready shared PCM.
fn ready_pcm(author: &SceneSnapshot) -> BTreeMap<AssetId, Arc<AudioData>> {
    author
        .entities
        .iter()
        .filter_map(|entity| entity.audio_source.as_ref())
        .map(|source| {
            (
                source.clip,
                Arc::new(AudioData {
                    version: 1,
                    sample_rate: 48_000,
                    channels: 1,
                    samples: vec![0, 1200, -1200, 0].into(),
                }),
            )
        })
        .collect()
}

#[test]
fn actual_audio_demo_script_publishes_scoped_intents_and_keeps_authoring_unchanged() -> Result<()> {
    let author = ContentSource::from_json(include_bytes!("../../../demo/assets/audio.scene"))?.base;
    let (cue, script) = author
        .entities
        .iter()
        .find_map(|entity| entity.script.as_ref().map(|script| (entity.id, script)))
        .context("audio demo script")?;
    let background = author
        .entities
        .iter()
        .find(|entity| {
            entity
                .audio_source
                .as_ref()
                .is_some_and(|source| source.autoplay && source.looping)
        })
        .context("background source")?
        .id;
    let audio = ready_pcm(&author);
    let resources = SceneResources {
        scripts: ScriptLibrary::from([(
            script.asset,
            include_str!("../../../demo/assets/scripts/audio_demo.rhai").into(),
        )]),
        audio,
        ..Default::default()
    };
    let mut play = PlaySession::start_with_resources(author.clone(), resources)?;
    let initial = play.audio_frame()?.context("initial audio intent")?;
    assert_eq!(initial.voices.len(), 2);
    assert!(
        initial
            .voices
            .iter()
            .all(|voice| voice.generation == 1 && voice.playing)
    );
    assert_eq!(
        initial
            .voices
            .iter()
            .find(|voice| voice.entity == background)
            .unwrap()
            .gain,
        0.18
    );
    let mut input = ActionInput {
        enabled: true,
        ..Default::default()
    };
    for _ in 0..8 {
        play.advance_with_input(FIXED_DELTA, input.clone())?;
    }
    assert!(Arc::ptr_eq(
        &initial.voices,
        &play.audio_frame()?.unwrap().voices
    ));
    input.held.set(Action::Interact, true);
    input.events.push(ActionEvent {
        action: Action::Interact,
        pressed: true,
    });
    play.advance_with_input(FIXED_DELTA, input.clone())?;
    input.events.clear();
    for _ in 0..8 {
        play.advance_with_input(FIXED_DELTA, input.clone())?;
    }
    let replay = play.audio_frame()?.unwrap();
    assert_eq!(
        replay
            .voices
            .iter()
            .find(|voice| voice.entity == cue)
            .unwrap()
            .generation,
        2
    );
    assert_eq!(
        replay
            .voices
            .iter()
            .find(|voice| voice.entity == background)
            .unwrap()
            .generation,
        1
    );
    input.events.push(ActionEvent {
        action: Action::Jump,
        pressed: true,
    });
    play.advance_with_input(FIXED_DELTA, input)?;
    let stopped = play.audio_frame()?.unwrap();
    assert!(
        !stopped
            .voices
            .iter()
            .find(|voice| voice.entity == cue)
            .unwrap()
            .playing
    );
    assert!(
        stopped
            .voices
            .iter()
            .find(|voice| voice.entity == background)
            .unwrap()
            .playing
    );
    play.set_paused(true);
    play.play_sound(cue)?;
    play.set_sound_volume(cue, 0.4)?;
    let pending = play.audio_frame()?.unwrap();
    assert!(pending.paused);
    let voice = pending
        .voices
        .iter()
        .find(|voice| voice.entity == cue)
        .unwrap();
    assert!(voice.playing);
    assert_eq!(voice.generation, 3);
    assert_eq!(voice.gain, 0.4);
    let steps = play.steps();
    play.single_step()?;
    assert_eq!(play.steps(), steps + 1);
    assert!(play.audio_frame()?.unwrap().paused);
    assert_eq!(play.snapshot(), author);
    let world = play.world_id();
    play.restart()?;
    let restarted = play.audio_frame()?.unwrap();
    assert_ne!(restarted.world, world);
    assert!(restarted.paused);
    assert!(restarted.voices.iter().all(|voice| voice.generation == 1));
    assert_eq!(
        restarted
            .voices
            .iter()
            .find(|voice| voice.entity == cue)
            .unwrap()
            .gain,
        0.65
    );
    assert_eq!(play.snapshot(), author);
    play.stop();
    assert!(play.audio_frame()?.is_none());
    assert!(play.script_diagnostics().iter().all(|entry| !entry.failed));
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|entry| entry.message == "已重播提示音。")
            .count(),
        1
    );
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|entry| entry.message == "已停止提示音。")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn actual_spatial_audio_demo_moves_without_replaying_and_toggles_only_on_fresh_input() -> Result<()>
{
    let author =
        ContentSource::from_json(include_bytes!("../../../demo/assets/spatial_audio.scene"))?.base;
    let authored_bytes = author.to_json()?;
    let emitter = author
        .entities
        .iter()
        .find(|entity| entity.audio_source.is_some())
        .context("spatial emitter")?;
    assert_eq!(
        emitter.id.to_string(),
        "b3000000-0000-4000-8000-000000000001"
    );
    let id = emitter.id;
    let initial_position = emitter.transform.translation;
    let script = emitter.script.as_ref().context("spatial demo script")?;
    let mut play = PlaySession::start_with_resources(
        author.clone(),
        SceneResources {
            scripts: ScriptLibrary::from([(
                script.asset,
                include_str!("../../../demo/assets/scripts/spatial_audio.rhai").into(),
            )]),
            audio: ready_pcm(&author),
            ..Default::default()
        },
    )?;
    let initial = play.audio_frame()?.context("spatial audio frame")?;
    assert_eq!(initial.voices.len(), 1);
    assert!(initial.listener.is_none());
    assert_eq!(initial.voices[0].entity, id);
    assert_eq!(initial.voices[0].gain, 0.9);
    assert!(initial.voices[0].playing && initial.voices[0].looping);
    assert_eq!(initial.voices[0].generation, 1);
    let profile = initial.voices[0].spatial.context("spatial profile")?;
    assert_eq!(profile.position, initial_position);
    assert_eq!((profile.min_distance, profile.max_distance), (1., 30.));
    let position = |play: &mut PlaySession| {
        play.shared_snapshot()
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .transform
            .translation
    };
    let mut input = ActionInput {
        enabled: true,
        ..Default::default()
    };
    for tick in 1..=60 {
        play.advance_with_input(FIXED_DELTA, input.clone())?;
        let frame = play.audio_frame()?.unwrap();
        let voice = &frame.voices[0];
        let point = voice.spatial.unwrap().position;
        assert_eq!(point, position(&mut play));
        let angle = std::f64::consts::PI + f64::from(tick) * FIXED_DELTA * 0.6;
        let expected = [(3. * angle.cos()) as f32, 0.7, (3. * angle.sin()) as f32];
        assert!(
            point
                .into_iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 1e-5)
        );
        assert!(voice.playing);
        assert_eq!(
            voice.generation, 1,
            "moving the emitter cannot restart its clip"
        );
        assert!(Arc::ptr_eq(&initial.voices[0].data, &voice.data));
        assert!(
            Arc::ptr_eq(&frame.voices, &play.audio_frame()?.unwrap().voices),
            "an unchanged getter reuses the resolved positions batch"
        );
        assert!(frame.listener.is_none());
    }
    assert!(
        position(&mut play)[2] < -1.5,
        "the actual script must advance around its orbit"
    );
    input.held.set(Action::Interact, true);
    input.events.push(ActionEvent {
        action: Action::Interact,
        pressed: true,
    });
    play.advance_with_input(FIXED_DELTA, input.clone())?;
    input.events.clear();
    for _ in 0..5 {
        play.advance_with_input(FIXED_DELTA, input.clone())?;
    }
    let stopped = play.audio_frame()?.unwrap();
    assert!(!stopped.voices[0].playing);
    assert_eq!(stopped.voices[0].generation, 1);
    input.held.set(Action::Interact, false);
    input.events.push(ActionEvent {
        action: Action::Interact,
        pressed: false,
    });
    play.advance_with_input(FIXED_DELTA, input.clone())?;
    input.held.set(Action::Interact, true);
    input.events = vec![ActionEvent {
        action: Action::Interact,
        pressed: true,
    }];
    play.advance_with_input(FIXED_DELTA, input.clone())?;
    input.events.clear();
    for _ in 0..5 {
        play.advance_with_input(FIXED_DELTA, input.clone())?;
    }
    let replay = play.audio_frame()?.unwrap();
    assert!(replay.voices[0].playing);
    assert_eq!(replay.voices[0].generation, 2);
    play.set_paused(true);
    let input = ActionInput {
        enabled: true,
        ..Default::default()
    };
    play.advance_with_input(FIXED_DELTA * 5., input.clone())?;
    let paused = play.audio_frame()?.unwrap();
    assert!(paused.paused);
    assert!(Arc::ptr_eq(&replay.voices, &paused.voices));
    let steps = play.steps();
    play.single_step()?;
    let stepped = play.audio_frame()?.unwrap();
    assert!(stepped.paused);
    assert_eq!(play.steps(), steps + 1);
    assert_ne!(
        stepped.voices[0].spatial.unwrap().position,
        paused.voices[0].spatial.unwrap().position
    );
    assert_eq!(stepped.voices[0].generation, 2);
    let old_world = play.world_id();
    let mut restart = input;
    restart.events = vec![
        ActionEvent {
            action: Action::Restart,
            pressed: true,
        },
        ActionEvent {
            action: Action::Restart,
            pressed: false,
        },
    ];
    play.advance_with_input(0., restart)?;
    let restarted = play.audio_frame()?.unwrap();
    assert_ne!(restarted.world, old_world);
    assert!(restarted.paused);
    assert_eq!(play.steps(), 0);
    assert_eq!(play.snapshot(), author);
    assert_eq!(
        restarted.voices[0].spatial.unwrap().position,
        initial_position
    );
    assert!(restarted.voices[0].playing);
    assert_eq!(restarted.voices[0].generation, 1);
    assert!(Arc::ptr_eq(
        &initial.voices[0].data,
        &restarted.voices[0].data
    ));
    assert_eq!(author.to_json()?, authored_bytes);
    assert!(play.script_diagnostics().iter().all(|entry| !entry.failed));
    for message in ["已关闭空间声源。", "已开启空间声源。"] {
        assert_eq!(
            play.script_diagnostics()
                .iter()
                .filter(|entry| entry.message == message)
                .count(),
            1
        );
    }
    play.stop();
    assert!(play.audio_frame()?.is_none());
    Ok(())
}
