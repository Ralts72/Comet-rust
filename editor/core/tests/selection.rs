use anyhow::Result;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::{Document, FieldEdit, FieldTarget, ObjectTarget};
use comet_scene::{
    ContentSource, EntityData, EntityRef, FieldOverride, PrefabInstance, PrefabResolver,
    SceneSnapshot,
    schema::{self, FieldValue},
};

#[test]
fn explicit_override_reset_restores_inheritance_without_touching_other_declarations() -> Result<()>
{
    let mut f = fixture()?;
    let other = EntityData::new("other prefab entity");
    let other_id = other.id;
    f.library.source.base.entities.push(other);
    let mut source = f.doc.source();
    let direct = source
        .instances
        .iter_mut()
        .find(|instance| instance.id == f.direct)
        .unwrap();
    direct.overrides = vec![
        FieldOverride {
            entity: f.prefab_entity,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([0.; 3]),
        },
        FieldOverride {
            entity: f.prefab_entity,
            field: schema::ROTATION,
            value: FieldValue::Quat([0., 0., 0., 1.]),
        },
        FieldOverride {
            entity: f.prefab_entity,
            field: schema::SCALE,
            value: FieldValue::Vec3([2.; 3]),
        },
        FieldOverride {
            entity: other_id,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([3.; 3]),
        },
    ];
    source
        .instances
        .iter_mut()
        .find(|instance| instance.id == f.attached)
        .unwrap()
        .overrides
        .push(FieldOverride {
            entity: f.prefab_entity,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([9., 0., 0.]),
        });
    f.doc = Document::open_source(source)?;
    let before = f.doc.shared_source();
    let mut expected = f.doc.source();
    expected
        .instances
        .iter_mut()
        .find(|instance| instance.id == f.direct)
        .unwrap()
        .overrides
        .retain(|value| {
            value.entity != f.prefab_entity
                || (value.field != schema::TRANSLATION && value.field != schema::ROTATION)
        });
    f.doc.revert_overrides(
        f.direct,
        f.prefab_entity,
        &[schema::TRANSLATION, schema::ROTATION],
        &mut f.library,
    )?;
    let after = f.doc.shared_source();
    assert_eq!(*after, expected);
    assert_eq!(f.doc.history_len(), 1);
    assert!(f.doc.is_dirty());
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("inheritance.scene");
    f.doc.save(&path)?;
    assert!(!f.doc.is_dirty());
    assert_eq!(Document::load(&path)?.shared_source(), after);
    f.doc.undo()?;
    assert_eq!(f.doc.shared_source(), before);
    assert!(f.doc.is_dirty());
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), after);
    assert!(!f.doc.is_dirty());

    // The removed equal-to-source pin must follow a later source revision. The
    // other instance and the retained scale/other-entity declarations stay pinned.
    f.library.source.base.entities[0].transform.translation = [8., 0., 0.];
    let expanded = f.doc.shared_source().expand(&mut f.library)?;
    let direct = expanded
        .entities
        .iter()
        .find(|entity| entity.id == f.prefab_entity.in_instance(f.direct))
        .unwrap();
    assert_eq!(direct.transform.translation, [8., 0., 0.]);
    assert_eq!(direct.transform.scale, [2.; 3]);
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|entity| entity.id == f.prefab_entity.in_instance(f.attached))
            .unwrap()
            .transform
            .translation,
        [9., 0., 0.]
    );
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|entity| entity.id == other_id.in_instance(f.direct))
            .unwrap()
            .transform
            .translation,
        [3.; 3]
    );
    Ok(())
}

#[test]
fn no_op_and_rejected_override_resets_preserve_redo_and_accepted_preview() -> Result<()> {
    let mut f = fixture()?;
    let mut source = f.doc.source();
    source
        .instances
        .iter_mut()
        .find(|instance| instance.id == f.direct)
        .unwrap()
        .overrides = vec![FieldOverride {
        entity: f.prefab_entity,
        field: schema::SCALE,
        value: FieldValue::Vec3([2.; 3]),
    }];
    f.doc = Document::open_source(source)?;
    f.doc
        .revert_overrides(f.direct, f.prefab_entity, &[schema::SCALE], &mut f.library)?;
    let after = f.doc.shared_source();
    f.doc.undo()?;
    let before = f.doc.shared_source();
    let revision = f.doc.revision();
    let state = f.doc.state_id();
    for fields in [&[][..], &[schema::TRANSLATION][..]] {
        f.doc
            .revert_overrides(f.direct, f.prefab_entity, fields, &mut f.library)?;
        assert!(std::sync::Arc::ptr_eq(&before, &f.doc.shared_source()));
        assert_eq!(f.doc.revision(), revision);
        assert_eq!(f.doc.state_id(), state);
        assert_eq!(f.doc.history_len(), 0);
        assert!(!f.doc.is_dirty());
    }
    let unknown = schema::FieldKey {
        component: u32::MAX,
        field: 1,
    };
    for (instance, entity, fields) in [
        (InstanceId::new(), f.prefab_entity, vec![schema::SCALE]),
        (f.direct, f.root, vec![schema::SCALE]),
        (
            f.direct,
            f.prefab_entity.in_instance(f.direct),
            vec![schema::SCALE],
        ),
        (f.direct, EntityId::new(), vec![]),
        (f.direct, f.prefab_entity, vec![schema::SCALE, unknown]),
        (
            f.direct,
            f.prefab_entity,
            vec![schema::SCALE, schema::SCALE],
        ),
        (f.direct, f.prefab_entity, vec![schema::SCALE, schema::MASS]),
    ] {
        assert!(
            f.doc
                .revert_overrides(instance, entity, &fields, &mut f.library)
                .is_err()
        );
        assert!(std::sync::Arc::ptr_eq(&before, &f.doc.shared_source()));
        assert_eq!(f.doc.revision(), revision);
        assert_eq!(f.doc.state_id(), state);
        assert_eq!(f.doc.history_len(), 0);
    }
    f.library.fail = true;
    assert!(
        f.doc
            .revert_overrides(f.direct, f.prefab_entity, &[schema::SCALE], &mut f.library,)
            .is_err()
    );
    f.library.fail = false;
    assert!(std::sync::Arc::ptr_eq(&before, &f.doc.shared_source()));
    f.doc.begin()?;
    f.doc.preview_field_values(
        &[translated(FieldTarget::Authored(f.root), [1., 2., 3.])],
        &mut f.library,
    )?;
    let preview = f.doc.shared_source();
    let revision = f.doc.revision();
    let calls = f.library.calls;
    assert!(
        f.doc
            .revert_overrides(f.direct, f.prefab_entity, &[schema::SCALE], &mut f.library,)
            .is_err()
    );
    assert_eq!(f.library.calls, calls);
    assert!(f.doc.is_editing());
    assert!(std::sync::Arc::ptr_eq(&preview, &f.doc.shared_source()));
    assert_eq!(f.doc.revision(), revision);
    f.doc.cancel()?;
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), after);
    Ok(())
}

#[test]
fn override_reset_can_recover_a_declaration_after_its_source_component_is_removed() -> Result<()> {
    let mut f = fixture()?;
    f.library.source.base.entities[0].physics = Some(Default::default());
    let mut source = f.doc.source();
    source.instances.retain(|instance| instance.id == f.direct);
    source
        .instances
        .iter_mut()
        .find(|instance| instance.id == f.direct)
        .unwrap()
        .overrides = vec![
        FieldOverride {
            entity: f.prefab_entity,
            field: schema::MASS,
            value: FieldValue::Scalar(3.),
        },
        FieldOverride {
            entity: f.prefab_entity,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([2.; 3]),
        },
    ];
    f.doc = Document::open_source(source)?;
    f.doc.shared_source().expand(&mut f.library)?;
    f.library.source.base.entities[0].physics = None;
    assert!(f.doc.shared_source().expand(&mut f.library).is_err());
    f.doc
        .revert_overrides(f.direct, f.prefab_entity, &[schema::MASS], &mut f.library)?;
    let after = f.doc.shared_source();
    let instance = after
        .instances
        .iter()
        .find(|instance| instance.id == f.direct)
        .unwrap();
    assert_eq!(instance.overrides.len(), 1);
    assert_eq!(instance.overrides[0].field, schema::TRANSLATION);
    after.expand(&mut f.library)?;
    assert_eq!(f.doc.history_len(), 1);
    Ok(())
}

#[test]
fn override_reset_rejects_an_overflowing_expansion_without_publishing_or_losing_redo() -> Result<()>
{
    let mut f = fixture()?;
    f.library.source.base.entities[0].transform.scale = [1e20; 3];
    let mut source = f.doc.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.root)
        .unwrap()
        .transform
        .scale = [1e20; 3];
    source
        .instances
        .retain(|instance| instance.id == f.attached);
    source.instances[0].overrides = vec![FieldOverride {
        entity: f.prefab_entity,
        field: schema::SCALE,
        value: FieldValue::Vec3([1e-20; 3]),
    }];
    source.expand(&mut f.library)?;
    f.doc = Document::open_source(source)?;
    f.doc
        .clone_selection(&[ObjectTarget::Authored(f.observer)], &mut f.library)?;
    let after = f.doc.shared_source();
    f.doc.undo()?;
    let before = f.doc.shared_source();
    let revision = f.doc.revision();
    let state = f.doc.state_id();
    assert!(
        f.doc
            .revert_overrides(
                f.attached,
                f.prefab_entity,
                &[schema::SCALE],
                &mut f.library,
            )
            .is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&before, &f.doc.shared_source()));
    assert_eq!(f.doc.revision(), revision);
    assert_eq!(f.doc.state_id(), state);
    assert_eq!(f.doc.history_len(), 0);
    assert!(!f.doc.is_dirty());
    f.doc.shared_source().expand(&mut f.library)?;
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), after);
    Ok(())
}

struct Library {
    asset: AssetId,
    source: ContentSource,
    calls: usize,
    fail: bool,
}
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.calls += 1;
        anyhow::ensure!(!self.fail && id == self.asset, "missing prefab");
        Ok(self.source.clone())
    }
}
struct Fixture {
    doc: Document,
    library: Library,
    root: EntityId,
    child: EntityId,
    observer: EntityId,
    attached: InstanceId,
    direct: InstanceId,
    prefab_entity: EntityId,
}
fn fixture() -> Result<Fixture> {
    let prefab = EntityData::new("prefab root");
    let prefab_entity = prefab.id;
    let library = Library {
        asset: AssetId::new(),
        source: ContentSource {
            base: SceneSnapshot {
                entities: vec![prefab],
                ..Default::default()
            },
            ..Default::default()
        },
        calls: 0,
        fail: false,
    };
    let root = EntityData::new("root");
    let mut child = EntityData::new("child");
    child.parent = Some(root.id);
    child.links.target = Some(EntityRef::local(root.id));
    let mut observer = EntityData::new("observer");
    observer.links.target = Some(EntityRef::local(root.id));
    observer.links.connections = vec![
        EntityRef::local(child.id),
        EntityRef::External {
            scene: AssetId::new(),
            entity: child.id,
        },
    ];
    let attached = PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab: library.asset,
        parent: Some(child.id),
        overrides: vec![],
    };
    let direct = PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab: library.asset,
        parent: None,
        overrides: vec![],
    };
    let result = Fixture {
        root: root.id,
        child: child.id,
        observer: observer.id,
        attached: attached.id,
        direct: direct.id,
        prefab_entity,
        library,
        doc: Document::open_source(ContentSource {
            base: SceneSnapshot {
                entities: vec![root, child, observer],
                ..Default::default()
            },
            instances: vec![attached, direct],
            ..Default::default()
        })?,
    };
    Ok(result)
}

#[test]
fn mixed_clone_deduplicates_subtrees_and_attached_instances_in_one_history_state() -> Result<()> {
    let mut f = fixture()?;
    let mut source = f.doc.source();
    let external = source
        .base
        .entities
        .iter()
        .find(|e| e.id == f.observer)
        .unwrap()
        .links
        .connections[1]
        .clone();
    for entity in &mut source.base.entities {
        if entity.id == f.root {
            entity.links.target = Some(EntityRef::in_instance(f.direct, f.prefab_entity));
        } else if entity.id == f.child {
            entity.links.connections = vec![
                EntityRef::in_instance(f.attached, f.prefab_entity),
                external.clone(),
            ];
        }
    }
    f.doc = Document::open_source(source)?;
    let before = f.doc.shared_source();
    let copied = f.doc.clone_selection(
        &[
            ObjectTarget::Authored(f.root),
            ObjectTarget::Authored(f.child),
            ObjectTarget::Authored(f.root),
            ObjectTarget::Prefab(f.attached),
            ObjectTarget::Prefab(f.direct),
            ObjectTarget::Prefab(f.direct),
        ],
        &mut f.library,
    )?;
    assert_eq!(copied.entities.len(), 2);
    assert_eq!(copied.instances.len(), 2);
    assert!(copied.entities.contains_key(&f.child));
    let after = f.doc.shared_source();
    assert_eq!(after.base.entities.len(), 5);
    assert_eq!(after.instances.len(), 4);
    let child = after
        .base
        .entities
        .iter()
        .find(|entity| entity.id == copied.entities[&f.child])
        .unwrap();
    assert_eq!(child.parent, Some(copied.entities[&f.root]));
    assert_eq!(
        child.links.target,
        Some(EntityRef::local(copied.entities[&f.root]))
    );
    assert_eq!(
        child.links.connections,
        [
            EntityRef::in_instance(copied.instances[&f.attached], f.prefab_entity),
            external,
        ]
    );
    let root = after
        .base
        .entities
        .iter()
        .find(|entity| entity.id == copied.entities[&f.root])
        .unwrap();
    assert_eq!(
        root.links.target,
        Some(EntityRef::in_instance(
            copied.instances[&f.direct],
            f.prefab_entity,
        ))
    );
    for entity in &before.base.entities {
        assert_eq!(
            after.base.entities.iter().find(|e| e.id == entity.id),
            Some(entity)
        );
    }
    let attached = after
        .instances
        .iter()
        .find(|instance| instance.id == copied.instances[&f.attached])
        .unwrap();
    assert_eq!(attached.parent, Some(copied.entities[&f.child]));
    assert_eq!(attached.prefab, f.library.asset);
    let direct = after
        .instances
        .iter()
        .find(|instance| instance.id == copied.instances[&f.direct])
        .unwrap();
    assert_eq!(direct.parent, None);
    assert_eq!(f.doc.history_len(), 1);
    f.doc.undo()?;
    assert_eq!(f.doc.shared_source(), before);
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), after);
    Ok(())
}

#[test]
fn mixed_remove_is_one_undo_step_and_clears_deleted_author_and_instance_links() -> Result<()> {
    let mut f = fixture()?;
    let mut source = f.doc.source();
    let spare = InstanceId::new();
    source.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: spare,
        prefab: f.library.asset,
        parent: None,
        overrides: vec![],
    });
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.observer)
        .unwrap()
        .links
        .connections
        .extend([
            EntityRef::in_instance(f.direct, f.prefab_entity),
            EntityRef::in_instance(f.attached, f.prefab_entity),
            EntityRef::in_instance(spare, f.prefab_entity),
        ]);
    f.doc = Document::open_source(source)?;
    let before = f.doc.shared_source();
    f.doc.remove_selection(
        &[
            ObjectTarget::Authored(f.root),
            ObjectTarget::Authored(f.child),
            ObjectTarget::Authored(f.root),
            ObjectTarget::Prefab(f.direct),
        ],
        &mut f.library,
    )?;
    let after = f.doc.shared_source();
    assert_eq!(after.base.entities.len(), 1);
    assert_eq!(after.base.entities[0].id, f.observer);
    assert!(after.base.entities[0].links.target.is_none());
    assert_eq!(after.base.entities[0].links.connections.len(), 2);
    assert!(matches!(
        after.base.entities[0].links.connections[0],
        EntityRef::External { .. }
    ));
    assert_eq!(
        after.base.entities[0].links.connections[1],
        EntityRef::in_instance(spare, f.prefab_entity)
    );
    assert_eq!(after.instances.len(), 1);
    assert_eq!(after.instances[0].id, spare);
    assert_eq!(f.doc.history_len(), 1);
    f.doc.undo()?;
    assert_eq!(f.doc.shared_source(), before);
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), after);
    Ok(())
}

#[test]
fn invalid_mixed_structure_does_not_publish_a_prefix_or_discard_redo() -> Result<()> {
    let mut f = fixture()?;
    f.doc
        .clone_selection(&[ObjectTarget::Authored(f.root)], &mut f.library)?;
    f.doc.undo()?;
    let before = f.doc.shared_source();
    let revision = f.doc.revision();
    for stale in [
        ObjectTarget::Authored(EntityId::new()),
        ObjectTarget::Prefab(InstanceId::new()),
    ] {
        let selection = [
            ObjectTarget::Authored(f.root),
            ObjectTarget::Prefab(f.direct),
            stale,
        ];
        assert!(f.doc.clone_selection(&selection, &mut f.library).is_err());
        assert!(f.doc.remove_selection(&selection, &mut f.library).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &f.doc.shared_source()));
        assert_eq!(f.doc.revision(), revision);
        assert_eq!(f.doc.history_len(), 0);
    }
    f.library.fail = true;
    assert!(
        f.doc
            .clone_selection(&[ObjectTarget::Authored(f.root)], &mut f.library)
            .is_err()
    );
    assert!(
        f.doc
            .remove_selection(&[ObjectTarget::Authored(f.root)], &mut f.library)
            .is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&before, &f.doc.shared_source()));
    f.library.fail = false;
    assert!(
        f.doc
            .clone_selection(&[], &mut f.library)?
            .entities
            .is_empty()
    );
    f.doc.remove_selection(&[], &mut f.library)?;
    assert_eq!(f.doc.revision(), revision);
    f.doc.redo()?;
    assert_eq!(f.doc.history_len(), 1);
    Ok(())
}

fn translated(target: FieldTarget, translation: [f32; 3]) -> FieldEdit {
    FieldEdit {
        target,
        field: schema::TRANSLATION,
        value: FieldValue::Vec3(translation),
    }
}

#[test]
fn value_previews_restore_original_declarations_without_erasing_other_field_changes() -> Result<()>
{
    let mut f = fixture()?;
    let target = FieldTarget::Prefab {
        instance: f.direct,
        entity: f.prefab_entity,
    };
    let before = f.doc.shared_source();
    let state = f.doc.state_id();
    let scaled = |value| FieldEdit {
        target,
        field: schema::SCALE,
        value: FieldValue::Vec3(value),
    };
    assert!(f.doc.preview_field_values(&[], &mut f.library).is_err());
    f.doc.begin()?;
    f.doc.preview_field_values(
        &[
            translated(target, [2., 3., 4.]),
            scaled([2.; 3]),
            translated(FieldTarget::Authored(f.root), [7., 8., 9.]),
        ],
        &mut f.library,
    )?;
    f.doc
        .preview_field_values(&[translated(target, [0.; 3])], &mut f.library)?;
    let partial = f.doc.shared_source();
    let instance = partial
        .instances
        .iter()
        .find(|instance| instance.id == f.direct)
        .unwrap();
    assert_eq!(instance.overrides.len(), 1);
    assert_eq!(instance.overrides[0].field, schema::SCALE);
    assert_eq!(
        partial
            .base
            .entities
            .iter()
            .find(|entity| entity.id == f.root)
            .unwrap()
            .transform
            .translation,
        [7., 8., 9.]
    );
    assert!(f.doc.is_dirty());
    f.doc.preview_field_values(
        &[
            scaled([1.; 3]),
            translated(FieldTarget::Authored(f.root), [0.; 3]),
        ],
        &mut f.library,
    )?;
    assert!(std::sync::Arc::ptr_eq(&before, &f.doc.shared_source()));
    assert_eq!(f.doc.state_id(), state);
    assert!(!f.doc.is_dirty());
    f.doc.commit()?;
    assert_eq!(f.doc.history_len(), 0);
    Ok(())
}

#[test]
fn value_round_trips_preserve_existing_pins_and_explicit_override_writes_remain_idempotent()
-> Result<()> {
    let mut f = fixture()?;
    let value = comet_scene::FieldOverride {
        entity: f.prefab_entity,
        field: schema::TRANSLATION,
        value: FieldValue::Vec3([0.; 3]),
    };
    let mut source = f.doc.source();
    source
        .instances
        .iter_mut()
        .find(|instance| instance.id == f.direct)
        .unwrap()
        .overrides
        .push(value.clone());
    f.doc = Document::open_source(source)?;
    let before = f.doc.shared_source();
    let target = FieldTarget::Prefab {
        instance: f.direct,
        entity: f.prefab_entity,
    };
    f.doc.begin()?;
    f.doc
        .preview_field_values(&[translated(target, [3.; 3])], &mut f.library)?;
    f.doc
        .preview_field_values(&[translated(target, [0.; 3])], &mut f.library)?;
    f.doc.commit()?;
    assert_eq!(f.doc.shared_source(), before);
    assert_eq!(f.doc.history_len(), 0);
    assert!(!f.doc.is_dirty());

    let mut source = f.doc.source();
    source
        .instances
        .iter_mut()
        .find(|instance| instance.id == f.direct)
        .unwrap()
        .overrides
        .clear();
    f.doc = Document::open_source(source)?;
    f.doc.begin()?;
    f.doc
        .preview_override(f.direct, value.clone(), &mut f.library)?;
    let pin = f.doc.shared_source();
    let revision = f.doc.revision();
    f.doc.preview_override(f.direct, value, &mut f.library)?;
    assert!(std::sync::Arc::ptr_eq(&pin, &f.doc.shared_source()));
    assert_eq!(f.doc.revision(), revision);
    f.doc.commit()?;
    assert_eq!(f.doc.history_len(), 1);
    f.doc.undo()?;
    let undone = f.doc.shared_source();
    f.doc.begin()?;
    f.doc
        .preview_field_values(&[translated(target, [3.; 3])], &mut f.library)?;
    f.doc
        .preview_field_values(&[translated(target, [0.; 3])], &mut f.library)?;
    f.doc.commit()?;
    assert_eq!(f.doc.shared_source(), undone);
    assert_eq!(f.doc.history_len(), 0);
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), pin);
    f.library.source.base.entities[0].transform.translation = [9.; 3];
    let expanded = f.doc.source().expand(&mut f.library)?;
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|entity| entity.id == f.prefab_entity.in_instance(f.direct))
            .unwrap()
            .transform
            .translation,
        [0.; 3]
    );
    Ok(())
}

#[test]
fn returning_values_restore_savepoint_and_history_without_normalizing_other_edits() -> Result<()> {
    use comet_scene::{BodyMotion, Character, ColliderShape, PhysicsBody};
    let mut f = fixture()?;
    let mut character = EntityData::new("character");
    character.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    character.character = Some(Character {
        player_controlled: false,
        ..Default::default()
    });
    let id = character.id;
    let mut prefab = character.clone();
    prefab.id = f.prefab_entity;
    f.library.source.base.entities = vec![prefab];
    let mut source = f.doc.source();
    source.base.entities.push(character);
    source.instances.retain(|instance| instance.id == f.direct);
    f.doc = Document::open_source(source)?;
    let before = f.doc.shared_source();
    let state = f.doc.state_id();
    let speed = |value| FieldEdit {
        target: FieldTarget::Prefab {
            instance: f.direct,
            entity: f.prefab_entity,
        },
        field: schema::CROUCH_SPEED,
        value: FieldValue::Scalar(value),
    };
    f.doc.begin()?;
    f.doc.preview_field_values(
        &[
            speed(2.5),
            translated(FieldTarget::Authored(id), [1., 0., 0.]),
        ],
        &mut f.library,
    )?;
    f.doc.preview_field_values(&[speed(2.)], &mut f.library)?;

    assert!(f.doc.is_dirty());
    f.doc.preview_field_values(
        &[translated(FieldTarget::Authored(id), [0.; 3])],
        &mut f.library,
    )?;
    assert!(std::sync::Arc::ptr_eq(&before, &f.doc.shared_source()));
    assert_eq!(f.doc.state_id(), state);
    assert!(!f.doc.is_dirty());
    f.doc.commit()?;
    assert_eq!(f.doc.history_len(), 0);
    Ok(())
}

#[test]
fn rejected_value_batches_preserve_the_accepted_preview_and_allow_a_complete_retry() -> Result<()> {
    let mut f = fixture()?;
    let target = FieldTarget::Prefab {
        instance: f.direct,
        entity: f.prefab_entity,
    };
    let before = f.doc.shared_source();
    f.doc.begin()?;
    f.doc.preview_field_values(
        &[
            translated(target, [2.; 3]),
            translated(FieldTarget::Authored(f.root), [1.; 3]),
        ],
        &mut f.library,
    )?;
    let accepted = f.doc.shared_source();
    let revision = f.doc.revision();
    let state = f.doc.state_id();
    for invalid in [
        translated(FieldTarget::Authored(f.child), [f32::NAN, 0., 0.]),
        translated(
            FieldTarget::Prefab {
                instance: f.direct,
                entity: EntityId::new(),
            },
            [0.; 3],
        ),
        FieldEdit {
            target: FieldTarget::Authored(f.child),
            field: schema::TRANSLATION,
            value: FieldValue::Scalar(1.),
        },
    ] {
        assert!(
            f.doc
                .preview_field_values(&[translated(target, [0.; 3]), invalid], &mut f.library)
                .is_err()
        );
        assert!(std::sync::Arc::ptr_eq(&accepted, &f.doc.shared_source()));
        assert_eq!(f.doc.revision(), revision);
        assert_eq!(f.doc.state_id(), state);
        assert_eq!(f.doc.history_len(), 0);
    }
    let repeated = translated(target, [0.; 3]);
    assert!(
        f.doc
            .preview_field_values(&[repeated.clone(), repeated], &mut f.library)
            .is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&accepted, &f.doc.shared_source()));
    f.library.fail = true;
    assert!(
        f.doc
            .preview_field_values(&[translated(target, [0.; 3])], &mut f.library)
            .is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&accepted, &f.doc.shared_source()));
    f.library.fail = false;
    f.doc.preview_field_values(
        &[
            translated(target, [0.; 3]),
            translated(FieldTarget::Authored(f.root), [0.; 3]),
        ],
        &mut f.library,
    )?;
    f.doc.commit()?;
    assert_eq!(f.doc.shared_source(), before);
    assert_eq!(f.doc.history_len(), 0);
    Ok(())
}

#[test]
fn prefab_value_baselines_compare_source_entity_ids_before_instance_remapping() -> Result<()> {
    use comet_scene::{BodyMotion, JointConnection, PhysicsBody, PointJoint, Script, ScriptValue};
    let mut f = fixture()?;
    let mut target_body = EntityData::new("target");
    target_body.physics = Some(PhysicsBody::default());
    let referenced = target_body.id;
    let owner = &mut f.library.source.base.entities[0];
    owner.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    owner.links.target = Some(EntityRef::local(referenced));
    owner.point_joint = Some(PointJoint {
        settings: comet_scene::JointSettings {
            connection: JointConnection::Body(EntityRef::local(referenced)),
            ..Default::default()
        },
        ..Default::default()
    });
    let mut script = Script::new(AssetId::new());
    script.parameters.insert(
        "target".into(),
        ScriptValue::Entity(Some(EntityRef::local(referenced))),
    );
    owner.script = Some(script.clone());
    f.library.source.base.entities.push(target_body);
    let mut source = f.doc.source();
    source.instances.retain(|instance| instance.id == f.direct);
    f.doc = Document::open_source(source)?;
    let before = f.doc.shared_source();
    let target = FieldTarget::Prefab {
        instance: f.direct,
        entity: f.prefab_entity,
    };
    let field = |field, value| FieldEdit {
        target,
        field,
        value,
    };
    f.doc.begin()?;
    f.doc.preview_field_values(
        &[
            field(schema::TARGET, FieldValue::Entity(None)),
            field(
                schema::SCRIPT_PARAMETERS,
                FieldValue::ScriptParameters(Default::default()),
            ),
            field(
                schema::POINT_CONNECTION,
                FieldValue::JointConnection(JointConnection::Unbound),
            ),
        ],
        &mut f.library,
    )?;
    f.doc.preview_field_values(
        &[
            field(
                schema::TARGET,
                FieldValue::Entity(Some(EntityRef::local(referenced))),
            ),
            field(
                schema::SCRIPT_PARAMETERS,
                FieldValue::ScriptParameters(script.parameters),
            ),
            field(
                schema::POINT_CONNECTION,
                FieldValue::JointConnection(JointConnection::Body(EntityRef::local(referenced))),
            ),
        ],
        &mut f.library,
    )?;
    assert_eq!(f.doc.shared_source(), before);
    f.doc.commit()?;
    assert_eq!(f.doc.history_len(), 0);
    let expanded = f.doc.source().expand(&mut f.library)?;
    let owner = expanded
        .entities
        .iter()
        .find(|entity| entity.id == f.prefab_entity.in_instance(f.direct))
        .unwrap();
    let expected = EntityRef::local(referenced.in_instance(f.direct));
    assert_eq!(owner.links.target, Some(expected.clone()));
    assert_eq!(
        owner.script.as_ref().unwrap().parameters["target"],
        ScriptValue::Entity(Some(expected.clone()))
    );
    assert_eq!(
        owner.point_joint.as_ref().unwrap().settings.connection,
        JointConnection::Body(expected)
    );
    Ok(())
}

#[test]
fn value_previews_do_not_remove_components_added_earlier_in_the_transaction() -> Result<()> {
    let mut f = fixture()?;
    f.doc.begin()?;
    f.doc.preview_script(
        f.root,
        Some(comet_scene::Script::new(AssetId::new())),
        &mut f.library,
    )?;
    for enabled in [false, true] {
        f.doc.preview_field_values(
            &[FieldEdit {
                target: FieldTarget::Authored(f.root),
                field: schema::SCRIPT_ENABLED,
                value: FieldValue::Bool(enabled),
            }],
            &mut f.library,
        )?;
        assert_eq!(
            f.doc
                .source()
                .base
                .entities
                .iter()
                .find(|entity| entity.id == f.root)
                .unwrap()
                .script
                .as_ref()
                .unwrap()
                .enabled,
            enabled
        );
    }
    assert!(f.doc.is_dirty());
    f.doc.commit()?;
    assert_eq!(f.doc.history_len(), 1);
    f.doc.undo()?;
    assert!(
        f.doc
            .source()
            .base
            .entities
            .iter()
            .find(|entity| entity.id == f.root)
            .unwrap()
            .script
            .is_none()
    );
    Ok(())
}

#[test]
fn mixed_field_preview_expands_once_and_commits_or_cancels_as_a_batch() -> Result<()> {
    let mut f = fixture()?;
    let before = f.doc.shared_source();
    let edits = [
        translated(FieldTarget::Authored(f.root), [1., 2., 3.]),
        translated(FieldTarget::Authored(f.child), [-2., 0., 4.]),
        translated(
            FieldTarget::Prefab {
                instance: f.direct,
                entity: f.prefab_entity,
            },
            [5., 6., 7.],
        ),
    ];
    assert!(f.doc.preview_fields(&edits, &mut f.library).is_err());
    f.doc.begin()?;
    f.library.calls = 0;
    f.doc.preview_fields(&edits, &mut f.library)?;
    assert_eq!(
        f.library.calls, 2,
        "each direct instance is resolved once for the complete candidate"
    );
    let after = f.doc.shared_source();
    assert_ne!(after, before);
    assert_eq!(
        after
            .base
            .entities
            .iter()
            .find(|entity| entity.id == f.root)
            .unwrap()
            .transform
            .translation,
        [1., 2., 3.]
    );
    assert_eq!(
        after
            .base
            .entities
            .iter()
            .find(|entity| entity.id == f.child)
            .unwrap()
            .transform
            .translation,
        [-2., 0., 4.]
    );
    let expanded = after.expand(&mut f.library)?;
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|entity| entity.id == f.prefab_entity.in_instance(f.direct))
            .unwrap()
            .transform
            .translation,
        [5., 6., 7.]
    );
    assert_eq!(f.doc.history_len(), 0);
    f.doc.commit()?;
    assert_eq!(f.doc.history_len(), 1);
    f.doc.undo()?;
    assert_eq!(f.doc.shared_source(), before);
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), after);
    f.doc.begin()?;
    f.doc.preview_fields(
        &[translated(FieldTarget::Authored(f.root), [20.; 3])],
        &mut f.library,
    )?;
    f.doc.cancel()?;
    assert_eq!(f.doc.shared_source(), after);
    assert_eq!(f.doc.history_len(), 1);
    Ok(())
}

#[test]
fn failed_batch_fields_preserve_the_last_accepted_preview_and_revision() -> Result<()> {
    let mut f = fixture()?;
    f.doc.begin()?;
    f.doc.preview_fields(
        &[translated(FieldTarget::Authored(f.root), [1.; 3])],
        &mut f.library,
    )?;
    let accepted = f.doc.shared_source();
    let revision = f.doc.revision();
    let first = translated(FieldTarget::Authored(f.child), [3.; 3]);
    for second in [
        translated(FieldTarget::Authored(EntityId::new()), [0.; 3]),
        translated(
            FieldTarget::Prefab {
                instance: f.direct,
                entity: EntityId::new(),
            },
            [0.; 3],
        ),
        translated(
            FieldTarget::Prefab {
                instance: InstanceId::new(),
                entity: f.prefab_entity,
            },
            [0.; 3],
        ),
        translated(FieldTarget::Authored(f.root), [f32::NAN, 0., 0.]),
    ] {
        assert!(
            f.doc
                .preview_fields(&[first.clone(), second], &mut f.library)
                .is_err()
        );
        assert!(std::sync::Arc::ptr_eq(&accepted, &f.doc.shared_source()));
        assert_eq!(f.doc.revision(), revision);
        assert_eq!(f.doc.history_len(), 0);
    }
    assert!(
        f.doc
            .preview_fields(&[first.clone(), first], &mut f.library)
            .is_err()
    );
    f.library.fail = true;
    assert!(
        f.doc
            .preview_fields(
                &[translated(FieldTarget::Authored(f.root), [5.; 3])],
                &mut f.library
            )
            .is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&accepted, &f.doc.shared_source()));
    f.doc.cancel()?;
    assert!(!f.doc.is_dirty());
    Ok(())
}

#[test]
fn batch_crouch_fields_publish_authored_and_prefab_changes_together() -> Result<()> {
    use comet_scene::{BodyMotion, Character, ColliderShape, PhysicsBody};
    let mut f = fixture()?;
    let mut entity = EntityData::new("character");
    entity.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    entity.character = Some(Character {
        player_controlled: false,
        ..Default::default()
    });
    let id = entity.id;
    let mut prefab = entity.clone();
    prefab.id = f.prefab_entity;
    f.library.source.base.entities = vec![prefab];
    let mut source = f.doc.source();
    source.base.entities.push(entity);
    source.instances.retain(|instance| instance.id == f.direct);
    f.doc = Document::open_source(source)?;
    let before = f.doc.shared_source();
    f.doc.begin()?;
    f.doc.preview_fields(
        &[
            FieldEdit {
                target: FieldTarget::Authored(id),
                field: schema::CROUCH_HEIGHT_RATIO,
                value: FieldValue::Scalar(0.8),
            },
            FieldEdit {
                target: FieldTarget::Prefab {
                    instance: f.direct,
                    entity: f.prefab_entity,
                },
                field: schema::CROUCH_SPEED,
                value: FieldValue::Scalar(2.5),
            },
        ],
        &mut f.library,
    )?;
    f.doc.commit()?;
    let after = f.doc.shared_source();
    f.doc.undo()?;
    assert_eq!(f.doc.shared_source(), before);
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), after);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("batch.scene");
    f.doc.save(&path)?;
    assert_eq!(Document::load(&path)?.shared_source(), after);
    Ok(())
}

#[test]
fn baseline_field_preview_restores_omitted_authored_values_and_prefab_override_declarations()
-> Result<()> {
    let mut f = fixture()?;
    let before = f.doc.shared_source();
    assert!(
        f.doc
            .preview_fields_from_baseline(&[], &mut f.library)
            .is_err()
    );
    f.doc.begin()?;
    f.doc.preview_fields_from_baseline(
        &[
            translated(FieldTarget::Authored(f.root), [1., 2., 3.]),
            translated(
                FieldTarget::Prefab {
                    instance: f.direct,
                    entity: f.prefab_entity,
                },
                [4., 5., 6.],
            ),
        ],
        &mut f.library,
    )?;
    assert_ne!(f.doc.shared_source(), before);
    let edits = [FieldEdit {
        target: FieldTarget::Authored(f.child),
        field: schema::ROTATION,
        value: FieldValue::Quat(comet_core::math::Quat::from_rotation_y(0.3).to_array()),
    }];
    f.library.calls = 0;
    f.doc.preview_fields_from_baseline(&edits, &mut f.library)?;
    assert_eq!(f.library.calls, 2);
    let accepted = f.doc.shared_source();
    assert_eq!(accepted.instances, before.instances);
    assert_eq!(
        accepted.base.entities.iter().find(|e| e.id == f.root),
        before.base.entities.iter().find(|e| e.id == f.root)
    );
    let revision = f.doc.revision();
    assert!(
        f.doc
            .preview_fields_from_baseline(
                &[translated(FieldTarget::Authored(EntityId::new()), [1.; 3])],
                &mut f.library
            )
            .is_err()
    );
    assert_eq!(f.doc.shared_source(), accepted);
    assert_eq!(f.doc.revision(), revision);
    f.doc.preview_fields_from_baseline(&[], &mut f.library)?;
    assert_eq!(f.doc.shared_source(), before);
    f.doc.commit()?;
    assert_eq!(f.doc.history_len(), 0);
    assert!(!f.doc.is_dirty());
    Ok(())
}

#[test]
fn placement_keeps_world_intervals_and_per_copy_local_and_instance_references_in_one_history()
-> Result<()> {
    use comet_core::math::{Mat4, Quat, Vec3};
    let mut f = fixture()?;
    let mut source = f.doc.source();
    let mut parent = EntityData::new("rotated mirrored parent");
    parent.transform.translation = [8., 4., -3.];
    parent.transform.rotation = Quat::from_rotation_y(0.7).to_array();
    parent.transform.scale = [-2., 3., 0.5];
    let parent_id = parent.id;
    source.base.entities.push(parent);
    let root = source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.root)
        .unwrap();
    root.parent = Some(parent_id);
    root.transform.translation = [1., 2., 3.];
    root.links.target = Some(EntityRef::in_instance(f.attached, f.prefab_entity));
    f.doc = Document::open_source(source)?;
    let before = f.doc.shared_source();
    let world_id = f.doc.world_id();
    let old_world = comet_scene::world_transforms(&f.doc.expand(&mut f.library)?)?;
    let maps = f.doc.place_copies(
        &[f.root, f.child, f.observer, f.root],
        3,
        [2., -1., 3.],
        &mut f.library,
    )?;
    let after = f.doc.shared_source();
    let new_world = comet_scene::world_transforms(&f.doc.expand(&mut f.library)?)?;
    assert_eq!(f.doc.world_id(), world_id);
    assert_eq!(f.doc.history_len(), 1);
    assert_eq!(maps.len(), 3);
    assert_eq!(after.base.entities.len(), before.base.entities.len() + 9);
    assert_eq!(after.instances.len(), before.instances.len() + 3);
    let original_entities: std::collections::BTreeSet<_> = before
        .base
        .entities
        .iter()
        .map(|entity| entity.id)
        .collect();
    let mut copy_ids = std::collections::BTreeSet::new();
    for (index, map) in maps.iter().enumerate() {
        assert_eq!(map.entities.len(), 3);
        assert_eq!(map.instances.len(), 1);
        for (&old, &new) in &map.entities {
            assert!(!original_entities.contains(&new) && copy_ids.insert(new));
            let expected = Mat4::from_translation(Vec3::new(2., -1., 3.) * (index + 1) as f32)
                * old_world[&old];
            assert!(new_world[&new].abs_diff_eq(expected, 2e-5));
        }
        let root = after
            .base
            .entities
            .iter()
            .find(|entity| entity.id == map.entities[&f.root])
            .unwrap();
        assert_eq!(root.parent, Some(parent_id));
        assert_eq!(
            root.links.target,
            Some(EntityRef::in_instance(
                map.instances[&f.attached],
                f.prefab_entity
            ))
        );
        let child = after
            .base
            .entities
            .iter()
            .find(|entity| entity.id == map.entities[&f.child])
            .unwrap();
        assert_eq!(child.parent, Some(root.id));
        assert_eq!(child.links.target, Some(EntityRef::local(root.id)));
        let observer = after
            .base
            .entities
            .iter()
            .find(|entity| entity.id == map.entities[&f.observer])
            .unwrap();
        assert_eq!(observer.links.target, Some(EntityRef::local(root.id)));
        assert_eq!(observer.links.connections[0], EntityRef::local(child.id));
        assert_eq!(
            observer.links.connections[1],
            before
                .base
                .entities
                .iter()
                .find(|entity| entity.id == f.observer)
                .unwrap()
                .links
                .connections[1]
        );
        let instance = after
            .instances
            .iter()
            .find(|instance| instance.id == map.instances[&f.attached])
            .unwrap();
        assert_eq!(instance.parent, Some(child.id));
        assert_eq!(instance.prefab, f.library.asset);
        assert!(
            new_world[&f.prefab_entity.in_instance(instance.id)].abs_diff_eq(
                Mat4::from_translation(Vec3::new(2., -1., 3.) * (index + 1) as f32)
                    * old_world[&f.prefab_entity.in_instance(f.attached)],
                2e-5
            )
        );
    }
    for entity in &before.base.entities {
        assert_eq!(
            after
                .base
                .entities
                .iter()
                .find(|current| current.id == entity.id),
            Some(entity)
        );
    }
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("placed.scene");
    f.doc.save(&path)?;
    assert_eq!(
        Document::load(&path)?.expand(&mut f.library)?,
        f.doc.expand(&mut f.library)?
    );
    f.doc.undo()?;
    assert_eq!(f.doc.shared_source(), before);
    assert!(f.doc.is_dirty());
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), after);
    assert!(!f.doc.is_dirty());
    Ok(())
}

#[test]
fn invalid_placement_keeps_preview_savepoint_and_redo_and_zero_spacing_needs_no_parent_inverse()
-> Result<()> {
    use std::sync::Arc;
    let mut f = fixture()?;
    f.doc.clone_subtrees(&[f.root], &mut f.library)?;
    let redo = f.doc.shared_source();
    f.doc.undo()?;
    let before = f.doc.shared_source();
    let revision = f.doc.revision();
    for (copies, step) in [
        (0, [0.; 3]),
        (1001, [0.; 3]),
        (1, [f32::NAN; 3]),
        (1, [f32::INFINITY; 3]),
        (3, [f32::MAX; 3]),
    ] {
        assert!(
            f.doc
                .place_copies(&[f.root], copies, step, &mut f.library)
                .is_err()
        );
        assert!(Arc::ptr_eq(&before, &f.doc.shared_source()));
        assert_eq!(f.doc.revision(), revision);
        assert_eq!(f.doc.history_len(), 0);
        assert!(!f.doc.is_dirty());
    }
    f.library.fail = true;
    assert!(
        f.doc
            .place_copies(&[f.root], 3, [2., 0., 0.], &mut f.library)
            .is_err()
    );
    f.library.fail = false;
    assert!(Arc::ptr_eq(&before, &f.doc.shared_source()));
    f.doc.begin()?;
    f.doc.preview_name(f.root, "accepted preview".into())?;
    let preview = f.doc.shared_source();
    assert!(
        f.doc
            .place_copies(&[f.root], 3, [2., 0., 0.], &mut f.library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&preview, &f.doc.shared_source()));
    assert!(f.doc.is_editing());
    f.doc.cancel()?;
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), redo);

    for scale in [0., 1e30] {
        let mut source = before.as_ref().clone();
        let parent = source
            .base
            .entities
            .iter_mut()
            .find(|entity| entity.id == f.root)
            .unwrap();
        parent.transform.scale = [scale; 3];
        let mut doc = Document::open_source(source)?;
        let accepted = doc.shared_source();
        let step = if scale == 0. {
            [1., 0., 0.]
        } else {
            [1e-30, 0., 0.]
        };
        assert!(
            doc.place_copies(&[f.child], 2, step, &mut f.library)
                .is_err()
        );
        assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
        let maps = doc.place_copies(&[f.child], 1000, [0.; 3], &mut f.library)?;
        assert_eq!(maps.len(), 1000);
        assert_eq!(doc.history_len(), 1);
    }
    Ok(())
}
