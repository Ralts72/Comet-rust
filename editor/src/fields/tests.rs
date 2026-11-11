use super::*;
use comet_scene::{ContentSource, EntityData, PrefabInstance, PrefabResolver, SceneSnapshot};

#[test]
fn failed_field_edits_close_only_new_transactions_and_keep_an_accepted_gesture() -> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let entity = EntityData::new("field transaction");
    let id = entity.id;
    let mut source = editor.document.source();
    source.base.entities.push(entity);
    editor.document = Document::open_source(source)?;
    let original = editor.document.shared_source();
    assert!(
        editor
            .edit(
                id,
                schema::TRANSLATION,
                FieldValue::Vec3([f32::NAN, 0., 0.]),
                &mut assets
            )
            .is_err()
    );
    assert!(!editor.document.is_editing());
    assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
    editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3([1., 2., 3.]),
        &mut assets,
    )?;
    let accepted = editor.document.shared_source();
    assert!(
        editor
            .edit(
                id,
                schema::TRANSLATION,
                FieldValue::Vec3([f32::NAN, 0., 0.]),
                &mut assets
            )
            .is_err()
    );
    assert!(editor.document.is_editing());
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
    Ok(())
}

#[test]
fn grouped_values_share_one_transaction_and_keep_prefab_identity_and_failed_candidates_atomic()
-> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let mut source = editor.document.source();
    let authored = EntityData::new("group author");
    let id = authored.id;
    source.base.entities.push(authored);
    let instance = source.instances[0].clone();
    let prefab = assets.prefab(instance.prefab)?;
    let node = prefab.base.entities[0].id;
    let expanded = node.in_instance(instance.id);
    editor.document = Document::open_source(source)?;
    let original = editor.document.shared_source();
    let saved = std::fs::read(&editor.source)?;
    let edits = [
        FieldUpdate {
            entity: id,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([4., 2., 3.]),
        },
        FieldUpdate {
            entity: expanded,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([4., 5., 6.]),
        },
    ];
    editor.edit_fields(&edits, &mut assets)?;
    let accepted = editor.document.shared_source();
    assert!(editor.document.is_editing());
    assert_eq!(
        accepted
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .transform
            .translation,
        [4., 2., 3.]
    );
    let declaration = accepted
        .instances
        .iter()
        .find(|value| value.id == instance.id)
        .unwrap();
    let value = declaration
        .overrides
        .iter()
        .find(|value| value.entity == node && value.field == schema::TRANSLATION)
        .unwrap();
    assert_eq!(value.value, FieldValue::Vec3([4., 5., 6.]));
    assert_eq!(assets.prefab(instance.prefab)?, prefab);
    let mut bad = edits.clone();
    bad[0].value = FieldValue::Vec3([99.; 3]);
    bad[1].value = FieldValue::Vec3([f32::NAN; 3]);
    assert!(editor.edit_fields(&bad, &mut assets).is_err());
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert!(editor.document.is_editing());
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
    editor.document.redo()?;
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert_eq!(std::fs::read(&editor.source)?, saved);
    editor.save(&mut assets)?;
    assert_eq!(Document::load(&editor.source)?.shared_source(), accepted);
    Ok(())
}

#[test]
fn a_nested_member_rejects_the_whole_group_before_opening_a_transaction() -> Result<()> {
    use comet_asset_pipeline::{SourceMeta, prepare};
    use comet_core::InstanceId;
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let mut source = editor.document.source();
    let inner = source.instances[0].clone();
    let node = assets.prefab(inner.prefab)?.base.entities[0].id;
    let nested_path = editor.project_root().join("assets/group-nested.prefab");
    std::fs::write(
        &nested_path,
        ContentSource {
            instances: vec![inner.clone()],
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    let outer = InstanceId::new();
    source.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: outer,
        prefab: SourceMeta::load(&nested_path)?.id,
        parent: None,
        overrides: vec![],
    });
    let author = EntityData::new("editable member");
    let id = author.id;
    source.base.entities.push(author);
    editor.document = Document::open_source(source)?;
    let nested = node.in_instance(inner.id.in_instance(outer));
    assert!(
        editor
            .snapshot(&mut assets)?
            .entities
            .iter()
            .any(|entity| entity.id == nested)
    );
    let original = editor.document.shared_source();
    let updates = [id, nested].map(|entity| FieldUpdate {
        entity,
        field: schema::TRANSLATION,
        value: FieldValue::Vec3([8.; 3]),
    });
    assert!(editor.edit_fields(&updates, &mut assets).is_err());
    assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
    assert!(!editor.document.is_editing());
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn returning_all_group_values_to_their_baselines_preserves_existing_pins_and_inheritance()
-> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let mut source = editor.document.source();
    let first = source.instances[0].clone();
    let node = assets.prefab(first.prefab)?.base.entities[0].clone();
    let value = node.transform.translation;
    source.instances[0]
        .overrides
        .push(comet_scene::FieldOverride {
            entity: node.id,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3(value),
        });
    let second = comet_core::InstanceId::new();
    source.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: second,
        prefab: first.prefab,
        parent: None,
        overrides: vec![],
    });
    editor.document = Document::open_source(source)?;
    let original = editor.document.shared_source();
    let state = editor.document.state_id();
    let mut updates = [first.id, second].map(|instance| FieldUpdate {
        entity: node.id.in_instance(instance),
        field: schema::TRANSLATION,
        value: FieldValue::Vec3([9., 8., 7.]),
    });
    editor.edit_fields(&updates, &mut assets)?;
    for update in &mut updates {
        update.value = FieldValue::Vec3(value);
    }
    editor.edit_fields(&updates, &mut assets)?;
    editor.commit()?;
    assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
    assert_eq!(editor.document.state_id(), state);
    assert_eq!(editor.document.history_len(), 0);
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn reference_routes_share_one_cache_and_keep_instance_and_animator_source_identities_separate()
-> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let prefab = crate::test_support::prepare_skin_asset(&editor)?;
    assets.refresh()?;
    let product = assets.prefab(prefab)?;
    let controller = product
        .base
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .unwrap()
        .id;
    let owner = EntityData::new("author");
    let other = EntityData::new("other");
    let first = comet_core::InstanceId::new();
    let second = comet_core::InstanceId::new();
    editor.document = comet_editor_core::Document::open_source(ContentSource {
        base: SceneSnapshot {
            entities: vec![owner.clone(), other.clone()],
            ..Default::default()
        },
        instances: [first, second]
            .into_iter()
            .map(|id| PrefabInstance {
                translation: [0.; 3],
                id,
                prefab,
                parent: None,
                overrides: vec![],
            })
            .collect(),
        ..Default::default()
    })?;
    let ctx = egui::Context::default();
    let cache = editor.cached_field_references(&ctx, owner.id, &mut assets)?;
    let mut value = FieldValue::AnimatorBindings(BTreeMap::from([
        (
            controller,
            Some(EntityRef::local(controller.in_instance(first))),
        ),
        (other.id, Some(EntityRef::local(other.id))),
    ]));
    cache.write_value(&mut value)?;
    let FieldValue::AnimatorBindings(bindings) = value else {
        unreachable!()
    };
    assert_eq!(
        bindings[&controller],
        Some(EntityRef::InInstance {
            instance: first,
            entity: controller
        })
    );
    assert_eq!(bindings[&other.id], Some(EntityRef::local(other.id)));
    for id in assets.index().products.keys().copied().collect::<Vec<_>>() {
        assets.evict(id);
    }
    let again = editor.cached_field_references(&ctx, owner.id, &mut assets)?;
    assert!(Arc::ptr_eq(&cache, &again));
    assert_eq!(assets.resident_count(), 0);
    let other_cache = editor.cached_field_references(&ctx, other.id, &mut assets)?;
    assert!(!Arc::ptr_eq(&cache, &other_cache));
    let replaced = editor.cached_field_references(&ctx, owner.id, &mut assets)?;
    assert!(
        !Arc::ptr_eq(&cache, &replaced),
        "one replaceable entry, not one table per owner"
    );

    let own = editor.field_references(
        FieldTarget::Prefab {
            instance: first,
            entity: controller,
        },
        &mut assets,
    )?;
    let mut local = FieldValue::Entity(Some(EntityRef::local(controller.in_instance(first))));
    own.write_value(&mut local)?;
    assert_eq!(
        local,
        FieldValue::Entity(Some(EntityRef::local(controller)))
    );
    let mut sibling = FieldValue::Entity(Some(EntityRef::local(controller.in_instance(second))));
    let before = sibling.clone();
    assert!(own.write_value(&mut sibling).is_err());
    assert_eq!(sibling, before);
    let mut author = FieldValue::Entity(Some(EntityRef::local(owner.id)));
    assert!(own.write_value(&mut author).is_err());
    editor.start(&mut assets)?;
    assert!(
        editor
            .cached_field_references(&ctx, owner.id, &mut assets)
            .is_err()
    );
    editor.stop();
    Ok(())
}
