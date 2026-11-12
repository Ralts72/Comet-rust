use anyhow::{Result, bail, ensure};
use comet_core::{AssetId, InstanceId};
use comet_editor_core::Document;
use comet_scene::{
    BodyMotion, Character, ColliderShape, ContentSource, EntityData, PhysicsBody, PrefabInstance,
    PrefabResolver, SceneSnapshot,
};
use std::sync::Arc;

struct NoPrefabs;
impl PrefabResolver for NoPrefabs {
    fn prefab(&mut self, _: AssetId) -> Result<ContentSource> {
        bail!("unexpected prefab")
    }
}
fn capsule() -> PhysicsBody {
    PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    }
}

#[test]
fn component_transactions_keep_history_and_savepoints() -> Result<()> {
    let entity = EntityData::new("角色");
    let id = entity.id;
    let mut doc = Document::open(SceneSnapshot {
        version: comet_scene::scene::SCENE_VERSION,
        entities: vec![entity],
    })?;
    let legacy = doc.shared_source();
    assert!(
        doc.preview_physics(
            id,
            Some(capsule()),
            Some(Character::default()),
            &mut NoPrefabs
        )
        .is_err()
    );
    doc.begin()?;
    doc.preview_physics(
        id,
        Some(capsule()),
        Some(Character::default()),
        &mut NoPrefabs,
    )?;
    assert!(doc.is_dirty());
    for radius in [0.4, 0.5, 0.6] {
        let mut body = capsule();
        body.shape = ColliderShape::Capsule {
            half_height: 0.5,
            radius,
        };
        doc.preview_physics(id, Some(body), Some(Character::default()), &mut NoPrefabs)?;
    }
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);
    let configured = doc.shared_source();
    doc.undo()?;
    assert_eq!(doc.shared_source(), legacy);
    assert!(!doc.is_dirty());
    doc.redo()?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("main.scene");
    doc.save(&path)?;
    assert_eq!(Document::load(&path)?.shared_source(), configured);
    assert!(!doc.is_dirty());
    doc.begin()?;
    assert!(
        doc.preview_physics(id, None, Some(Character::default()), &mut NoPrefabs)
            .is_err()
    );
    assert!(Arc::ptr_eq(&configured, &doc.shared_source()));
    doc.preview_physics(id, None, None, &mut NoPrefabs)?;
    doc.cancel()?;
    assert!(!doc.is_dirty());
    doc.begin()?;
    doc.preview_physics(id, None, None, &mut NoPrefabs)?;
    doc.commit()?;
    assert!(doc.source().base.entities[0].physics.is_none());
    doc.undo()?;
    assert_eq!(doc.shared_source(), configured);
    assert!(!doc.is_dirty());
    Ok(())
}

#[test]
fn invalid_component_candidates_preserve_last_preview_and_reject_prefab_ownership() -> Result<()> {
    let root = EntityData::new("root");
    let id = root.id;
    let mut child = EntityData::new("child");
    child.parent = Some(id);
    let child_id = child.id;
    let mut doc = Document::open(SceneSnapshot {
        entities: vec![root, child],
        ..Default::default()
    })?;
    doc.begin()?;
    doc.preview_physics(
        id,
        Some(capsule()),
        Some(Character::default()),
        &mut NoPrefabs,
    )?;
    let valid = doc.shared_source();
    let revision = doc.revision();
    let mut invalid = capsule();
    invalid.sensor = true;
    assert!(
        doc.preview_physics(
            id,
            Some(invalid),
            Some(Character::default()),
            &mut NoPrefabs
        )
        .is_err()
    );
    assert!(
        doc.preview_physics(child_id, Some(PhysicsBody::default()), None, &mut NoPrefabs)
            .is_err()
    );
    assert!(
        doc.preview_physics(comet_core::EntityId::new(), None, None, &mut NoPrefabs)
            .is_err()
    );
    assert!(Arc::ptr_eq(&valid, &doc.shared_source()));
    assert_eq!(doc.revision(), revision);
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);

    let mut prefab_entity = EntityData::new("prefab player");
    prefab_entity.physics = Some(capsule());
    prefab_entity.character = Some(Character::default());
    let prefab_id = AssetId::new();
    let instance_id = InstanceId::new();
    let expanded_id = prefab_entity.id.in_instance(instance_id);
    struct Resolver(AssetId, ContentSource);
    impl PrefabResolver for Resolver {
        fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
            ensure!(id == self.0);
            Ok(self.1.clone())
        }
    }
    let mut resolver = Resolver(
        prefab_id,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![prefab_entity],
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let mut source = doc.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == id)
        .unwrap()
        .character = None;
    source.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: instance_id,
        prefab: prefab_id,
        parent: None,
        overrides: vec![],
    });
    doc = Document::open_source(source)?;
    let before = doc.shared_source();
    doc.begin()?;
    assert!(
        doc.preview_physics(
            id,
            Some(capsule()),
            Some(Character::default()),
            &mut resolver
        )
        .is_err()
    );
    assert!(
        doc.preview_physics(expanded_id, None, None, &mut resolver)
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &doc.shared_source()));
    doc.commit()?;
    assert_eq!(doc.history_len(), 0);
    Ok(())
}

#[test]
fn crouch_fields_preserve_authored_and_prefab_history_and_savepoints() -> Result<()> {
    use comet_scene::{
        FieldOverride,
        schema::{self, FieldValue},
    };
    let mut character = EntityData::new("character");
    character.physics = Some(capsule());
    character.character = Some(Character::default());
    let id = character.id;
    let snapshot = SceneSnapshot {
        version: comet_scene::scene::SCENE_VERSION,
        entities: vec![character],
    };
    let mut doc = Document::open(snapshot.clone())?;
    let legacy = doc.shared_source();
    doc.begin()?;
    doc.preview_field(&[id], schema::CROUCH_SPEED, FieldValue::Scalar(1.5))?;
    doc.commit()?;
    assert_eq!(
        doc.source().base.entities[0]
            .character
            .as_ref()
            .unwrap()
            .crouch_speed,
        1.5
    );
    doc.undo()?;
    assert_eq!(doc.shared_source(), legacy);
    doc.redo()?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("new.scene");
    doc.save(&path)?;
    assert_eq!(Document::load(&path)?.shared_source(), doc.shared_source());

    let prefab = AssetId::new();
    let instance = InstanceId::new();
    struct Resolver(AssetId, ContentSource);
    impl PrefabResolver for Resolver {
        fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
            ensure!(id == self.0);
            Ok(self.1.clone())
        }
    }
    let mut resolver = Resolver(
        prefab,
        ContentSource {
            base: snapshot,
            ..Default::default()
        },
    );
    let mut doc = Document::open_source(ContentSource {
        base: SceneSnapshot {
            version: comet_scene::scene::SCENE_VERSION,
            entities: vec![],
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    let original = doc.shared_source();
    doc.begin()?;
    doc.preview_override(
        instance,
        FieldOverride {
            entity: id,
            field: schema::CROUCH_HEIGHT_RATIO,
            value: FieldValue::Scalar(0.5),
        },
        &mut resolver,
    )?;
    doc.commit()?;
    let expanded = doc.source().expand(&mut resolver)?;
    assert_eq!(expanded.version, comet_scene::scene::SCENE_VERSION);
    assert_eq!(
        expanded.entities[0]
            .character
            .as_ref()
            .unwrap()
            .crouch_height_ratio,
        0.5
    );
    doc.undo()?;
    assert_eq!(doc.shared_source(), original);
    doc.redo()?;
    doc.save(&path)?;
    assert_eq!(
        Document::load(&path)?.source().expand(&mut resolver)?,
        expanded
    );
    Ok(())
}
