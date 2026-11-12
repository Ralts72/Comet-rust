use anyhow::{Context, Result};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::{Document, ObjectTarget};
use comet_scene::{
    ContentSource, EntityData, EntityRef, FieldOverride, PhysicsBody, PrefabInstance,
    PrefabResolver, SceneSnapshot,
    schema::{self, FieldValue},
};
use std::{collections::BTreeMap, fs, sync::Arc};

#[derive(Default)]
struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.0.get(&id).cloned().context("missing prefab")
    }
}
fn content(entities: Vec<EntityData>, instances: Vec<PrefabInstance>) -> ContentSource {
    ContentSource {
        base: SceneSnapshot {
            entities,
            ..Default::default()
        },
        instances,
        ..Default::default()
    }
}
fn instance(prefab: AssetId, parent: Option<EntityId>) -> PrefabInstance {
    PrefabInstance {
        id: InstanceId::new(),
        prefab,
        parent,
        translation: [3., 0., 0.],
        overrides: vec![],
    }
}

#[test]
fn captured_mixed_groups_survive_source_deletion_and_paste_with_independent_scopes() -> Result<()> {
    let leaf = AssetId::new();
    let wrapper = AssetId::new();
    let mut first = EntityData::new("source first");
    let second = EntityData::new("source second");
    first.links.target = Some(EntityRef::local(second.id));
    let leaf_ids = [first.id, second.id];
    let nested = instance(leaf, None);
    let inner = nested.id;
    let mut library = Library(BTreeMap::from([
        (leaf, content(vec![first, second], vec![])),
        (wrapper, content(vec![], vec![nested])),
    ]));
    let sources = library.0.clone();
    let mut parent = EntityData::new("original parent");
    parent.transform.translation = [10., 2., 0.];
    let mut root = EntityData::new("copied author");
    root.parent = Some(parent.id);
    root.transform.translation = [1., 2., 3.];
    let mut child = EntityData::new("child");
    child.parent = Some(root.id);
    child.links.target = Some(EntityRef::local(root.id));
    let mut attached = instance(leaf, Some(child.id));
    attached.overrides.push(FieldOverride {
        entity: leaf_ids[0],
        field: schema::TARGET,
        value: FieldValue::Entity(Some(EntityRef::local(leaf_ids[1]))),
    });
    let direct = instance(wrapper, Some(parent.id));
    root.links.target = Some(EntityRef::in_instance(attached.id, leaf_ids[0]));
    let (parent_id, root_id, child_id, attached_id, direct_id) =
        (parent.id, root.id, child.id, attached.id, direct.id);
    let targets = [
        ObjectTarget::Authored(root_id),
        ObjectTarget::Authored(child_id),
        ObjectTarget::Prefab(attached_id),
        ObjectTarget::Prefab(direct_id),
        ObjectTarget::Prefab(direct_id),
    ];
    let mut original = Document::open_source(content(
        vec![parent, root, child],
        vec![attached.clone(), direct],
    ))?;
    let before = original.shared_source();
    let snapshot = original.capture_selection(&targets, &mut library)?;
    assert!(Arc::ptr_eq(&before, &original.shared_source()));
    assert_eq!(
        (
            original.history_len(),
            original.revision(),
            original.state_id()
        ),
        (0, 0, 0)
    );
    original.remove_selection(&targets, &mut library)?;
    assert_eq!(original.shared_source().base.entities.len(), 1);
    assert!(original.shared_source().instances.is_empty());
    let map = original.paste_selection(&snapshot, Some(parent_id), &mut library)?;
    assert_eq!((map.entities.len(), map.instances.len()), (2, 2));
    assert_eq!(original.history_len(), 2);

    let mut destination = EntityData::new("new parent");
    destination.transform.translation = [-5., 1., 0.];
    let destination_id = destination.id;
    let mut document = Document::open_source(content(vec![destination], vec![]))?;
    let world = document.world_id();
    let initial = document.shared_source();
    for pass in 0..2 {
        let map = document.paste_selection(&snapshot, Some(destination_id), &mut library)?;
        assert_eq!((map.entities.len(), map.instances.len()), (2, 2));
        let source = document.shared_source();
        let author = source
            .base
            .entities
            .iter()
            .find(|e| e.id == map.entities[&root_id])
            .unwrap();
        assert_eq!(author.parent, Some(destination_id));
        assert_eq!(author.transform.translation, [1., 2., 3.]);
        assert_eq!(
            author.links.target,
            Some(EntityRef::in_instance(
                map.instances[&attached_id],
                leaf_ids[0]
            ))
        );
        let child = source
            .base
            .entities
            .iter()
            .find(|e| e.id == map.entities[&child_id])
            .unwrap();
        assert_eq!(child.parent, Some(author.id));
        assert_eq!(child.links.target, Some(EntityRef::local(author.id)));
        let copied = source
            .instances
            .iter()
            .find(|i| i.id == map.instances[&attached_id])
            .unwrap();
        assert_eq!(copied.parent, Some(child.id));
        assert_eq!(copied.overrides, attached.overrides);
        assert_eq!(copied.translation, attached.translation);
        let scope = inner.in_instance(map.instances[&direct_id]);
        let expanded = document.expand(&mut library)?;
        let first = expanded
            .entities
            .iter()
            .find(|e| e.id == leaf_ids[0].in_instance(scope))
            .unwrap();
        assert_eq!(first.parent, Some(destination_id));
        assert_eq!(
            first.links.target,
            Some(EntityRef::local(leaf_ids[1].in_instance(scope)))
        );
        assert_eq!(document.history_len(), pass + 1);
    }
    let after = document.shared_source();
    let expanded = document.expand(&mut library)?;
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("pasted.scene");
    document.save(&path)?;
    assert_eq!(Document::load(&path)?.expand(&mut library)?, expanded);
    document.undo()?;
    document.undo()?;
    assert!(Arc::ptr_eq(&initial, &document.shared_source()));
    assert!(document.is_dirty());
    document.redo()?;
    document.redo()?;
    assert!(Arc::ptr_eq(&after, &document.shared_source()));
    assert!(!document.is_dirty());
    assert_eq!(document.world_id(), world);
    assert_eq!(library.0, sources);
    Ok(())
}

#[test]
fn outside_local_and_instance_references_require_the_original_live_world() -> Result<()> {
    let asset = AssetId::new();
    let leaf = EntityData::new("leaf");
    let leaf_id = leaf.id;
    let mut library = Library(BTreeMap::from([(asset, content(vec![leaf], vec![]))]));
    let declaration = instance(asset, None);
    let declaration_id = declaration.id;
    let outside = EntityData::new("outside");
    let outside_id = outside.id;
    let mut root = EntityData::new("root");
    root.links.target = Some(EntityRef::local(outside_id));
    root.links
        .connections
        .push(EntityRef::in_instance(declaration_id, leaf_id));
    let root_id = root.id;
    let source = content(vec![outside, root], vec![declaration]);
    let mut document = Document::open_source(source.clone())?;
    let clipboard = document.capture_selection(&[ObjectTarget::Authored(root_id)], &mut library)?;
    let map = document.paste_selection(&clipboard, None, &mut library)?;
    let pasted = document.shared_source();
    let copy = pasted
        .base
        .entities
        .iter()
        .find(|e| e.id == map.entities[&root_id])
        .unwrap();
    assert_eq!(copy.links.target, Some(EntityRef::local(outside_id)));
    assert_eq!(
        copy.links.connections,
        [EntityRef::in_instance(declaration_id, leaf_id)]
    );
    // Even coincidental UUID matches in another document must not bind to unrelated live objects.
    let mut other = Document::open_source(source)?;
    let before = other.shared_source();
    assert!(
        other
            .paste_selection(&clipboard, None, &mut library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &other.shared_source()));
    document.remove_selection(
        &[
            ObjectTarget::Authored(outside_id),
            ObjectTarget::Prefab(declaration_id),
        ],
        &mut library,
    )?;
    let before = document.shared_source();
    let revision = document.revision();
    assert!(
        document
            .paste_selection(&clipboard, None, &mut library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &document.shared_source()));
    assert_eq!(document.revision(), revision);
    document.undo()?;
    document.paste_selection(&clipboard, None, &mut library)?;
    Ok(())
}

#[test]
fn failed_pastes_preserve_preview_saved_bytes_and_redo_and_allow_valid_retry() -> Result<()> {
    let asset = AssetId::new();
    let mut root = EntityData::new("body");
    root.physics = Some(PhysicsBody::default());
    let mut library = Library(BTreeMap::from([(asset, content(vec![root], vec![]))]));
    let declaration = instance(asset, None);
    let declaration_id = declaration.id;
    let origin = Document::open_source(content(vec![], vec![declaration]))?;
    let clipboard =
        origin.capture_selection(&[ObjectTarget::Prefab(declaration_id)], &mut library)?;
    let anchor = EntityData::new("anchor");
    let anchor_id = anchor.id;
    let mut document = Document::open_source(content(vec![anchor], vec![]))?;
    document.begin()?;
    document.preview_name(anchor_id, "redo".into())?;
    document.commit()?;
    let redo = document.shared_source();
    document.undo()?;
    let before = document.shared_source();
    let state = (
        document.state_id(),
        document.revision(),
        document.history_len(),
    );
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("saved.scene");
    document.save(&path)?;
    let bytes = fs::read(&path)?;
    for parent in [Some(anchor_id), Some(EntityId::new())] {
        assert!(
            document
                .paste_selection(&clipboard, parent, &mut library)
                .is_err()
        );
        assert!(Arc::ptr_eq(&before, &document.shared_source()));
        assert_eq!(
            (
                document.state_id(),
                document.revision(),
                document.history_len()
            ),
            state
        );
        assert!(!document.is_dirty());
    }
    let source = library.0.remove(&asset).unwrap();
    assert!(
        document
            .paste_selection(&clipboard, None, &mut library)
            .is_err()
    );
    library.0.insert(asset, source);
    document.begin()?;
    document.preview_name(anchor_id, "preview".into())?;
    let preview = document.shared_source();
    assert!(
        document
            .paste_selection(&clipboard, None, &mut library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&preview, &document.shared_source()));
    assert!(
        document
            .capture_selection(&[ObjectTarget::Authored(anchor_id)], &mut library)
            .is_err()
    );
    document.cancel()?;
    assert_eq!(fs::read(path)?, bytes);
    document.redo()?;
    assert!(Arc::ptr_eq(&redo, &document.shared_source()));
    document.undo()?;
    document.paste_selection(&clipboard, None, &mut library)?;
    let pasted = document.expand(&mut library)?;
    assert_eq!(
        pasted
            .entities
            .iter()
            .filter(|e| e.physics.is_some() && e.parent.is_none())
            .count(),
        1
    );
    // Source ownership context must reject pasting a prefab into its own definition.
    let mut prefab = Document::open_prefab(asset, library.prefab(asset)?, &mut library)?;
    assert!(
        prefab
            .paste_selection(&clipboard, None, &mut library)
            .is_err()
    );
    assert_eq!(prefab.history_len(), 0);
    // A unique region can paste into an empty document, but a second paste is atomic failure.
    let mut region = EntityData::new("region");
    region.navigation_region = Some(Default::default());
    let id = region.id;
    let origin = Document::open_source(content(vec![region], vec![]))?;
    let clipboard = origin.capture_selection(&[ObjectTarget::Authored(id)], &mut library)?;
    document.paste_selection(&clipboard, None, &mut library)?;
    let before = document.shared_source();
    assert!(
        document
            .paste_selection(&clipboard, None, &mut library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &document.shared_source()));
    Ok(())
}
