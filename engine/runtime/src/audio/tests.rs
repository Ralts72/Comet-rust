use super::*;
use comet_asset_data::AudioData;
use comet_core::AssetId;
use comet_scene::{AudioSource, EntityData};

#[test]
fn failed_spatial_candidates_do_not_publish_positions_or_revision_and_2d_never_resolves_hierarchies()
-> Result<()> {
    let clip = AssetId::new();
    let mut entity = EntityData::new("spatial");
    entity.transform.translation = [2., 3., 4.];
    entity.audio_source = Some(AudioSource {
        spatial: true,
        ..AudioSource::new(clip)
    });
    let mut snapshot = SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    };
    let library = [(
        clip,
        Arc::new(AudioData {
            version: 1,
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0; 8].into(),
        }),
    )]
    .into();
    let mut audio = Audio::new(&snapshot, &library, WorldId::new())?;
    assert!(
        audio.frame(false).is_err(),
        "unresolved spatial frames cannot escape as 2d or fake positions"
    );
    audio.sync_positions(&snapshot, 1)?;
    let before = audio.frame(false)?.unwrap();
    assert_eq!(before.voices[0].spatial.unwrap().position, [2., 3., 4.]);
    let mut bad = snapshot.clone();
    bad.entities[0].parent = Some(bad.entities[0].id);
    assert!(audio.sync_positions(&bad, 2).is_err());
    assert_eq!(audio.positions_revision, Some(1));
    assert!(audio.needs_positions(2));
    assert!(Arc::ptr_eq(&before.voices, &audio.voices));
    let mut missing = snapshot.clone();
    missing.entities.clear();
    assert!(audio.sync_positions(&missing, 2).is_err());
    assert_eq!(audio.positions_revision, Some(1));
    assert!(Arc::ptr_eq(&before.voices, &audio.voices));
    snapshot.entities[0].audio_source.as_mut().unwrap().spatial = false;
    let mut flat = Audio::new(&snapshot, &library, WorldId::new())?;
    let before = flat.frame(false)?.unwrap();
    flat.sync_positions(&bad, 99)?;
    assert!(Arc::ptr_eq(
        &before.voices,
        &flat.frame(false)?.unwrap().voices
    ));
    assert_eq!(
        flat.positions_revision, None,
        "2d does not inspect the invalid hierarchy or create a positions cache"
    );
    Ok(())
}

#[test]
fn generation_overflow_rejects_the_whole_batch_before_any_intent_can_change() -> Result<()> {
    let clip = AssetId::new();
    let mut entity = EntityData::new("sound");
    let id = entity.id;
    entity.audio_source = Some(AudioSource::new(clip));
    let snapshot = SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    };
    let library = [(
        clip,
        Arc::new(AudioData {
            version: 1,
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0; 8].into(),
        }),
    )]
    .into();
    let mut audio = Audio::new(&snapshot, &library, WorldId::new())?;
    Arc::make_mut(&mut audio.voices)[0].generation = u64::MAX - 1;
    let before = audio.frame(false)?.unwrap();
    assert!(
        audio
            .validate_commands(&[
                AudioCommand::Gain(id, 0.2),
                AudioCommand::Play(id),
                AudioCommand::Play(id)
            ])
            .is_err()
    );
    assert!(Arc::ptr_eq(
        &before.voices,
        &audio.frame(false)?.unwrap().voices
    ));
    let last_play = [AudioCommand::Play(id)];
    audio.validate_commands(&last_play)?;
    audio.apply_commands(&last_play);
    assert_eq!(audio.frame(false)?.unwrap().voices[0].generation, u64::MAX);
    assert!(!before.voices[0].playing);
    assert_eq!(before.voices[0].generation, u64::MAX - 1);
    Ok(())
}
