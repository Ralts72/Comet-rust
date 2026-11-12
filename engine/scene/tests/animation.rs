use anyhow::Result;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{
    Animator, ContentSource, EntityData, EntityRef, Model, PrefabInstance, Scene, SceneSnapshot,
    SkinBinding, Surface,
    schema::{self, FieldValue},
};
use std::collections::BTreeMap;

fn rig() -> (SceneSnapshot, EntityId, EntityId, EntityId, EntityId) {
    let mut owner = EntityData::new("rig");
    let mut bone = EntityData::new("bone");
    let mut mesh = EntityData::new("skin");
    let source = EntityId::new();
    let (root, joint, model) = (owner.id, bone.id, mesh.id);
    owner.animator = Some(Animator {
        controller: None,
        skeleton: AssetId::new(),
        clip: Some(AssetId::new()),
        bindings: [(source, Some(EntityRef::local(joint)))].into(),
        looping: true,
        speed: 1.,
    });
    bone.parent = Some(root);
    mesh.parent = Some(root);
    mesh.model = Some(Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    mesh.skin = Some(SkinBinding {
        skin: AssetId::new(),
        animator: Some(EntityRef::local(root)),
    });
    (
        SceneSnapshot {
            entities: vec![owner, bone, mesh],
            ..Default::default()
        },
        root,
        joint,
        model,
        source,
    )
}

#[test]
fn versioned_animation_components_round_trip_ecs_and_publish_only_valid_field_batches() -> Result<()>
{
    let (snapshot, owner, _, mesh, _) = rig();
    let mut scene = Scene::open(SceneSnapshot::from_json(&snapshot.to_json()?)?)?;
    let original = scene.shared_snapshot();
    let revision = scene.revision();
    assert!(
        scene
            .set_fields(&[
                comet_scene::FieldUpdate {
                    entity: owner,
                    field: schema::ANIMATOR_LOOPING,
                    value: FieldValue::Bool(false)
                },
                comet_scene::FieldUpdate {
                    entity: owner,
                    field: schema::ANIMATOR_SPEED,
                    value: FieldValue::Scalar(-1.)
                },
            ])
            .is_err()
    );
    assert_eq!(scene.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(&original, &scene.shared_snapshot()));
    scene.set_fields(&[comet_scene::FieldUpdate {
        entity: mesh,
        field: schema::SKIN_ANIMATOR,
        value: FieldValue::Entity(None),
    }])?;
    scene.set_fields(&[comet_scene::FieldUpdate {
        entity: owner,
        field: schema::ANIMATOR_SPEED,
        value: FieldValue::Scalar(2.),
    }])?;
    scene.update_poses(&[comet_scene::PoseUpdate {
        entity: owner,
        translation: [1., 0., 0.],
        rotation: [0., 0., 0., 1.],
    }])?;
    assert_eq!(
        scene
            .shared_snapshot()
            .entities
            .iter()
            .find(|e| e.id == owner)
            .unwrap()
            .animator
            .as_ref()
            .unwrap()
            .speed,
        2.
    );
    assert!(
        scene
            .shared_snapshot()
            .entities
            .iter()
            .find(|e| e.id == mesh)
            .unwrap()
            .skin
            .as_ref()
            .unwrap()
            .animator
            .is_none()
    );
    let mut no_model = snapshot.clone();
    no_model
        .entities
        .iter_mut()
        .find(|e| e.id == mesh)
        .unwrap()
        .model = None;
    assert!(no_model.validate().is_err());
    let mut json: serde_json::Value = serde_json::from_slice(&snapshot.to_json()?)?;
    json["version"] = 8.into();
    for key in ["animator", "skin"] {
        let mut value = json.clone();
        for entity in value["entities"].as_array_mut().unwrap() {
            entity.as_object_mut().unwrap().remove("animator");
            entity.as_object_mut().unwrap().remove("skin");
        }
        value["entities"][0][key] = serde_json::Value::Null;
        assert!(SceneSnapshot::from_json(&serde_json::to_vec(&value)?).is_err());
    }
    Ok(())
}

#[test]
fn bindings_decode_has_a_concrete_budget_and_rejects_duplicate_source_keys() -> Result<()> {
    let skeleton = AssetId::new();
    let source = EntityId::new();
    let duplicate = format!(
        r#"{{"skeleton":"{skeleton}","clip":null,"bindings":{{"{source}":null,"{source}":null}},"looping":true,"speed":1}}"#
    );
    assert!(serde_json::from_str::<Animator>(&duplicate).is_err());
    let bindings = (0..=comet_scene::MAX_ANIMATOR_BINDINGS)
        .map(|_| (EntityId::new(), None))
        .collect();
    let value = FieldValue::AnimatorBindings(bindings);
    assert!(
        schema::descriptor(schema::ANIMATOR_BINDINGS)?
            .validate_value(&value)
            .is_err()
    );
    assert!(serde_json::from_slice::<FieldValue>(&serde_json::to_vec(&value)?).is_err());
    Ok(())
}

#[test]
fn source_node_keys_survive_clone_delete_and_two_nested_instance_namespaces() -> Result<()> {
    let (mut snapshot, owner, joint, mesh, source) = rig();
    let external_scene = AssetId::new();
    let external_node = EntityId::new();
    snapshot.entities[0]
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .insert(
            external_node,
            Some(EntityRef::External {
                scene: external_scene,
                entity: EntityId::new(),
            }),
        );
    let before = snapshot.clone();
    let cloned = snapshot.clone_subtrees(&[owner])?;
    let copy = snapshot
        .entities
        .iter()
        .find(|e| e.id == cloned[&owner])
        .unwrap()
        .animator
        .as_ref()
        .unwrap();
    assert_eq!(
        copy.bindings[&source],
        Some(EntityRef::local(cloned[&joint]))
    );
    assert!(copy.bindings.contains_key(&external_node));
    assert!(
        matches!(copy.bindings[&external_node], Some(EntityRef::External { scene, .. }) if scene == external_scene)
    );
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|e| e.id == cloned[&mesh])
            .unwrap()
            .skin
            .as_ref()
            .unwrap()
            .animator,
        Some(EntityRef::local(cloned[&owner]))
    );
    snapshot.remove_subtrees(&[cloned[&joint]])?;
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|e| e.id == cloned[&owner])
            .unwrap()
            .animator
            .as_ref()
            .unwrap()
            .bindings[&source],
        None
    );
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|e| e.id == owner)
            .unwrap()
            .animator
            .as_ref()
            .unwrap()
            .bindings[&source],
        Some(EntityRef::local(joint))
    );

    let asset = AssetId::new();
    let wrapper = AssetId::new();
    let inner = InstanceId::new();
    let outer = InstanceId::new();
    let override_scene = AssetId::new();
    let mut bindings = before.entities[0]
        .animator
        .as_ref()
        .unwrap()
        .bindings
        .clone();
    bindings.insert(
        external_node,
        Some(EntityRef::External {
            scene: override_scene,
            entity: EntityId::new(),
        }),
    );
    let mut nested = ContentSource::default();
    nested.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: inner,
        prefab: asset,
        parent: None,
        overrides: vec![comet_scene::FieldOverride {
            entity: owner,
            field: schema::ANIMATOR_BINDINGS,
            value: FieldValue::AnimatorBindings(bindings),
        }],
    });
    assert!(nested.dependencies().contains(&override_scene));
    let mut top = ContentSource::default();
    top.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: outer,
        prefab: wrapper,
        parent: None,
        overrides: vec![],
    });
    let sources = BTreeMap::from([
        (
            asset,
            ContentSource {
                version: comet_scene::prefab::CONTENT_VERSION,
                base: before,
                instances: vec![],
            },
        ),
        (wrapper, nested),
    ]);
    struct Library(BTreeMap<AssetId, ContentSource>);
    impl comet_scene::PrefabResolver for Library {
        fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
            Ok(self.0[&id].clone())
        }
    }
    let expanded = top.expand(&mut Library(sources))?;
    let scope = inner.in_instance(outer);
    let owner = expanded
        .entities
        .iter()
        .find(|e| e.id == owner.in_instance(scope))
        .unwrap();
    assert_eq!(
        owner.animator.as_ref().unwrap().bindings[&source],
        Some(EntityRef::local(joint.in_instance(scope)))
    );
    assert!(
        owner
            .animator
            .as_ref()
            .unwrap()
            .bindings
            .contains_key(&external_node)
    );
    assert!(expanded.asset_dependencies().contains(&override_scene));
    assert!(
        expanded
            .asset_dependencies()
            .contains(&owner.animator.as_ref().unwrap().skeleton)
    );
    Ok(())
}
