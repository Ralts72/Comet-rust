use anyhow::Result;
use comet_asset_data::{
    AudioData,
    audio::{AUDIO_VERSION, MAX_AUDIO_SAMPLES},
};
use comet_audio::{
    AudioFrame, AudioLibrary, AudioVoice, MAX_TOTAL_SAMPLES, MAX_VOICES, validate_library,
};
use comet_core::{AssetId, EntityId, WorldId};
use std::sync::Arc;

fn audio(samples: usize) -> Arc<AudioData> {
    Arc::new(AudioData {
        version: AUDIO_VERSION,
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0; samples].into(),
    })
}
fn voice(data: Arc<AudioData>) -> AudioVoice {
    AudioVoice {
        entity: EntityId::new(),
        generation: 1,
        clip: AssetId::new(),
        data,
        playing: true,
        looping: false,
        gain: 1.,
        spatial: None,
    }
}
fn frame(voices: Vec<AudioVoice>) -> AudioFrame {
    AudioFrame {
        world: WorldId::new(),
        paused: false,
        listener: None,
        voices: voices.into(),
    }
}

#[test]
fn source_budget_counts_unique_clips_once_and_limits_aggregate_pcm() -> Result<()> {
    let shared = audio(MAX_AUDIO_SAMPLES);
    let original = voice(shared.clone());
    let voices = (0..MAX_VOICES)
        .map(|_| AudioVoice {
            entity: EntityId::new(),
            ..original.clone()
        })
        .collect();
    frame(voices).validate()?;
    let mut library =
        AudioLibrary::from([(original.clip, shared.clone()), (AssetId::new(), shared)]);
    assert_eq!(MAX_TOTAL_SAMPLES, MAX_AUDIO_SAMPLES * 2);
    validate_library(&library)?;
    library.insert(AssetId::new(), audio(1));
    assert!(validate_library(&library).is_err());
    let too_many = (0..=MAX_VOICES).map(|_| voice(audio(1))).collect();
    assert!(frame(too_many).validate().is_err());
    Ok(())
}

#[test]
fn conflicting_clip_versions_bad_voice_intents_and_invalid_pcm_are_rejected() -> Result<()> {
    let original = voice(audio(8));
    frame(vec![original.clone()]).validate()?;
    assert!(
        frame(vec![original.clone(), original.clone()])
            .validate()
            .is_err()
    );
    let changed = AudioVoice {
        entity: EntityId::new(),
        data: audio(8),
        ..original.clone()
    };
    assert!(frame(vec![original.clone(), changed]).validate().is_err());
    for gain in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(
            frame(vec![AudioVoice {
                gain,
                ..original.clone()
            }])
            .validate()
            .is_err()
        );
    }
    assert!(
        frame(vec![AudioVoice {
            generation: 0,
            ..original.clone()
        }])
        .validate()
        .is_err()
    );
    frame(vec![AudioVoice {
        generation: 0,
        playing: false,
        ..original.clone()
    }])
    .validate()?;
    assert!(
        frame(vec![AudioVoice {
            data: audio(0),
            ..original
        }])
        .validate()
        .is_err()
    );
    Ok(())
}
