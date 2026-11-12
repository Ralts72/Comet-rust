use anyhow::{Context, Result};
use comet_core::{AssetId, InstanceId};
use comet_editor_core::Document;
use comet_scene::{
    BodyMotion, ContentSource, EntityData, PhysicsBody, PrefabInstance, PrefabResolver, Scene,
    SceneSnapshot,
};
use std::{collections::BTreeMap, fs, sync::Arc};

struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.0.get(&id).cloned().context("missing prefab")
    }
}
fn content(entities: Vec<EntityData>) -> ContentSource {
    ContentSource {
        base: SceneSnapshot {
            entities,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn body(name: &str, motion: BodyMotion) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.physics = Some(PhysicsBody {
        motion,
        ..Default::default()
    });
    entity
}
fn nested(prefab: AssetId) -> ContentSource {
    ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: InstanceId::new(),
            prefab,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    }
}

#[test]
fn unparented_single_multiple_and_nested_physics_roots_form_one_reversible_insertion() -> Result<()>
{
    let single = AssetId::new();
    let multiple = AssetId::new();
    let middle = AssetId::new();
    let deep = AssetId::new();
    let mut library = Library(BTreeMap::from([
        (single, content(vec![body("single", BodyMotion::Fixed)])),
        (
            multiple,
            content(vec![
                body("fixed", BodyMotion::Fixed),
                body("dynamic", BodyMotion::Dynamic),
            ]),
        ),
        (middle, nested(multiple)),
        (deep, nested(middle)),
    ]));
    let temp = tempfile::tempdir()?;
    for (index, prefab, count) in [(0, single, 1), (1, multiple, 2), (2, deep, 2)] {
        let author = EntityData::new("existing author");
        let baseline = content(vec![author]);
        let mut document = Document::open_source(baseline.clone())?;
        let world = document.world_id();
        let before = document.shared_source();
        let descriptors = library.prefab(prefab)?.prefab_roots(&mut library)?;
        assert_eq!(descriptors.len(), count);
        assert!(descriptors.iter().all(|root| root.requires_scene_root()));
        let instance = document.insert_prefab(None, prefab, [0.; 3], &mut library)?;
        let inserted = document.shared_source();
        assert_eq!(inserted.base, baseline.base);
        assert_eq!(inserted.instances.len(), 1);
        assert_eq!(inserted.instances[0].id, instance);
        assert_eq!(inserted.instances[0].parent, None);
        assert_eq!(document.history_len(), 1);
        assert!(document.is_dirty());
        assert_eq!(document.world_id(), world);
        let expanded = document.expand(&mut library)?;
        assert_eq!(expanded.entities.len(), count + 1);
        for root in &descriptors {
            let entity = expanded
                .entities
                .iter()
                .find(|entity| entity.id == root.in_instance(instance))
                .unwrap();
            assert!(entity.physics.is_some() && entity.parent.is_none());
        }
        let mut scene = Scene::open(expanded.clone())?;
        assert_eq!(*scene.shared_snapshot(), expanded);
        document.undo()?;
        assert!(Arc::ptr_eq(&before, &document.shared_source()));
        assert_eq!(document.history_len(), 0);
        assert!(!document.is_dirty());
        document.redo()?;
        assert!(Arc::ptr_eq(&inserted, &document.shared_source()));
        assert_eq!(document.expand(&mut library)?, expanded);
        let path = temp.path().join(format!("placed-{index}.scene"));
        document.save(&path)?;
        let restored = ContentSource::from_json(&fs::read(path)?)?;
        assert_eq!(restored.instances[0].id, instance);
        assert_eq!(restored.instances[0].parent, None);
        assert_eq!(restored.expand(&mut library)?, expanded);
    }
    Ok(())
}

#[test]
fn authored_container_keeps_nonphysical_multi_roots_in_one_insertion() -> Result<()> {
    let prefab = AssetId::new();
    let first = EntityData::new("first");
    let second = EntityData::new("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut library = Library(BTreeMap::from([(prefab, content(vec![first, second]))]));
    let mut document = Document::open_source(ContentSource::default())?;
    let container = EntityData::new("container");
    let container_id = container.id;
    let instance = document.insert_prefab(Some(container), prefab, [0.; 3], &mut library)?;
    let inserted = document.shared_source();
    assert_eq!(inserted.base.entities.len(), 1);
    assert_eq!(inserted.instances[0].id, instance);
    assert_eq!(inserted.instances[0].parent, Some(container_id));
    let expanded = document.expand(&mut library)?;
    for id in [first_id, second_id] {
        assert_eq!(
            expanded
                .entities
                .iter()
                .find(|entity| entity.id == id.in_instance(instance))
                .unwrap()
                .parent,
            Some(container_id)
        );
    }
    assert_eq!(document.history_len(), 1);
    document.undo()?;
    assert!(document.source().base.entities.is_empty());
    assert!(document.source().instances.is_empty());
    assert!(!document.is_dirty());
    document.redo()?;
    assert!(Arc::ptr_eq(&inserted, &document.shared_source()));
    Ok(())
}

#[test]
fn rejected_placement_preserves_savepoint_redo_and_an_active_preview() -> Result<()> {
    let prefab = AssetId::new();
    let mut library = Library(BTreeMap::from([(
        prefab,
        content(vec![body("root body", BodyMotion::Dynamic)]),
    )]));
    let entity = EntityData::new("author");
    let id = entity.id;
    let mut document = Document::open_source(content(vec![entity]))?;
    document.begin()?;
    document.preview_name(id, "redo name".into())?;
    document.commit()?;
    let redo = document.shared_source();
    document.undo()?;
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("author.scene");
    document.save(&path)?;
    let saved = fs::read(&path)?;
    let before = document.shared_source();
    let revision = document.revision();
    let state = document.state_id();
    for (container, target) in [
        (Some(EntityData::new("invalid physics container")), prefab),
        (None, AssetId::new()),
    ] {
        assert!(
            document
                .insert_prefab(container, target, [0.; 3], &mut library)
                .is_err()
        );
        assert!(Arc::ptr_eq(&before, &document.shared_source()));
        assert_eq!(
            (document.revision(), document.state_id()),
            (revision, state)
        );
        assert_eq!(document.history_len(), 0);
        assert!(!document.is_dirty() && !document.is_editing());
        assert_eq!(fs::read(&path)?, saved);
    }
    document.begin()?;
    document.preview_name(id, "accepted preview".into())?;
    let preview = document.shared_source();
    let preview_revision = document.revision();
    assert!(
        document
            .insert_prefab(None, prefab, [0.; 3], &mut library)
            .is_err()
    );
    assert!(document.is_editing());
    assert!(Arc::ptr_eq(&preview, &document.shared_source()));
    assert_eq!(document.revision(), preview_revision);
    assert_eq!(document.history_len(), 0);
    document.cancel()?;
    assert!(Arc::ptr_eq(&before, &document.shared_source()));
    document.redo()?;
    assert!(Arc::ptr_eq(&redo, &document.shared_source()));
    Ok(())
}

#[test]
fn translated_physical_instances_roundtrip_copy_and_reject_failed_candidates_atomically()
-> Result<()> {
    let leaf = AssetId::new();
    let branch = AssetId::new();
    let mut first = body("first", BodyMotion::Fixed);
    first.transform.translation = [1., 5., 0.];
    let mut second = body("second", BodyMotion::Dynamic);
    second.transform.translation = [-1., 5., 0.];
    let mut library = Library(BTreeMap::from([
        (leaf, content(vec![first, second])),
        (branch, nested(leaf)),
    ]));
    let mut document = Document::open(Default::default())?;
    let initial = document.shared_source();
    let id = document.insert_prefab(None, branch, [10., 2., 3.], &mut library)?;
    let placed = document.shared_source();
    assert_eq!(placed.version, comet_scene::prefab::CONTENT_VERSION);
    assert!(placed.base.entities.is_empty());
    assert_eq!(placed.instances[0].translation, [10., 2., 3.]);
    let expanded = document.expand(&mut library)?;
    assert!(
        expanded
            .entities
            .iter()
            .all(|e| e.parent.is_none() && e.transform.translation[1] == 7.)
    );
    assert_eq!(document.history_len(), 1);
    let copy = document.duplicate_instance(id, &mut library)?;
    assert_ne!(copy, id);
    assert_eq!(
        document
            .shared_source()
            .instances
            .iter()
            .find(|i| i.id == copy)
            .unwrap()
            .translation,
        [10., 2., 3.]
    );
    document.undo()?;
    assert_eq!(document.shared_source(), placed);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("translated.scene");
    document.save(&path)?;
    assert_eq!(Document::load(&path)?.expand(&mut library)?, expanded);
    document.undo()?;
    assert_eq!(document.shared_source(), initial);
    for translation in [[f32::NAN, 0., 0.], [f32::MAX, f32::MAX, f32::MAX]] {
        // A missing product and a non-finite offset both preserve the redo branch.
        let prefab = if translation[0].is_finite() {
            AssetId::new()
        } else {
            branch
        };
        assert!(
            document
                .insert_prefab(None, prefab, translation, &mut library)
                .is_err()
        );
        assert!(Arc::ptr_eq(&initial, &document.shared_source()));
        assert_eq!(document.history_len(), 0);
    }
    document.redo()?;
    assert_eq!(document.shared_source(), placed);
    document.detach_instance(id, &mut library)?;
    assert_eq!(document.expand(&mut library)?, expanded);
    Ok(())
}

#[test]
fn mixed_batch_placement_keeps_nested_links_world_intervals_and_one_saved_history() -> Result<()> {
    use comet_core::math::{Mat4, Quat, Vec3};
    use comet_editor_core::ObjectTarget;
    use comet_scene::{
        EntityRef, FieldOverride,
        schema::{self, FieldValue},
    };

    let leaf = AssetId::new();
    let physical = AssetId::new();
    let wrapper = AssetId::new();
    let mut first = EntityData::new("first source root");
    first.transform.translation = [1., 2., 3.];
    let mut second = EntityData::new("second source root");
    second.transform.translation = [-2., 1., 0.];
    first.links.target = Some(EntityRef::local(second.id));
    let leaf_id = first.id;
    let mut physical_first = body("physical first", BodyMotion::Fixed);
    let physical_second = body("physical second", BodyMotion::Dynamic);
    physical_first.links.target = Some(EntityRef::local(physical_second.id));
    let physical_ids = [physical_first.id, physical_second.id];
    let nested_source = nested(physical);
    let inner = nested_source.instances[0].id;
    let mut library = Library(BTreeMap::from([
        (leaf, content(vec![first, second])),
        (physical, content(vec![physical_first, physical_second])),
        (wrapper, nested_source),
    ]));
    let sources = library.0.clone();
    let mut parent = EntityData::new("rotated mirrored attachment");
    parent.transform.translation = [8., 4., -3.];
    parent.transform.rotation = Quat::from_rotation_y(0.7).to_array();
    parent.transform.scale = [-2., 3., 0.5];
    let mut author = EntityData::new("author root");
    author.parent = Some(parent.id);
    author.transform.translation = [1., 2., 3.];
    let mut child = EntityData::new("author child");
    child.parent = Some(author.id);
    child.links.target = Some(EntityRef::local(author.id));
    let mut observer = EntityData::new("observer");
    let attached = InstanceId::new();
    let direct = InstanceId::new();
    let unparented = InstanceId::new();
    author.links.target = Some(EntityRef::in_instance(attached, leaf_id));
    observer.links.target = Some(EntityRef::in_instance(direct, leaf_id));
    let (parent_id, author_id, child_id, observer_id) =
        (parent.id, author.id, child.id, observer.id);
    let mut document = Document::open_source(ContentSource {
        base: SceneSnapshot {
            entities: vec![parent, author, child, observer],
            ..Default::default()
        },
        instances: vec![
            PrefabInstance {
                id: attached,
                prefab: leaf,
                parent: Some(child_id),
                translation: [0.5, 1., 2.],
                overrides: vec![FieldOverride {
                    entity: leaf_id,
                    field: schema::TRANSLATION,
                    value: FieldValue::Vec3([4., 0., 0.]),
                }],
            },
            PrefabInstance {
                id: direct,
                prefab: leaf,
                parent: Some(parent_id),
                translation: [3., 1., 0.],
                overrides: vec![],
            },
            PrefabInstance {
                id: unparented,
                prefab: wrapper,
                parent: None,
                translation: [7., 1., 0.],
                overrides: vec![],
            },
        ],
        ..Default::default()
    })?;
    let before = document.shared_source();
    let world_id = document.world_id();
    let original = comet_scene::world_transforms(&document.expand(&mut library)?)?;
    let maps = document.place_copies(
        &[
            ObjectTarget::Authored(author_id),
            ObjectTarget::Authored(child_id),
            ObjectTarget::Authored(observer_id),
            ObjectTarget::Prefab(attached),
            ObjectTarget::Prefab(direct),
            ObjectTarget::Prefab(unparented),
            ObjectTarget::Prefab(unparented),
        ],
        2,
        [2., -1., 3.],
        &mut library,
    )?;
    let after = document.shared_source();
    let expanded = document.expand(&mut library)?;
    let placed = comet_scene::world_transforms(&expanded)?;
    assert_eq!(document.world_id(), world_id);
    assert_eq!(document.history_len(), 1);
    assert_eq!(after.base.entities.len(), before.base.entities.len() + 6);
    assert_eq!(after.instances.len(), before.instances.len() + 6);
    for (index, map) in maps.iter().enumerate() {
        let shift = Mat4::from_translation(Vec3::new(2., -1., 3.) * (index + 1) as f32);
        assert_eq!(map.entities.len(), 3);
        assert_eq!(map.instances.len(), 3);
        for (&old, &new) in &map.entities {
            assert!(placed[&new].abs_diff_eq(shift * original[&old], 3e-5));
        }
        for (&old, &new) in &map.instances {
            let source = before.instances.iter().find(|i| i.id == old).unwrap();
            let copy = after.instances.iter().find(|i| i.id == new).unwrap();
            assert_eq!(copy.prefab, source.prefab);
            assert_eq!(copy.overrides, source.overrides);
            assert_eq!(
                copy.parent,
                source
                    .parent
                    .map(|id| map.entities.get(&id).copied().unwrap_or(id))
            );
            if old == attached {
                assert_eq!(copy.translation, source.translation);
            }
            for root in library.prefab(source.prefab)?.prefab_roots(&mut library)? {
                assert!(
                    placed[&root.in_instance(new)]
                        .abs_diff_eq(shift * original[&root.in_instance(old)], 3e-5)
                );
            }
        }
        let author = after
            .base
            .entities
            .iter()
            .find(|e| e.id == map.entities[&author_id])
            .unwrap();
        assert_eq!(
            author.links.target,
            Some(EntityRef::in_instance(map.instances[&attached], leaf_id))
        );
        let observer = after
            .base
            .entities
            .iter()
            .find(|e| e.id == map.entities[&observer_id])
            .unwrap();
        assert_eq!(
            observer.links.target,
            Some(EntityRef::in_instance(map.instances[&direct], leaf_id))
        );
        let scope = inner.in_instance(map.instances[&unparented]);
        let physical = expanded
            .entities
            .iter()
            .find(|e| e.id == physical_ids[0].in_instance(scope))
            .unwrap();
        assert!(physical.parent.is_none());
        assert_eq!(
            physical.links.target,
            Some(EntityRef::local(physical_ids[1].in_instance(scope)))
        );
    }
    for entity in &before.base.entities {
        assert_eq!(
            after.base.entities.iter().find(|e| e.id == entity.id),
            Some(entity)
        );
    }
    for instance in &before.instances {
        assert_eq!(
            after.instances.iter().find(|i| i.id == instance.id),
            Some(instance)
        );
    }
    assert_eq!(library.0, sources);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("batch.scene");
    document.save(&path)?;
    assert_eq!(Document::load(&path)?.expand(&mut library)?, expanded);
    document.undo()?;
    assert!(Arc::ptr_eq(&before, &document.shared_source()));
    assert!(document.is_dirty());
    document.redo()?;
    assert!(Arc::ptr_eq(&after, &document.shared_source()));
    assert!(!document.is_dirty());
    Ok(())
}

#[test]
fn instance_batches_reject_unrepresentable_roots_and_unique_components_without_losing_state()
-> Result<()> {
    use comet_editor_core::ObjectTarget;
    for unique_region in [false, true] {
        let prefab = AssetId::new();
        let mut root = EntityData::new("source root");
        if unique_region {
            root.navigation_region = Some(Default::default());
        } else {
            root.transform.translation = [1e20, 0., 0.];
        }
        let mut library = Library(BTreeMap::from([(
            prefab,
            content(vec![root, EntityData::new("small root")]),
        )]));
        let anchor = EntityData::new("author");
        let anchor_id = anchor.id;
        let mut document = Document::open_source(content(vec![anchor]))?;
        let instance = document.insert_prefab(None, prefab, [0.; 3], &mut library)?;
        document.begin()?;
        document.preview_name(anchor_id, "redo name".into())?;
        document.commit()?;
        let redo = document.shared_source();
        document.undo()?;
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("stable.scene");
        document.save(&path)?;
        let bytes = fs::read(&path)?;
        let before = document.shared_source();
        let state = (
            document.revision(),
            document.state_id(),
            document.history_len(),
        );
        let result = document.place_copies(
            &[ObjectTarget::Prefab(instance)],
            2,
            [1., 0., 0.],
            &mut library,
        );
        let error = format!("{:#}", result.unwrap_err());
        assert!(
            error.contains(if unique_region {
                "multiple navigation regions"
            } else {
                "precision"
            }),
            "{error}"
        );
        assert!(Arc::ptr_eq(&before, &document.shared_source()));
        assert_eq!(
            (
                document.revision(),
                document.state_id(),
                document.history_len()
            ),
            state
        );
        assert!(!document.is_dirty());
        assert_eq!(fs::read(&path)?, bytes);
        document.begin()?;
        document.preview_name(anchor_id, "valid preview".into())?;
        let preview = document.shared_source();
        assert!(
            document
                .place_copies(
                    &[ObjectTarget::Prefab(instance)],
                    1,
                    [2., 0., 0.],
                    &mut library
                )
                .is_err()
        );
        assert!(Arc::ptr_eq(&preview, &document.shared_source()));
        document.cancel()?;
        document.redo()?;
        assert!(Arc::ptr_eq(&redo, &document.shared_source()));
        document.undo()?;
        let result =
            document.place_copies(&[ObjectTarget::Prefab(instance)], 2, [0.; 3], &mut library);
        if unique_region {
            assert!(result.is_err());
            assert!(Arc::ptr_eq(&before, &document.shared_source()));
        } else {
            assert_eq!(result?.len(), 2);
            assert_eq!(document.shared_source().instances.len(), 3);
        }
    }
    Ok(())
}
