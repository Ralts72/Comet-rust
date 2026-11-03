use anyhow::Result;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{
    ContentSource, EntityData, EntityRef, FieldOverride, Model, PrefabInstance, PrefabResolver,
    Scene, SceneSnapshot, Surface,
    schema::{self, FieldValue},
};
use std::collections::BTreeMap;

fn fixture() -> SceneSnapshot {
    let mut root = EntityData::new("root");
    let mut child = EntityData::new("child");
    let outside = EntityData::new("outside");
    child.parent = Some(root.id);
    root.links.target = Some(EntityRef::local(child.id));
    root.links.connections = vec![
        EntityRef::local(outside.id),
        EntityRef::External {
            scene: AssetId::new(),
            entity: EntityId::new(),
        },
    ];
    root.model = Some(Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    SceneSnapshot {
        version: 1,
        entities: vec![root, child, outside],
    }
}
#[test]
fn clone_hierarchy_remaps_local_refs_and_preserves_other_identities() -> Result<()> {
    let mut s = fixture();
    let root = s.entities[0].clone();
    let child = s.entities[1].id;
    let map = s.clone_subtrees(&[root.id, child])?; // Overlapping roots must copy each entity once.
    assert_eq!(map.len(), 2);
    let copied = s.entities.iter().find(|e| e.id == map[&root.id]).unwrap();
    assert_eq!(copied.links.target, Some(EntityRef::local(map[&child])));
    assert_eq!(copied.links.connections, root.links.connections);
    assert_eq!(copied.model, root.model);
    assert_eq!(
        s.entities
            .iter()
            .find(|e| e.id == map[&child])
            .unwrap()
            .parent,
        Some(map[&root.id])
    );
    assert_eq!(s.entities.len(), 5);
    Ok(())
}
#[test]
fn remove_subtree_clears_local_references_and_preserves_external_references() -> Result<()> {
    let mut scene = fixture();
    let root = scene.entities[0].id;
    let child = scene.entities[1].id;
    let outside = scene.entities[2].id;
    let external = EntityRef::External {
        scene: AssetId::new(),
        entity: child,
    };
    scene.entities[2].links.target = Some(EntityRef::local(child));
    scene.entities[2].links.connections = vec![EntityRef::local(root), external.clone()];
    let before = scene.clone();
    assert!(scene.remove_subtrees(&[EntityId::new()]).is_err());
    assert_eq!(scene, before);
    let removed = scene.remove_subtrees(&[root])?;
    assert_eq!(removed.len(), 2);
    assert!(removed.contains(&child));
    let survivor = scene.entities.iter().find(|e| e.id == outside).unwrap();
    assert_eq!(survivor.links.target, None);
    assert_eq!(survivor.links.connections, vec![external]);
    Ok(())
}
#[test]
fn invalid_batches_and_hierarchy_edits_are_atomic() -> Result<()> {
    let mut s = fixture();
    let before = s.clone();
    let root = s.entities[0].id;
    let child = s.entities[1].id;
    assert!(
        s.set_field(
            &[root, EntityId::new()],
            schema::ROUGHNESS,
            FieldValue::Scalar(0.2)
        )
        .is_err()
    );
    assert_eq!(s, before);
    for v in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(
            s.set_field(&[root], schema::ROUGHNESS, FieldValue::Scalar(v))
                .is_err()
        );
        assert_eq!(s, before);
    }
    assert!(s.reparent(root, Some(child)).is_err());
    assert_eq!(s, before);
    assert!(s.reparent(root, Some(EntityId::new())).is_err());
    assert_eq!(s, before);
    assert!(s.clone_subtrees(&[root, root]).is_err());
    assert_eq!(s, before);
    s.reparent(child, None)?;
    assert_eq!(s.entities[1].parent, None);
    Ok(())
}
#[test]
fn serialized_content_rejects_corruption_and_preserves_save_order() -> Result<()> {
    let s = fixture();
    let bytes = s.to_json()?;
    let mut reversed = s.clone();
    reversed.entities.reverse();
    assert_eq!(bytes, reversed.to_json()?);
    let restored = SceneSnapshot::from_json(&bytes)?;
    assert_eq!(restored.to_json()?, bytes);
    let mut bad = s.clone();
    bad.version = 99;
    assert!(bad.validate().is_err());
    bad = s.clone();
    bad.entities[1].id = bad.entities[0].id;
    assert!(bad.validate().is_err());
    bad = s.clone();
    bad.entities[0].links.target = Some(EntityRef::local(EntityId::new()));
    assert!(bad.validate().is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
    value["extra"] = true.into();
    assert!(SceneSnapshot::from_json(&serde_json::to_vec(&value)?).is_err());
    Ok(())
}
#[test]
fn handles_are_scoped_to_world_and_replacement_generation() -> Result<()> {
    let s = fixture();
    let id = s.entities[0].id;
    let mut first = Scene::open(s.clone())?;
    let mut second = Scene::open(s.clone())?;
    let h = first.handle(id)?;
    assert_eq!(first.resolve(h)?.id, id);
    assert!(second.resolve(h).is_err());
    let mut bad = s.clone();
    bad.version = 0;
    assert!(first.replace(bad).is_err());
    assert!(first.resolve(h).is_ok());
    first.replace(s)?;
    assert!(first.resolve(h).is_err());
    assert_eq!(first.handle(id).and_then(|h| first.resolve(h))?.id, id);
    Ok(())
}
struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.0
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing {id}"))
    }
}
#[test]
fn nested_prefab_overrides_and_source_update_keep_stable_instance_ids() -> Result<()> {
    let leaf_id = AssetId::new();
    let branch_id = AssetId::new();
    let leaf = fixture();
    let root = leaf.entities[0].id;
    let material = leaf.entities[0].model.as_ref().unwrap().surface.material;
    let anchor = EntityData::new("branch");
    let anchor_id = anchor.id;
    let inner = InstanceId::new();
    let outer = InstanceId::new();
    let instance = PrefabInstance {
        id: inner,
        prefab: leaf_id,
        parent: Some(anchor_id),
        overrides: vec![FieldOverride {
            entity: root,
            field: schema::ROUGHNESS,
            value: FieldValue::Scalar(0.2),
        }],
    };
    let branch = ContentSource {
        version: 1,
        base: SceneSnapshot {
            version: 1,
            entities: vec![anchor],
        },
        instances: vec![instance],
    };
    let source = ContentSource {
        instances: vec![PrefabInstance {
            id: outer,
            prefab: branch_id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    let mut library = Library(BTreeMap::from([
        (
            leaf_id,
            ContentSource {
                base: leaf,
                ..Default::default()
            },
        ),
        (branch_id, branch),
    ]));
    let before = source.expand(&mut library)?;
    let expected = root.in_instance(inner.in_instance(outer));
    let e = before.entities.iter().find(|e| e.id == expected).unwrap();
    assert_eq!(e.parent, Some(anchor_id.in_instance(outer)));
    assert_eq!(e.model.as_ref().unwrap().surface.roughness, 0.2);
    assert_eq!(e.model.as_ref().unwrap().surface.material, material);
    let leaf = library.0.get_mut(&leaf_id).unwrap();
    leaf.base.entities[0].name = "updated".into();
    leaf.base.entities[0]
        .model
        .as_mut()
        .unwrap()
        .surface
        .roughness = 0.9;
    let after = source.expand(&mut library)?;
    let e = after.entities.iter().find(|e| e.id == expected).unwrap();
    assert_eq!(e.name, "updated");
    assert_eq!(e.model.as_ref().unwrap().surface.roughness, 0.2);
    assert_eq!(
        before.entities.iter().map(|e| e.id).collect::<Vec<_>>(),
        after.entities.iter().map(|e| e.id).collect::<Vec<_>>()
    );
    Ok(())
}
#[test]
fn prefab_cycles_missing_products_and_stale_overrides_fail() -> Result<()> {
    let id = AssetId::new();
    let instance = PrefabInstance {
        id: InstanceId::new(),
        prefab: id,
        parent: None,
        overrides: vec![],
    };
    let mut source = ContentSource {
        instances: vec![instance],
        ..Default::default()
    };
    let mut library = Library(BTreeMap::new());
    assert!(source.expand(&mut library).is_err());
    library.0.insert(id, source.clone());
    assert!(
        source
            .expand(&mut library)
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    library.0.insert(id, ContentSource::default());
    source.instances[0].overrides.push(FieldOverride {
        entity: EntityId::new(),
        field: schema::TRANSLATION,
        value: FieldValue::Vec3([1.; 3]),
    });
    assert!(source.expand(&mut library).is_err());
    Ok(())
}
