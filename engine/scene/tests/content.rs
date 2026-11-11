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
    let mut live = Scene::open(before.clone())?;
    let survivor_handle = live.handle(outside)?;
    let removed_handle = live.handle(child)?;
    let frozen = live.shared_snapshot();
    assert!(live.remove_subtrees(&[root, EntityId::new()]).is_err());
    assert!(std::sync::Arc::ptr_eq(&frozen, &live.shared_snapshot()));
    assert_eq!(live.remove_subtrees(&[root, child])?.len(), 2);
    assert_eq!(live.resolve(survivor_handle)?.links.target, None);
    assert!(live.resolve(removed_handle).is_err());
    assert_eq!(frozen.entities.len(), 3);
    assert!(scene.remove_subtrees(&[EntityId::new()]).is_err());
    assert_eq!(scene, before);
    let removed = scene.remove_subtrees(&[root])?;
    assert_eq!(removed.len(), 2);
    assert!(removed.contains(&child));
    let survivor = scene.entities.iter().find(|e| e.id == outside).unwrap();
    assert_eq!(survivor.links.target, None);
    assert_eq!(survivor.links.connections, vec![external]);
    assert_eq!(live.snapshot(), scene);
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
        translation: [0.; 3],
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
            translation: [0.; 3],
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
        translation: [0.; 3],
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
    assert!(source.prefab_roots(&mut library).is_err());
    library.0.insert(id, source.clone());
    assert!(
        source
            .expand(&mut library)
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    assert!(source.prefab_roots(&mut library).is_err());
    library.0.insert(id, ContentSource::default());
    source.instances[0].overrides.push(FieldOverride {
        entity: EntityId::new(),
        field: schema::TRANSLATION,
        value: FieldValue::Vec3([1.; 3]),
    });
    assert!(source.expand(&mut library).is_err());
    assert!(source.prefab_roots(&mut library).is_err());
    Ok(())
}

#[test]
fn prefab_roots_match_deep_expansion_and_keep_their_anchor_when_sources_are_reordered() -> Result<()>
{
    let leaf_asset = AssetId::new();
    let middle_asset = AssetId::new();
    let branch_asset = AssetId::new();
    let source_asset = AssetId::new();
    let instance = |prefab, parent| PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab,
        parent,
        overrides: vec![],
    };
    let leaf_first = EntityData::new("leaf first");
    let leaf_second = EntityData::new("leaf second");
    let mut leaf_child = EntityData::new("leaf child");
    leaf_child.parent = Some(leaf_first.id);
    let leaf_id = leaf_first.id;
    let leaf = ContentSource {
        base: SceneSnapshot {
            entities: vec![leaf_first, leaf_child, leaf_second],
            ..Default::default()
        },
        ..Default::default()
    };
    let inner = instance(leaf_asset, None);
    let inner_id = inner.id;
    let middle = ContentSource {
        instances: vec![inner],
        ..Default::default()
    };
    let anchor = EntityData::new("branch anchor");
    let free_middle = instance(middle_asset, None);
    let middle_id = free_middle.id;
    let attached_leaf = instance(leaf_asset, Some(anchor.id));
    let branch = ContentSource {
        base: SceneSnapshot {
            entities: vec![anchor],
            ..Default::default()
        },
        instances: vec![free_middle, attached_leaf],
        ..Default::default()
    };
    let first = EntityData::new("source first");
    let second = EntityData::new("source second");
    let free_branch = instance(branch_asset, None);
    let branch_id = free_branch.id;
    let attached_branch = instance(branch_asset, Some(first.id));
    let mut source = ContentSource {
        base: SceneSnapshot {
            entities: vec![first, second],
            ..Default::default()
        },
        instances: vec![free_branch, attached_branch, instance(leaf_asset, None)],
        ..Default::default()
    };
    let mut library = Library(BTreeMap::from([
        (leaf_asset, leaf),
        (middle_asset, middle),
        (branch_asset, branch),
        (source_asset, source.clone()),
    ]));
    let descriptors = source.prefab_roots(&mut library)?;
    assert_eq!(descriptors.len(), 7);
    assert!(descriptors.iter().all(|root| !root.requires_scene_root()));
    for outer in [InstanceId::new(), InstanceId::new()] {
        let wrapper = ContentSource {
            instances: vec![PrefabInstance {
                translation: [0.; 3],
                id: outer,
                prefab: source_asset,
                parent: None,
                overrides: vec![],
            }],
            ..Default::default()
        };
        let expanded = wrapper.expand(&mut library)?;
        let expected: std::collections::BTreeSet<_> = expanded
            .entities
            .iter()
            .filter(|entity| entity.parent.is_none())
            .map(|entity| entity.id)
            .collect();
        let actual = descriptors
            .iter()
            .map(|root| root.in_instance(outer))
            .collect();
        assert_eq!(expected, actual);
        let deep = leaf_id
            .in_instance(inner_id.in_instance(middle_id.in_instance(branch_id.in_instance(outer))));
        assert!(expected.contains(&deep));
        assert_ne!(
            deep,
            leaf_id
                .in_instance(inner_id)
                .in_instance(middle_id)
                .in_instance(branch_id)
                .in_instance(outer)
        );
        assert!(expected.contains(&descriptors[0].in_instance(outer)));
    }
    source.base.entities.reverse();
    source.instances.reverse();
    for prefab in library.0.values_mut() {
        prefab.base.entities.reverse();
        prefab.instances.reverse();
    }
    assert_eq!(descriptors, source.prefab_roots(&mut library)?);
    Ok(())
}

#[test]
fn prefab_root_discovery_validates_attached_declarations_and_the_existing_depth_boundary()
-> Result<()> {
    let asset = AssetId::new();
    let anchor = EntityData::new("anchor");
    let mut source = ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: InstanceId::new(),
            prefab: asset,
            parent: Some(anchor.id),
            overrides: vec![],
        }],
        base: SceneSnapshot {
            entities: vec![anchor],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut library = Library(BTreeMap::from([(asset, ContentSource::default())]));
    assert_eq!(source.prefab_roots(&mut library)?.len(), 1);
    source.instances[0].overrides.push(FieldOverride {
        entity: EntityId::new(),
        field: schema::TRANSLATION,
        value: FieldValue::Vec3([0.; 3]),
    });
    assert!(source.prefab_roots(&mut library).is_err());
    source.instances[0].overrides.clear();
    source.instances.push(source.instances[0].clone());
    assert!(source.prefab_roots(&mut library).is_err());

    let mut chain = ContentSource::default();
    for depth in 0..64 {
        let child = AssetId::new();
        library.0.insert(child, chain);
        chain = ContentSource {
            instances: vec![PrefabInstance {
                translation: [0.; 3],
                id: InstanceId::new(),
                prefab: child,
                parent: None,
                overrides: vec![],
            }],
            ..Default::default()
        };
        if depth == 62 {
            assert!(chain.prefab_roots(&mut library)?.is_empty());
        }
    }
    let error = chain.prefab_roots(&mut library).unwrap_err();
    assert!(format!("{error:#}").contains("64 levels"));
    Ok(())
}

#[test]
fn prefab_root_scope_rejects_direct_and_indirect_self_use_hidden_by_an_old_product() -> Result<()> {
    let root = AssetId::new();
    let branch = AssetId::new();
    let mut old = ContentSource::default();
    old.base.entities.push(EntityData::new("old source root"));
    let instance = |prefab| PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab,
        parent: None,
        overrides: vec![],
    };
    let mut library = Library(BTreeMap::from([
        (root, old.clone()),
        (
            branch,
            ContentSource {
                instances: vec![instance(root)],
                ..Default::default()
            },
        ),
    ]));
    for target in [root, branch] {
        let mut candidate = old.clone();
        candidate.instances.push(instance(target));
        let bytes = candidate.to_json()?;
        assert!(
            candidate.expand(&mut library).is_ok(),
            "an anonymous scene can use the old product"
        );
        let error = candidate.expand_prefab(root, &mut library).unwrap_err();
        assert!(format!("{error:#}").contains("cycle"));
        assert_eq!(candidate.to_json()?, bytes);
    }
    Ok(())
}

#[test]
fn prefab_root_scope_preserves_multiple_roots_and_nested_instance_identity() -> Result<()> {
    let root = AssetId::new();
    let child = AssetId::new();
    let first = EntityData::new("first root");
    let second = EntityData::new("second root");
    let nested = EntityData::new("nested root");
    let (first_id, second_id, nested_id) = (first.id, second.id, nested.id);
    let instances = [InstanceId::new(), InstanceId::new()];
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![first, second],
            ..Default::default()
        },
        instances: instances
            .into_iter()
            .zip([first_id, second_id])
            .map(|(id, parent)| PrefabInstance {
                translation: [0.; 3],
                id,
                prefab: child,
                parent: Some(parent),
                overrides: vec![],
            })
            .collect(),
        ..Default::default()
    };
    let mut library = Library(BTreeMap::from([(
        child,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![nested],
                ..Default::default()
            },
            ..Default::default()
        },
    )]));
    let before = source.to_json()?;
    let expanded = source.expand_prefab(root, &mut library)?;
    assert_eq!(expanded, source.expand(&mut library)?);
    assert_eq!(expanded.entities.len(), 4);
    for id in [first_id, second_id] {
        assert!(
            expanded
                .entities
                .iter()
                .any(|entity| entity.id == id && entity.parent.is_none())
        );
    }
    for (instance, parent) in instances.into_iter().zip([first_id, second_id]) {
        let entity = expanded
            .entities
            .iter()
            .find(|entity| entity.id == nested_id.in_instance(instance))
            .unwrap();
        assert_eq!(entity.parent, Some(parent));
    }
    assert_eq!(source.to_json()?, before);
    Ok(())
}

#[test]
fn instance_translation_versions_roots_and_detachment_preserve_scopes() -> Result<()> {
    let leaf = AssetId::new();
    let branch = AssetId::new();
    let mut root = EntityData::new("root");
    root.transform.translation = [1., 2., 3.];
    root.transform.scale = [2.; 3];
    let mut child = EntityData::new("child");
    child.parent = Some(root.id);
    child.transform.translation = [1., 0., 0.];
    child.links.target = Some(EntityRef::local(root.id));
    let leaf_source = ContentSource {
        base: SceneSnapshot {
            entities: vec![root.clone(), child],
            ..Default::default()
        },
        ..Default::default()
    };
    let direct = InstanceId::new();
    let attached = InstanceId::new();
    let mut mount = EntityData::new("mount");
    mount.transform.translation = [0., 4., 0.];
    let branch_source = ContentSource {
        version: 2,
        base: SceneSnapshot {
            entities: vec![mount.clone()],
            ..Default::default()
        },
        instances: vec![
            PrefabInstance {
                id: direct,
                prefab: leaf,
                parent: None,
                translation: [2., 0., 0.],
                overrides: vec![],
            },
            PrefabInstance {
                id: attached,
                prefab: leaf,
                parent: Some(mount.id),
                translation: [0., 1., 0.],
                overrides: vec![],
            },
        ],
    };
    let mut library = Library(BTreeMap::from([
        (leaf, leaf_source),
        (branch, branch_source),
    ]));
    let outer = InstanceId::new();
    let source = ContentSource {
        version: 2,
        instances: vec![PrefabInstance {
            id: outer,
            prefab: branch,
            parent: None,
            translation: [10., 20., 30.],
            overrides: vec![],
        }],
        ..Default::default()
    };
    let expanded = source.expand(&mut library)?;
    let worlds = comet_scene::world_transforms(&expanded)?;
    let direct_root = root.id.in_instance(direct.in_instance(outer));
    let attached_root = root.id.in_instance(attached.in_instance(outer));
    assert_eq!(
        worlds[&direct_root].w_axis.truncate().to_array(),
        [13., 22., 33.]
    );
    assert_eq!(
        worlds[&attached_root].w_axis.truncate().to_array(),
        [11., 27., 33.]
    );
    let mut detached = source.clone();
    detached.detach_instance(outer, None, &mut library)?;
    assert_eq!(detached.expand(&mut library)?, expanded);
    for instance in detached.instances.iter().map(|i| i.id).collect::<Vec<_>>() {
        detached.detach_instance(instance, None, &mut library)?;
        assert_eq!(detached.expand(&mut library)?, expanded);
    }
    assert!(detached.instances.is_empty());
    let bytes = source.to_json()?;
    assert_eq!(ContentSource::from_json(&bytes)?, source);
    let mut legacy = source.clone();
    legacy.version = 1;
    assert!(legacy.validate().is_err());
    legacy.instances[0].translation = [0.; 3];
    let legacy_bytes = legacy.to_json()?;
    assert!(!std::str::from_utf8(&legacy_bytes)?.contains("translation"));
    assert_eq!(
        ContentSource::from_json(&legacy_bytes)?.instances[0].translation,
        [0.; 3]
    );
    legacy.version = 2;
    for offset in [[f32::NAN, 0., 0.], [f32::INFINITY, 0., 0.]] {
        legacy.instances[0].translation = offset;
        assert!(legacy.validate().is_err());
    }
    let mut overflow = source;
    overflow.instances[0].translation = [f32::MAX, 0., 0.];
    library.0.get_mut(&branch).unwrap().instances[0].translation = [f32::MAX, 0., 0.];
    assert!(overflow.expand(&mut library).is_err());
    let before = overflow.clone();
    assert!(overflow.detach_instance(outer, None, &mut library).is_err());
    assert_eq!(overflow, before);
    Ok(())
}
