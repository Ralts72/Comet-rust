use anyhow::Result;
use comet_core::{EntityId, math::Quat};
use comet_runtime::PlaySession;
use comet_scene::{
    EntityData, FieldUpdate, PhysicsBody, SceneSnapshot,
    schema::{self, FieldValue},
};
use std::sync::Arc;

fn update(entity: EntityId, field: schema::FieldKey, value: FieldValue) -> FieldUpdate {
    FieldUpdate {
        entity,
        field,
        value,
    }
}

#[test]
fn runtime_transform_fields_publish_as_one_validated_batch_and_keep_world_identity() -> Result<()> {
    let root = EntityData::new("visual");
    let id = root.id;
    let mut play = PlaySession::start(SceneSnapshot {
        entities: vec![root],
        ..Default::default()
    })?;
    let world = play.world_id();
    let rotation = Quat::from_rotation_z(0.5).to_array();
    play.set_fields(&[
        update(id, schema::TRANSLATION, FieldValue::Vec3([1., 2., 3.])),
        update(id, schema::ROTATION, FieldValue::Quat(rotation)),
        update(id, schema::SCALE, FieldValue::Vec3([2., 0.5, 1.])),
    ])?;
    assert_eq!(play.world_id(), world);
    let snapshot = play.snapshot();
    assert_eq!(snapshot.entities[0].transform.translation, [1., 2., 3.]);
    assert_eq!(snapshot.entities[0].transform.rotation, rotation);
    assert_eq!(snapshot.entities[0].transform.scale, [2., 0.5, 1.]);
    Ok(())
}

#[test]
fn invalid_or_forbidden_runtime_batch_never_publishes_its_valid_prefix() -> Result<()> {
    let visual = EntityData::new("visual");
    let id = visual.id;
    let mut body = EntityData::new("fixed body");
    body.physics = Some(PhysicsBody::default());
    let body_id = body.id;
    let mut play = PlaySession::start(SceneSnapshot {
        entities: vec![body, visual],
        ..Default::default()
    })?;
    let before = play.shared_snapshot();
    for invalid in [
        update(id, schema::ROTATION, FieldValue::Quat([0.; 4])),
        update(body_id, schema::TRANSLATION, FieldValue::Vec3([1., 2., 3.])),
        update(body_id, schema::MASS, FieldValue::Scalar(2.)),
        update(id, schema::SCRIPT_ENABLED, FieldValue::Bool(false)),
        update(
            EntityId::new(),
            schema::TRANSLATION,
            FieldValue::Vec3([0.; 3]),
        ),
        update(id, schema::TRANSLATION, FieldValue::Vec3([5.; 3])),
    ] {
        assert!(
            play.set_fields(&[
                update(id, schema::TRANSLATION, FieldValue::Vec3([1., 2., 3.])),
                invalid,
            ])
            .is_err()
        );
        assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
    }
    assert_eq!(*before, play.snapshot());
    Ok(())
}

#[test]
fn unresolved_author_references_cannot_enter_or_be_injected_into_a_running_world() -> Result<()> {
    use comet_scene::EntityRef;
    let mut visual = EntityData::new("author visual");
    let id = visual.id;
    let reference = EntityRef::in_instance(comet_core::InstanceId::new(), EntityId::new());
    visual.links.target = Some(reference.clone());
    let authored = SceneSnapshot {
        entities: vec![visual.clone()],
        ..Default::default()
    };
    authored.validate()?;
    let error = PlaySession::start(authored)
        .err()
        .expect("unresolved author world must be rejected");
    assert!(error.to_string().contains("unresolved prefab instance"));
    visual.links.target = None;
    let mut play = PlaySession::start(SceneSnapshot {
        version: 9,
        entities: vec![visual],
    })?;
    let before = play.shared_snapshot();
    let world = play.world_id();
    for (field, value) in [
        (schema::TARGET, FieldValue::Entity(Some(reference.clone()))),
        (schema::CONNECTIONS, FieldValue::Entities(vec![reference])),
    ] {
        assert!(
            play.set_fields(&[
                update(id, schema::TRANSLATION, FieldValue::Vec3([3.; 3])),
                update(id, field, value),
            ])
            .unwrap_err()
            .to_string()
            .contains("unresolved prefab instance")
        );
        assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
        assert_eq!(play.world_id(), world);
    }
    Ok(())
}
