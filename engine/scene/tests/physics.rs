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
