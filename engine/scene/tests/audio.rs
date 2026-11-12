use anyhow::Result;
use comet_core::AssetId;
use comet_scene::{
    AudioSource, ContentSource, EntityData, FieldOverride, FieldUpdate, NavigationRegion,
    PoseUpdate, PrefabInstance, Scene, SceneSnapshot,
    schema::{self, FieldValue},
};

#[test]
fn audio_protocol_preserves_clip_identity_and_overrides() -> Result<()> {
    let clip = AssetId::new();
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
        translation: [0.; 3],
        id: comet_core::InstanceId::new(),
        prefab: AssetId::new(),
        parent: None,
        overrides: vec![FieldOverride {
            entity: id,
            field: schema::AUDIO_GAIN,
            value: FieldValue::Scalar(0.3),
        }],
    };
    let author = ContentSource {
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

    Ok(())
}

#[test]
fn spatial_audio_rejects_invalid_distances_and_expands_prefab_overrides() -> Result<()> {
    let mut entity = EntityData::new("sound");
    let id = entity.id;
    entity.audio_source = Some(AudioSource::new(AssetId::new()));
    let baseline = SceneSnapshot {
        version: comet_scene::scene::SCENE_VERSION,
        entities: vec![entity],
    };
    let bytes = baseline.to_json()?;
    assert_eq!(SceneSnapshot::from_json(&bytes)?, baseline);
    for field in ["spatial", "min_distance", "max_distance"] {
        let mut wire = serde_json::to_value(&baseline)?;
        wire["entities"][0]["audio_source"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(SceneSnapshot::from_json(&serde_json::to_vec(&wire)?).is_err());
    }
    let mut source = baseline.clone();

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
        assert_eq!(source.entities, baseline.entities);
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
        base: baseline,
        ..Default::default()
    };
    let author = ContentSource {
        base: SceneSnapshot {
            version: comet_scene::scene::SCENE_VERSION,
            entities: vec![],
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
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
    let expanded = author.expand(&mut Resolver(prefab))?;
    assert!(!expanded.entities[0].audio_source.as_ref().unwrap().spatial);
    assert_eq!(expanded.version, comet_scene::scene::SCENE_VERSION);
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
