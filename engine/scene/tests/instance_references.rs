use anyhow::Result;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{
    Animator, ContentSource, EntityData, EntityRef, FieldOverride, JointConnection, JointSettings,
    PhysicsBody, PointJoint, PrefabInstance, PrefabResolver, Scene, SceneSnapshot, Script,
    ScriptValue,
    schema::{self, FieldValue},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, asset: AssetId) -> Result<ContentSource> {
        self.0
            .get(&asset)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing prefab {asset}"))
    }
}
fn content(
    version: u32,
    entities: Vec<EntityData>,
    instances: Vec<PrefabInstance>,
) -> ContentSource {
    ContentSource {
        base: SceneSnapshot { version, entities },
        instances,
        ..Default::default()
    }
}
fn instance(id: InstanceId, prefab: AssetId) -> PrefabInstance {
    PrefabInstance {
        id,
        prefab,
        parent: None,
        overrides: vec![],
    }
}
fn target(snapshot: &SceneSnapshot, owner: EntityId) -> Option<EntityRef> {
    snapshot
        .entities
        .iter()
        .find(|entity| entity.id == owner)
        .unwrap()
        .links
        .target
        .clone()
}

#[test]
fn direct_source_identity_roundtrips_without_becoming_an_authored_expanded_uuid() -> Result<()> {
    let mut owner = EntityData::new("author");
    let mut child = EntityData::new("prefab target");
    child.id = owner.id; // Equal source UUIDs are valid in different authoring namespaces.
    let source_id = child.id;
    let prefab = AssetId::new();
    let first = InstanceId::new();
    let second = InstanceId::new();
    owner.links.target = Some(EntityRef::in_instance(first, source_id));
    owner.links.connections = vec![EntityRef::in_instance(second, source_id)];
    let mut source = content(
        10,
        vec![owner.clone()],
        vec![instance(first, prefab), instance(second, prefab)],
    );
    let mut library = Library([(prefab, content(1, vec![child], vec![]))].into());
    source.validate()?;
    source.canonicalize();
    let bytes = source.to_json()?;
    assert_eq!(ContentSource::from_json(&bytes)?, source);
    let json: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        json["base"]["entities"][0]["links"]["target"]["scope"],
        "in_instance"
    );
    assert_eq!(json["version"], 1);
    let mut document_mirror = Scene::open(source.base.clone())?;
    assert_eq!(document_mirror.snapshot(), source.base);
    assert!(source.base.validate_expanded().is_err());
    let expanded = source.expand(&mut library)?;
    expanded.validate_expanded()?;
    assert_eq!(expanded.entities.len(), 3);
    assert_eq!(
        target(&expanded, owner.id),
        Some(EntityRef::local(source_id.in_instance(first)))
    );
    assert_ne!(source_id.in_instance(first), source_id.in_instance(second));
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|entity| entity.id == owner.id)
            .unwrap()
            .links
            .connections,
        [EntityRef::local(source_id.in_instance(second))]
    );
    assert_eq!(source.dependencies(), BTreeSet::from([prefab]));
    let mut fake_local = source.clone();
    fake_local.base.entities[0].links.target = Some(EntityRef::local(source_id.in_instance(first)));
    assert!(
        fake_local.validate().is_err(),
        "expanded IDs never become valid author Local references"
    );
    let mut old = source;
    old.base.version = 9;
    assert!(
        old.validate()
            .unwrap_err()
            .to_string()
            .contains("version 10")
    );
    Ok(())
}

#[test]
fn nested_sources_compose_instance_scopes_before_mapping_entities_and_never_search_descendants()
-> Result<()> {
    let leaf_asset = AssetId::new();
    let branch_asset = AssetId::new();
    let leaf = EntityData::new("leaf target");
    let leaf_id = leaf.id;
    let inner = InstanceId::new();
    let mut branch = EntityData::new("branch owner");
    let branch_id = branch.id;
    branch.links.target = Some(EntityRef::in_instance(inner, leaf_id));
    let first = InstanceId::new();
    let second = InstanceId::new();
    let mut owner = EntityData::new("outer owner");
    owner.links.target = Some(EntityRef::in_instance(first, branch_id));
    let owner_id = owner.id;
    let source = content(
        10,
        vec![owner],
        vec![
            instance(first, branch_asset),
            instance(second, branch_asset),
        ],
    );
    let mut library = Library(
        [
            (leaf_asset, content(1, vec![leaf], vec![])),
            (
                branch_asset,
                content(10, vec![branch], vec![instance(inner, leaf_asset)]),
            ),
        ]
        .into(),
    );
    let expanded = source.expand(&mut library)?;
    assert_eq!(
        target(&expanded, owner_id),
        Some(EntityRef::local(branch_id.in_instance(first)))
    );
    for outer in [first, second] {
        let expected = leaf_id.in_instance(inner.in_instance(outer));
        assert_eq!(
            target(&expanded, branch_id.in_instance(outer)),
            Some(EntityRef::local(expected))
        );
        assert_ne!(expected, leaf_id.in_instance(inner).in_instance(outer));
    }
    for forbidden in [leaf_id, leaf_id.in_instance(inner)] {
        let mut invalid = source.clone();
        invalid.base.entities[0].links.target = Some(EntityRef::in_instance(first, forbidden));
        assert!(
            invalid
                .expand(&mut library)
                .unwrap_err()
                .to_string()
                .contains("direct source entity")
        );
    }
    Ok(())
}

#[test]
fn overrides_keep_the_child_lexical_scope_and_upgrade_for_new_reference_values() -> Result<()> {
    let leaf_asset = AssetId::new();
    let branch_asset = AssetId::new();
    let leaf = EntityData::new("leaf");
    let leaf_id = leaf.id;
    let branch = EntityData::new("overridden branch");
    let branch_id = branch.id;
    let inner = InstanceId::new();
    let outer = InstanceId::new();
    let mut declaration = instance(outer, branch_asset);
    declaration.overrides.push(FieldOverride {
        entity: branch_id,
        field: schema::TARGET,
        value: FieldValue::Entity(Some(EntityRef::in_instance(inner, leaf_id))),
    });
    let source = content(10, vec![], vec![declaration, instance(inner, leaf_asset)]);
    let original_branch = content(1, vec![branch], vec![instance(inner, leaf_asset)]);
    let mut library = Library(
        [
            (leaf_asset, content(1, vec![leaf], vec![])),
            (branch_asset, original_branch.clone()),
        ]
        .into(),
    );
    let expanded = source.expand(&mut library)?;
    assert_eq!(
        target(&expanded, branch_id.in_instance(outer)),
        Some(EntityRef::local(
            leaf_id.in_instance(inner.in_instance(outer))
        ))
    );
    assert_ne!(
        target(&expanded, branch_id.in_instance(outer)),
        Some(EntityRef::local(leaf_id.in_instance(inner)))
    );
    assert_eq!(
        library.0[&branch_asset], original_branch,
        "candidate overrides never edit the source product"
    );
    let mut old = source.clone();
    old.base.version = 9;
    assert!(
        old.validate()
            .unwrap_err()
            .to_string()
            .contains("version 10")
    );
    let parent_only = InstanceId::new();
    let mut invalid = source;
    invalid.instances.push(instance(parent_only, leaf_asset));
    invalid
        .instances
        .iter_mut()
        .find(|instance| instance.id == outer)
        .unwrap()
        .overrides[0]
        .value = FieldValue::Entity(Some(EntityRef::in_instance(parent_only, leaf_id)));
    assert!(
        invalid
            .expand(&mut library)
            .unwrap_err()
            .to_string()
            .contains("missing prefab instance")
    );
    assert_eq!(library.0[&branch_asset], original_branch);
    Ok(())
}

#[test]
fn all_reference_values_share_version_remap_and_explicit_instance_deletion_rules() -> Result<()> {
    let old = InstanceId::new();
    let new = InstanceId::new();
    let mut entity = EntityData::new("owner");
    let source_id = entity.id;
    let reference = EntityRef::in_instance(old, source_id);
    let external = EntityRef::External {
        scene: AssetId::new(),
        entity: source_id,
    };
    let parameters = [("next".into(), ScriptValue::Entity(Some(reference.clone())))].into();
    let bindings = [(source_id, Some(reference.clone()))].into();
    for value in [
        FieldValue::Entity(Some(reference.clone())),
        FieldValue::Entities(vec![reference.clone()]),
        FieldValue::ScriptParameters(parameters),
        FieldValue::AnimatorBindings(bindings),
        FieldValue::JointConnection(JointConnection::Body(reference.clone())),
    ] {
        assert_eq!(schema::required_value_scene_version(&value), 10);
    }
    assert_eq!(
        schema::required_value_scene_version(&FieldValue::Entity(Some(external.clone()))),
        1
    );
    entity.links.target = Some(reference.clone());
    entity.links.connections = vec![
        EntityRef::local(source_id),
        reference.clone(),
        external.clone(),
    ];
    let mut script = Script::new(AssetId::new());
    script
        .parameters
        .insert("next".into(), ScriptValue::Entity(Some(reference.clone())));
    entity.script = Some(script);
    let mut animator = Animator::new(AssetId::new());
    animator.bindings.insert(source_id, Some(reference.clone()));
    entity.animator = Some(animator);
    entity.physics = Some(PhysicsBody::default());
    entity.point_joint = Some(PointJoint {
        settings: JointSettings {
            connection: JointConnection::Body(reference),
            ..Default::default()
        },
        ..Default::default()
    });
    schema::remap_instance_references(&mut entity, &[(old, new)].into())?;
    let replacement = EntityId::new();
    schema::remap(&mut entity, &[(source_id, replacement)].into())?;
    for reference in schema::entity_references(&entity) {
        if let EntityRef::InInstance { instance, entity } = reference {
            assert_eq!(instance, new);
            assert_eq!(
                entity, source_id,
                "source entity keys are never scene UUID mappings"
            );
        }
    }
    assert!(
        entity
            .animator
            .as_ref()
            .unwrap()
            .bindings
            .contains_key(&source_id)
    );
    schema::clear_instance_references(&mut entity, &BTreeSet::from([new]))?;
    assert!(entity.links.target.is_none());
    assert_eq!(
        entity.links.connections,
        [EntityRef::local(replacement), external]
    );
    assert_eq!(
        entity.script.as_ref().unwrap().parameters["next"],
        ScriptValue::Entity(None)
    );
    assert_eq!(entity.animator.as_ref().unwrap().bindings[&source_id], None);
    assert_eq!(
        entity.point_joint.as_ref().unwrap().settings.connection,
        JointConnection::Unbound
    );
    Ok(())
}

#[test]
fn missing_declarations_targets_and_expanded_collisions_reject_without_rewriting_sources()
-> Result<()> {
    let prefab = AssetId::new();
    let declaration = InstanceId::new();
    let child = EntityData::new("child");
    let child_id = child.id;
    let mut owner = EntityData::new("owner");
    owner.links.target = Some(EntityRef::in_instance(declaration, child_id));
    let source = content(10, vec![owner], vec![instance(declaration, prefab)]);
    let mut library = Library([(prefab, content(1, vec![child], vec![]))].into());
    let mut missing = source.clone();
    missing.instances.clear();
    assert!(
        missing
            .validate()
            .unwrap_err()
            .to_string()
            .contains("missing prefab instance")
    );
    let mut stale = source.clone();
    stale.base.entities[0].links.target =
        Some(EntityRef::in_instance(declaration, EntityId::new()));
    let before = stale.clone();
    assert!(stale.expand(&mut library).is_err());
    assert_eq!(stale, before);
    let mut collision = source.clone();
    collision.base.entities[0].id = child_id.in_instance(declaration);
    assert!(
        collision
            .expand(&mut library)
            .unwrap_err()
            .to_string()
            .contains("duplicate entity")
    );
    assert_eq!(source.expand(&mut library)?.entities.len(), 2);
    Ok(())
}
