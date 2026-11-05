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
        let instance = document.insert_prefab(None, prefab, &mut library)?;
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
    let instance = document.insert_prefab(Some(container), prefab, &mut library)?;
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
                .insert_prefab(container, target, &mut library)
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
    assert!(document.insert_prefab(None, prefab, &mut library).is_err());
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
