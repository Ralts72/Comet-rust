use anyhow::Result;
use comet_core::AssetId;
use comet_scene::{
    AudioSource, ContentSource, EntityData, FieldOverride, FieldUpdate, NavigationRegion,
    PoseUpdate, PrefabInstance, Scene, SceneSnapshot,
    schema::{self, FieldValue},
};

#[test]
fn audio_protocol_requires_scene_seven_and_preserves_required_clip_identity_and_overrides()
-> Result<()> {
    let clip = AssetId::new();
    for version in 1..=6 {
        let mut source = SceneSnapshot {
            version,
            entities: vec![EntityData::new("old")],
        };
        let json = source.to_json()?;
        assert!(!String::from_utf8(json.clone())?.contains("audio_source"));
        assert_eq!(SceneSnapshot::from_json(&json)?, source);
        source.entities[0].audio_source = Some(AudioSource::new(clip));
        assert!(source.validate().is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&json)?;
        value["entities"][0]["audio_source"] = serde_json::Value::Null;
        assert!(SceneSnapshot::from_json(&serde_json::to_vec(&value)?).is_err());
    }
    let mut entity = EntityData::new("sound");
    entity.audio_source = Some(AudioSource::new(clip));
    let id = entity.id;
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        ..Default::default()
    };
    let instance = PrefabInstance {
        id: comet_core::InstanceId::new(),
        prefab: AssetId::new(),
        parent: None,
        overrides: vec![FieldOverride {
            entity: id,
            field: schema::AUDIO_GAIN,
            value: FieldValue::Scalar(0.3),
        }],
    };
    let mut author = ContentSource {
        instances: vec![instance],
        ..Default::default()
    };
    struct Resolver(ContentSource);
    impl comet_scene::PrefabResolver for Resolver {
        fn prefab(&mut self, _: AssetId) -> Result<ContentSource> {
            Ok(self.0.clone())
        }
    }
    let mut resolver = Resolver(source.clone());
    assert_eq!(source.base.asset_dependencies(), [clip].into());
    let expanded = author.expand(&mut resolver)?;
    assert_eq!(
        expanded.entities[0].audio_source.as_ref().unwrap().gain,
        0.3
    );
    assert_eq!(
        expanded.entities[0].audio_source.as_ref().unwrap().clip,
        clip
    );
    author.base.version = 6;
    assert!(author.validate().is_err());
    Ok(())
}

#[test]
fn spatial_audio_preserves_compact_v7_sources_and_uses_v8_only_for_explicit_spatial_configuration()
-> Result<()> {
    let mut entity = EntityData::new("legacy sound");
    let id = entity.id;
    entity.audio_source = Some(AudioSource::new(AssetId::new()));
    let legacy = SceneSnapshot {
        version: 7,
        entities: vec![entity],
    };
    let bytes = legacy.to_json()?;
    let json = String::from_utf8(bytes.clone())?;
    assert!(
        !json.contains("spatial")
            && !json.contains("min_distance")
            && !json.contains("max_distance")
    );
    assert_eq!(SceneSnapshot::from_json(&bytes)?, legacy);
    for name in ["spatial", "min_distance", "max_distance"] {
        for version in [7, 8] {
            let mut raw: serde_json::Value = serde_json::from_slice(&bytes)?;
            raw["version"] = version.into();
            raw["entities"][0]["audio_source"][name] = serde_json::Value::Null;
            assert!(
                SceneSnapshot::from_json(&serde_json::to_vec(&raw)?).is_err(),
                "{version}/{name}"
            );
        }
    }
    for (field, value) in [
        (schema::AUDIO_SPATIAL, FieldValue::Bool(true)),
        (schema::AUDIO_MIN_DISTANCE, FieldValue::Scalar(0.)),
        (schema::AUDIO_MAX_DISTANCE, FieldValue::Scalar(50.)),
    ] {
        assert_eq!(schema::required_scene_version(field), 8);
        let mut source = legacy.clone();
        assert!(source.set_field(&[id], field, value.clone()).is_err());
        assert_eq!(source, legacy);
        source.version = 8;
        source.set_field(&[id], field, value)?;
        assert_eq!(
            source.entities[0]
                .audio_source
                .as_ref()
                .unwrap()
                .required_scene_version(),
            8
        );
    }
    assert_eq!(schema::required_scene_version(schema::AUDIO_CLIP), 7);
    let mut source = legacy.clone();
    source.version = 8;
    for (min, max) in [
        (-1., 30.),
        (1., 1.),
        (2., 1.),
        (0., f32::MIN_POSITIVE / 2.),
        (f32::NAN, 30.),
        (1., f32::INFINITY),
    ] {
        assert!(
            source
                .set_fields(&[
                    FieldUpdate {
                        entity: id,
                        field: schema::AUDIO_MIN_DISTANCE,
                        value: FieldValue::Scalar(min)
                    },
                    FieldUpdate {
                        entity: id,
                        field: schema::AUDIO_MAX_DISTANCE,
                        value: FieldValue::Scalar(max)
                    }
                ])
                .is_err()
        );
        assert_eq!(source.entities, legacy.entities);
    }
    source.set_field(&[id], schema::AUDIO_MIN_DISTANCE, FieldValue::Scalar(0.))?;
    source.validate()?;
    struct Resolver(ContentSource);
    impl comet_scene::PrefabResolver for Resolver {
        fn prefab(&mut self, _: AssetId) -> Result<ContentSource> {
            Ok(self.0.clone())
        }
    }
    let prefab = ContentSource {
        base: legacy,
        ..Default::default()
    };
    let mut author = ContentSource {
        base: SceneSnapshot {
            version: 7,
            entities: vec![],
        },
        instances: vec![PrefabInstance {
            id: comet_core::InstanceId::new(),
            prefab: AssetId::new(),
            parent: None,
            overrides: vec![FieldOverride {
                entity: id,
                field: schema::AUDIO_SPATIAL,
                value: FieldValue::Bool(false),
            }],
        }],
        ..Default::default()
    };
    assert!(
        author.validate().is_err(),
        "an explicit new field declaration requires v8 even at its default value"
    );
    author.base.version = 8;
    let expanded = author.expand(&mut Resolver(prefab))?;
    assert!(!expanded.entities[0].audio_source.as_ref().unwrap().spatial);
    assert_eq!(expanded.version, 8);
    Ok(())
}

#[test]
fn audio_and_navigation_field_edits_survive_ecs_pose_publication_and_invalid_gain_is_atomic()
-> Result<()> {
    let mut entity = EntityData::new("components");
    let id = entity.id;
    entity.audio_source = Some(AudioSource::new(AssetId::new()));
    entity.navigation_region = Some(NavigationRegion::default());
    let mut scene = Scene::open(SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    })?;
    let handle = scene.handle(id)?;
    scene.set_fields(&[
        FieldUpdate {
            entity: id,
            field: schema::AUDIO_GAIN,
            value: FieldValue::Scalar(0.4),
        },
        FieldUpdate {
            entity: id,
            field: schema::NAVIGATION_CELL_SIZE,
            value: FieldValue::Scalar(0.5),
        },
    ])?;
    scene.update_poses(&[PoseUpdate {
        entity: id,
        translation: [1., 0., 0.],
        rotation: [0., 0., 0., 1.],
    }])?;
    let snapshot = scene.shared_snapshot();
    assert_eq!(
        snapshot.entities[0].audio_source.as_ref().unwrap().gain,
        0.4
    );
    assert_eq!(
        snapshot.entities[0]
            .navigation_region
            .as_ref()
            .unwrap()
            .cell_size,
        0.5
    );
    assert!(scene.resolve(handle).is_ok());
    for gain in [-1., 1.1, f32::NAN, f32::INFINITY] {
        assert!(
            scene
                .set_fields(&[FieldUpdate {
                    entity: id,
                    field: schema::AUDIO_GAIN,
                    value: FieldValue::Scalar(gain)
                }])
                .is_err()
        );
        assert!(std::sync::Arc::ptr_eq(&snapshot, &scene.shared_snapshot()));
    }
    Ok(())
}
