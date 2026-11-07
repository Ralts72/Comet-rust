use anyhow::Result;
use comet_asset_data::{
    AudioData,
    audio::{AUDIO_VERSION, MAX_AUDIO_BYTES, MAX_AUDIO_SAMPLES},
    product::{AssetKind, Product},
};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{
    AudioSource, ContentSource, EntityData, FieldOverride, PrefabInstance, SceneSnapshot, schema,
};
use std::{collections::BTreeMap, sync::Arc};

fn audio() -> AudioData {
    AudioData {
        version: AUDIO_VERSION,
        sample_rate: 44_100,
        channels: 2,
        samples: Arc::from([i16::MIN, i16::MAX, -1, 0]),
    }
}

#[test]
fn pcm_protocol_round_trips_exact_samples_and_clones_share_storage() -> Result<()> {
    let audio = audio();
    audio.validate()?;
    let cloned = audio.clone();
    assert!(Arc::ptr_eq(&audio.samples, &cloned.samples));
    let encoded = serde_json::to_vec(&audio)?;
    assert!(encoded.len() <= MAX_AUDIO_BYTES);
    let decoded: AudioData = serde_json::from_slice(&encoded)?;
    assert_eq!(decoded, audio);
    let product = Product::Audio(decoded);
    assert_eq!(product.kind(), AssetKind::Audio);
    assert!(product.dependencies().is_empty());
    product.validate()?;
    let mut extra = serde_json::to_value(&audio)?;
    extra["device"] = "speaker".into();
    assert!(serde_json::from_value::<AudioData>(extra).is_err());
    Ok(())
}

#[test]
fn pcm_protocol_rejects_bad_shapes_versions_rates_and_sample_budgets() -> Result<()> {
    for rate in [8_000, 192_000] {
        let mut valid = audio();
        valid.sample_rate = rate;
        valid.validate()?;
    }
    let valid = audio();
    let invalid = [
        AudioData {
            version: AUDIO_VERSION + 1,
            ..valid.clone()
        },
        AudioData {
            sample_rate: 7_999,
            ..valid.clone()
        },
        AudioData {
            sample_rate: 192_001,
            ..valid.clone()
        },
        AudioData {
            channels: 0,
            ..valid.clone()
        },
        AudioData {
            channels: 3,
            ..valid.clone()
        },
        AudioData {
            samples: Arc::from([]),
            ..valid.clone()
        },
        AudioData {
            samples: Arc::from([1]),
            ..valid.clone()
        },
        AudioData {
            samples: vec![0; MAX_AUDIO_SAMPLES + 2].into(),
            ..valid.clone()
        },
    ];
    for value in invalid {
        assert!(value.validate().is_err());
    }
    let json = format!(
        "{{\"version\":1,\"sample_rate\":8000,\"channels\":1,\"samples\":[{}0]}}",
        "0,".repeat(MAX_AUDIO_SAMPLES)
    );
    let error = serde_json::from_str::<AudioData>(&json).unwrap_err();
    assert!(error.to_string().contains("sample budget"));
    Ok(())
}

#[test]
fn authored_and_prefab_override_audio_dependencies_require_audio_products() -> Result<()> {
    let clip = AssetId::new();
    let mut owner = EntityData::new("muted author source");
    owner.audio_source = Some(AudioSource {
        gain: 0.,
        ..AudioSource::new(clip)
    });
    let product = Product::Scene(ContentSource {
        base: SceneSnapshot {
            entities: vec![owner],
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(product.dependencies(), [clip].into_iter().collect());
    let mut kinds = BTreeMap::from([(clip, AssetKind::Audio)]);
    product.validate_dependency_kinds(|id| kinds.get(&id).copied())?;
    kinds.insert(clip, AssetKind::Material);
    assert!(
        product
            .validate_dependency_kinds(|id| kinds.get(&id).copied())
            .is_err()
    );

    let prefab = AssetId::new();
    let overridden = Product::Scene(ContentSource {
        instances: vec![PrefabInstance {
            id: InstanceId::new(),
            prefab,
            parent: None,
            overrides: vec![FieldOverride {
                entity: EntityId::new(),
                field: schema::AUDIO_CLIP,
                value: schema::FieldValue::Asset(clip),
            }],
        }],
        ..Default::default()
    });
    assert_eq!(
        overridden.dependencies(),
        [prefab, clip].into_iter().collect()
    );
    kinds.insert(prefab, AssetKind::Prefab);
    assert!(
        overridden
            .validate_dependency_kinds(|id| kinds.get(&id).copied())
            .is_err()
    );
    kinds.insert(clip, AssetKind::Audio);
    overridden.validate_dependency_kinds(|id| kinds.get(&id).copied())?;
    Ok(())
}
