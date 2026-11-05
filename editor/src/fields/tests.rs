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
