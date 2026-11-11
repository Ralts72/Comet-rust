use anyhow::{Result, ensure};
use comet_core::{AssetId, InstanceId};
use comet_editor_core::Document;
use comet_scene::{
    ContentSource, EntityData, EntityRef, FieldOverride, PrefabInstance, PrefabResolver,
    SceneSnapshot,
    schema::{self, FieldValue},
};
use std::{collections::BTreeMap, sync::Arc};

struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.0
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing prefab {id}"))
    }
}

#[test]
fn capture_keeps_local_root_pose_identity_nested_scopes_and_current_preview_without_touching_history()
-> Result<()> {
    let mut parent = EntityData::new("outside parent");
    parent.transform.translation = [20., 30., 40.];
    parent.transform.scale = [-2., 3., 0.5];
    let mut root = EntityData::new("captured root");
    root.parent = Some(parent.id);
    root.transform.translation = [3., 4., 5.];
    let mut child = EntityData::new("captured child");
    child.parent = Some(root.id);
    child.links.target = Some(EntityRef::local(root.id));
    let mut leaf = EntityData::new("linked leaf");
    // A source UUID may equal an author UUID without sharing its namespace.
    leaf.id = root.id;
    leaf.links.target = Some(EntityRef::local(leaf.id));
    let asset = AssetId::new();
    let instance = PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab: asset,
        parent: Some(child.id),
        overrides: vec![FieldOverride {
            entity: leaf.id,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([7., 0., 0.]),
        }],
    };
    root.links.target = Some(EntityRef::in_instance(instance.id, leaf.id));
    // Incoming links do not make the captured subtree depend on the outside parent.
    parent.links.target = Some(EntityRef::local(child.id));
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![parent.clone(), root.clone(), child.clone()],
            ..Default::default()
        },
        instances: vec![instance.clone()],
        ..Default::default()
    };
    let mut library = Library(BTreeMap::from([(
        asset,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![leaf],
                ..Default::default()
            },
            ..Default::default()
        },
    )]));
    let library_before = library.0.clone();
    let mut document = Document::open_source(source)?;
    document.clone_subtrees(&[root.id], &mut library)?;
    let redo = document.shared_source();
    document.undo()?;
    document.begin()?;
    document.preview_name(root.id, "accepted preview".into())?;
    let accepted = document.shared_source();
    let identity = (
        document.world_id(),
        document.revision(),
        document.state_id(),
        document.history_len(),
    );
    let content = document.capture_prefab(root.id, &mut library)?;
    assert!(Arc::ptr_eq(&accepted, &document.shared_source()));
    assert_eq!(
        (
            document.world_id(),
            document.revision(),
            document.state_id(),
            document.history_len()
        ),
        identity
    );
    assert!(document.is_editing() && document.is_dirty());
    let captured = content
        .base
        .entities
        .iter()
        .find(|entity| entity.id == root.id)
        .unwrap();
    root.parent = None;
    root.name = "accepted preview".into();
    assert_eq!(captured, &root);
    assert_eq!(
        content
            .base
            .entities
            .iter()
            .find(|entity| entity.id == child.id),
        Some(&child)
    );
    assert_eq!(content.base.entities.len(), 2);
    assert_eq!(
        content.instances.as_slice(),
        std::slice::from_ref(&instance)
    );
    assert_eq!(content.base.version, accepted.base.version);
    assert_eq!(content.version, accepted.version);
    let snapshot = content.expand(&mut library)?;
    assert!(
        snapshot
            .entities
            .iter()
            .any(|entity| entity.id == root.id.in_instance(instance.id))
    );
    assert_eq!(library.0, library_before);
    assert_eq!(ContentSource::from_json(&content.to_json()?)?, content);
    document.cancel()?;
    document.redo()?;
    assert_eq!(document.shared_source(), redo);
    Ok(())
}

#[test]
fn capture_rejects_outgoing_local_instance_external_and_current_root_cycles_without_mutating_the_document()
-> Result<()> {
    let mut root = EntityData::new("root");
    let outside = EntityData::new("outside");
    let leaf = EntityData::new("outside instance leaf");
    let asset = AssetId::new();
    let instance = PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab: asset,
        parent: None,
        overrides: vec![],
    };
    let library_source = ContentSource {
        base: SceneSnapshot {
            entities: vec![leaf.clone()],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut library = Library(BTreeMap::from([(asset, library_source)]));
    for target in [
        EntityRef::local(outside.id),
        EntityRef::in_instance(instance.id, leaf.id),
        EntityRef::External {
            scene: AssetId::new(),
            entity: outside.id,
        },
    ] {
        root.links.target = Some(target);
        let mut document = Document::open_source(ContentSource {
            base: SceneSnapshot {
                entities: vec![root.clone(), outside.clone()],
                ..Default::default()
            },
            instances: vec![instance.clone()],
            ..Default::default()
        })?;
        document.begin()?;
        document.preview_name(root.id, "accepted".into())?;
        let accepted = document.shared_source();
        assert!(document.capture_prefab(root.id, &mut library).is_err());
        assert!(Arc::ptr_eq(&accepted, &document.shared_source()));
        assert!(document.is_editing() && document.is_dirty());
        assert_eq!(document.history_len(), 0);
    }
    root.links.target = None;
    let root_asset = AssetId::new();
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![root.clone()],
            ..Default::default()
        },
        instances: vec![instance],
        ..Default::default()
    };
    library.0.insert(root_asset, ContentSource::default());
    let document = Document::open_prefab(root_asset, source, &mut library)?;
    library
        .0
        .get_mut(&asset)
        .unwrap()
        .instances
        .push(PrefabInstance {
            translation: [0.; 3],
            id: InstanceId::new(),
            prefab: root_asset,
            parent: None,
            overrides: vec![],
        });
    let accepted = document.shared_source();
    ensure!(
        accepted.expand(&mut library).is_ok(),
        "old product masks the owning prefab cycle"
    );
    let error = document.capture_prefab(root.id, &mut library).unwrap_err();
    assert!(format!("{error:#}").contains("cycle"));
    assert!(Arc::ptr_eq(&accepted, &document.shared_source()));
    Ok(())
}
