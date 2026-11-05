use anyhow::Result;
use comet_core::EntityId;
use comet_scene::{
    BodyMotion, Character, ColliderShape, ContentSource, EntityData, PhysicsBody, PoseUpdate,
    Scene, SceneSnapshot,
    schema::{self, FieldValue},
};
fn body() -> EntityData {
    let mut e = EntityData::new("body");
    e.physics = Some(PhysicsBody::default());
    e
}
#[test]
fn legacy_scenes_roundtrip_without_silent_version_or_identity_changes() -> Result<()> {
    let e = EntityData::new("legacy");
    let id = e.id;
    let old = SceneSnapshot {
        version: 1,
        entities: vec![e],
    };
    let json = old.to_json()?;
    assert!(!String::from_utf8_lossy(&json).contains("physics"));
    let loaded = SceneSnapshot::from_json(&json)?;
    let mut world = Scene::open(loaded.clone())?;
    assert_eq!(world.snapshot(), old);
    assert_eq!(world.snapshot().entities[0].id, id);
    let mut physics = old;
    physics.entities[0].physics = Some(PhysicsBody::default());
    assert!(physics.validate().is_err());
    physics.version = 2;
    assert_eq!(SceneSnapshot::from_json(&physics.to_json()?)?, physics);
    let source = ContentSource {
        base: loaded,
        ..Default::default()
    };
    struct Empty;
    impl comet_scene::PrefabResolver for Empty {
        fn prefab(&mut self, _: comet_core::AssetId) -> Result<ContentSource> {
            unreachable!()
        }
    }
    assert_eq!(source.expand(&mut Empty)?.version, 1);
    Ok(())
}

#[test]
fn crouch_configuration_has_explicit_legacy_defaults_and_atomic_schema_validation() -> Result<()> {
    let mut entity = body();
    entity.physics.as_mut().unwrap().motion = BodyMotion::Kinematic;
    entity.physics.as_mut().unwrap().shape = ColliderShape::Capsule {
        half_height: 0.5,
        radius: 0.3,
    };
    entity.character = Some(Character::default());
    let id = entity.id;
    let snapshot = SceneSnapshot {
        version: 2,
        entities: vec![entity],
    };
    assert!(!String::from_utf8_lossy(&snapshot.to_json()?).contains("crouch"));
    let mut legacy: serde_json::Value = serde_json::from_slice(&snapshot.to_json()?)?;
    legacy["entities"][0]["character"]
        .as_object_mut()
        .unwrap()
        .remove("crouch_speed");
    legacy["entities"][0]["character"]
        .as_object_mut()
        .unwrap()
        .remove("crouch_height_ratio");
    let mut loaded = SceneSnapshot::from_json(&serde_json::to_vec(&legacy)?)?;
    assert_eq!(loaded, snapshot);
    assert!(
        loaded
            .set_field(&[id], schema::CROUCH_SPEED, FieldValue::Scalar(1.2))
            .is_err()
    );
    assert_eq!(loaded, snapshot);
    loaded.version = 3;
    let accepted = loaded.clone();
    for (field, value) in [
        (schema::CROUCH_SPEED, 0.),
        (schema::CROUCH_HEIGHT_RATIO, 0.09),
        (schema::CROUCH_HEIGHT_RATIO, 1.01),
    ] {
        assert!(
            loaded
                .set_field(&[id], field, FieldValue::Scalar(value))
                .is_err()
        );
        assert_eq!(loaded, accepted);
    }
    loaded.set_field(&[id], schema::CROUCH_SPEED, FieldValue::Scalar(1.2))?;
    loaded.set_field(&[id], schema::CROUCH_HEIGHT_RATIO, FieldValue::Scalar(0.5))?;
    assert_eq!(
        loaded.entities[0].character.as_ref().unwrap().crouch_speed,
        1.2
    );
    assert_eq!(SceneSnapshot::from_json(&loaded.to_json()?)?, loaded);
    Ok(())
}

#[test]
fn crouch_prefab_overrides_require_v3_and_nested_expansion_keeps_the_newest_version() -> Result<()>
{
    use comet_core::{AssetId, InstanceId};
    use comet_scene::{FieldOverride, PrefabInstance, PrefabResolver};
    let asset = AssetId::new();
    let instance = InstanceId::new();
    let mut entity = body();
    entity.physics.as_mut().unwrap().motion = BodyMotion::Kinematic;
    entity.physics.as_mut().unwrap().shape = ColliderShape::Capsule {
        half_height: 0.5,
        radius: 0.3,
    };
    entity.character = Some(Character::default());
    let id = entity.id;
    let prefab = ContentSource {
        base: SceneSnapshot {
            version: 2,
            entities: vec![entity],
        },
        ..Default::default()
    };
    struct Library(std::collections::BTreeMap<AssetId, ContentSource>);
    impl PrefabResolver for Library {
        fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
            Ok(self.0[&id].clone())
        }
    }
    let mut source = ContentSource {
        base: SceneSnapshot {
            version: 2,
            entities: vec![],
        },
        instances: vec![PrefabInstance {
            id: instance,
            prefab: asset,
            parent: None,
            overrides: vec![FieldOverride {
                entity: id,
                field: schema::CROUCH_HEIGHT_RATIO,
                value: FieldValue::Scalar(0.5),
            }],
        }],
        ..Default::default()
    };
    assert!(source.validate().is_err());
    source.base.version = 3;
    let mut library = Library(std::collections::BTreeMap::from([(asset, prefab.clone())]));
    // Existing v1 containers could already override physics fields in a v2 prefab.
    let mut legacy = source.clone();
    legacy.base.version = 1;
    legacy.instances[0].overrides[0].field = schema::MASS;
    legacy.instances[0].overrides[0].value = FieldValue::Scalar(2.);
    let legacy = ContentSource::from_json(&legacy.to_json()?)?.expand(&mut library)?;
    assert_eq!(legacy.version, 2);
    assert_eq!(legacy.entities[0].physics.as_ref().unwrap().mass, 2.);
    let expanded = source.expand(&mut library)?;
    assert_eq!(expanded.version, 3);
    assert_eq!(expanded.entities[0].id, id.in_instance(instance));
    assert_eq!(
        expanded.entities[0]
            .character
            .as_ref()
            .unwrap()
            .crouch_height_ratio,
        0.5
    );
    // An older outer container must still retain the inner prefab's scene version.
    let parent_asset = AssetId::new();
    let outer = ContentSource {
        base: SceneSnapshot {
            version: 1,
            entities: vec![],
        },
        instances: vec![PrefabInstance {
            id: InstanceId::new(),
            prefab: parent_asset,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    library.0.insert(parent_asset, source);
    assert_eq!(outer.expand(&mut library)?.version, 3);
    assert_eq!(library.0[&asset], prefab);
    Ok(())
}
#[test]
fn physics_validation_and_schema_edits_reject_invalid_candidates_atomically() -> Result<()> {
    let e = body();
    let id = e.id;
    let mut snapshot = SceneSnapshot {
        entities: vec![e],
        ..Default::default()
    };
    let original = snapshot.clone();
    assert!(
        snapshot
            .set_field(&[id], schema::MASS, FieldValue::Scalar(0.))
            .is_err()
    );
    assert_eq!(snapshot, original);
    snapshot.set_field(&[id], schema::MASS, FieldValue::Scalar(2.))?;
    let mut c = body();
    c.character = Some(Character::default());
    assert!(
        SceneSnapshot {
            entities: vec![c.clone()],
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    c.physics.as_mut().unwrap().motion = BodyMotion::Kinematic;
    c.physics.as_mut().unwrap().shape = ColliderShape::Capsule {
        half_height: 0.5,
        radius: 0.3,
    };
    let mut two = c.clone();
    two.id = EntityId::new();
    assert!(
        SceneSnapshot {
            entities: vec![c.clone(), two],
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    c.transform.scale = [1., 2., 1.];
    assert!(
        SceneSnapshot {
            entities: vec![c],
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    Ok(())
}
#[test]
fn batched_pose_writeback_preserves_handles_and_rolls_back_on_invalid_or_stale_input() -> Result<()>
{
    let a = body();
    let mut b = body();
    b.transform.scale = [2.; 3];
    let (a_id, b_id) = (a.id, b.id);
    let mut world = Scene::open(SceneSnapshot {
        entities: vec![a, b],
        ..Default::default()
    })?;
    let handle = world.handle(a_id)?;
    let update = |entity, translation| PoseUpdate {
        entity,
        translation,
        rotation: [0., 0., 0., 1.],
    };
    let before = world.snapshot();
    assert!(
        world
            .update_poses(&[update(a_id, [3.; 3]), update(b_id, [f32::NAN; 3])])
            .is_err()
    );
    assert_eq!(world.snapshot(), before);
    assert!(
        world
            .update_poses(&[update(a_id, [3.; 3]), update(EntityId::new(), [0.; 3])])
            .is_err()
    );
    assert_eq!(world.snapshot(), before);
    world.update_poses(&[update(a_id, [3.; 3]), update(b_id, [4.; 3])])?;
    assert_eq!(world.resolve(handle)?.transform.translation, [3.; 3]);
    assert_eq!(
        world
            .snapshot()
            .entities
            .iter()
            .find(|e| e.id == b_id)
            .unwrap()
            .transform
            .scale,
        [2.; 3]
    );
    Ok(())
}

#[test]
fn immutable_snapshots_are_reused_until_a_successful_pose_change() -> Result<()> {
    use std::sync::Arc;
    let e = body();
    let id = e.id;
    let mut scene = Scene::open(SceneSnapshot {
        entities: vec![e],
        ..Default::default()
    })?;
    let before = scene.shared_snapshot();
    let revision = scene.revision();
    let update = |x| PoseUpdate {
        entity: id,
        translation: [x, 0., 0.],
        rotation: [0., 0., 0., 1.],
    };
    scene.update_poses(&[update(0.)])?;
    assert_eq!(scene.revision(), revision);
    assert!(Arc::ptr_eq(&before, &scene.shared_snapshot()));
    assert!(scene.update_poses(&[update(f32::NAN)]).is_err());
    assert!(Arc::ptr_eq(&before, &scene.shared_snapshot()));
    scene.update_poses(&[update(2.)])?;
    assert!(scene.revision() > revision);
    assert!(!Arc::ptr_eq(&before, &scene.shared_snapshot()));
    assert_eq!(before.entities[0].transform.translation, [0.; 3]);
    Ok(())
}

#[test]
fn round_colliders_require_uniform_scale_independent_of_object_size() -> anyhow::Result<()> {
    for shape in [
        ColliderShape::Sphere { radius: 0.5 },
        ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.25,
        },
    ] {
        for magnitude in [1e-6, 1., 1e6] {
            let mut entity = EntityData::new("round collider");
            entity.physics = Some(PhysicsBody {
                shape: shape.clone(),
                ..Default::default()
            });
            entity.transform.scale = [magnitude; 3];
            let mut scene = SceneSnapshot {
                entities: vec![entity],
                ..Default::default()
            };
            scene.validate()?;
            scene.entities[0].transform.scale[1] *= 1.1;
            assert!(
                scene.validate().is_err(),
                "10 percent anisotropy at scale {magnitude}"
            );
            scene.entities[0].transform.scale[1] = f32::from_bits(magnitude.to_bits() + 1);
            scene.validate()?;
        }
    }
    Ok(())
}
