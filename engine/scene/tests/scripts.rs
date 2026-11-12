use anyhow::Result;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{
    ContentSource, EntityData, EntityRef, FieldOverride, FieldUpdate, MAX_SCRIPT_KEY_BYTES,
    MAX_SCRIPT_PARAMETERS, MAX_SCRIPT_TEXT_BYTES, Model, PrefabInstance, PrefabResolver, Scene,
    SceneSnapshot, Script, ScriptParameters, ScriptValue, Surface,
    schema::{self, FieldValue},
};
use std::{collections::BTreeMap, sync::Arc};

fn scene(entities: Vec<EntityData>) -> SceneSnapshot {
    SceneSnapshot {
        entities,
        ..Default::default()
    }
}
fn scripted(name: &str) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.script = Some(Script::new(AssetId::new()));
    entity
}

#[test]
fn script_protocol_keeps_explicit_typed_parameters_and_defaults() -> Result<()> {
    let entity = EntityData::new("legacy");
    let asset = AssetId::new();
    let default: Script = serde_json::from_value(serde_json::json!({"asset":asset}))?;
    assert!(default.enabled);
    assert!(default.parameters.is_empty());
    let mut scripted = entity;
    scripted.script = Some(default);
    let mut snapshot = SceneSnapshot {
        version: comet_scene::scene::SCENE_VERSION,
        entities: vec![scripted],
    };
    let script = snapshot.entities[0].script.as_mut().unwrap();
    script.parameters = BTreeMap::from([
        ("bool".into(), ScriptValue::Bool(true)),
        ("integer".into(), ScriptValue::Integer(i64::MIN)),
        ("number".into(), ScriptValue::Number(f64::MAX)),
        ("text".into(), ScriptValue::Text("中文".into())),
        ("vec3".into(), ScriptValue::Vec3([1., 2., 3.])),
        ("entity".into(), ScriptValue::Entity(None)),
    ]);
    let json: serde_json::Value = serde_json::from_slice(&snapshot.to_json()?)?;
    for name in ["bool", "integer", "number", "text", "vec3", "entity"] {
        assert_eq!(
            json["entities"][0]["script"]["parameters"][name]["type"],
            name
        );
    }
    assert!(json["entities"][0]["script"]["parameters"]["entity"]["value"].is_null());
    assert_eq!(SceneSnapshot::from_json(&snapshot.to_json()?)?, snapshot);
    Ok(())
}

#[test]
fn parameters_enforce_finite_values_and_byte_budgets_before_field_publication() -> Result<()> {
    let mut entity = scripted("bounded");
    let mut parameters: ScriptParameters = (0..MAX_SCRIPT_PARAMETERS)
        .map(|index| (format!("p{index}"), ScriptValue::Integer(index as i64)))
        .collect();
    parameters.insert(
        "p0".into(),
        ScriptValue::Text("x".repeat(MAX_SCRIPT_TEXT_BYTES)),
    );
    entity.script.as_mut().unwrap().parameters = parameters.clone();
    let mut original = scene(vec![entity]);
    original.validate()?;
    let id = original.entities[0].id;
    parameters.insert("overflow".into(), ScriptValue::Bool(false));
    let mut invalid = vec![parameters];
    for (key, value) in [
        ("".into(), ScriptValue::Bool(true)),
        (" ".into(), ScriptValue::Bool(true)),
        (
            "x".repeat(MAX_SCRIPT_KEY_BYTES + 1),
            ScriptValue::Bool(true),
        ),
        (
            "文".repeat(MAX_SCRIPT_KEY_BYTES / 3 + 1),
            ScriptValue::Bool(true),
        ),
        (
            "p".into(),
            ScriptValue::Text("x".repeat(MAX_SCRIPT_TEXT_BYTES + 1)),
        ),
        ("p".into(), ScriptValue::Number(f64::NAN)),
        ("p".into(), ScriptValue::Number(f64::INFINITY)),
        ("p".into(), ScriptValue::Vec3([0., f32::NEG_INFINITY, 0.])),
    ] {
        invalid.push(BTreeMap::from([(key, value)]));
    }
    for parameters in invalid {
        let before = original.clone();
        assert!(
            original
                .set_field(
                    &[id],
                    schema::SCRIPT_PARAMETERS,
                    FieldValue::ScriptParameters(parameters)
                )
                .is_err()
        );
        assert_eq!(original, before);
    }
    let parameters = BTreeMap::from([(
        "x".repeat(MAX_SCRIPT_KEY_BYTES),
        ScriptValue::Integer(i64::MAX),
    )]);
    original.set_field(
        &[id],
        schema::SCRIPT_PARAMETERS,
        FieldValue::ScriptParameters(parameters),
    )?;
    original.entities[0].script.as_ref().unwrap().validate()?;
    Ok(())
}

#[test]
fn script_references_participate_in_dependencies_clone_remapping_and_delete_cleanup() -> Result<()>
{
    let mut root = scripted("root");
    let mut child = EntityData::new("child");
    child.parent = Some(root.id);
    let outside_scene = AssetId::new();
    root.script.as_mut().unwrap().parameters = BTreeMap::from([
        (
            "self".into(),
            ScriptValue::Entity(Some(EntityRef::local(root.id))),
        ),
        (
            "child".into(),
            ScriptValue::Entity(Some(EntityRef::local(child.id))),
        ),
        (
            "external".into(),
            ScriptValue::Entity(Some(EntityRef::External {
                scene: outside_scene,
                entity: child.id,
            })),
        ),
    ]);
    let asset = root.script.as_ref().unwrap().asset;
    let root_id = root.id;
    let child_id = child.id;
    let mut snapshot = scene(vec![root, child]);
    assert_eq!(
        snapshot.asset_dependencies(),
        [asset, outside_scene].into_iter().collect()
    );
    let copied = snapshot.clone_subtrees(&[root_id])?;
    let parameters = &snapshot
        .entities
        .iter()
        .find(|entity| entity.id == copied[&root_id])
        .unwrap()
        .script
        .as_ref()
        .unwrap()
        .parameters;
    assert_eq!(
        parameters["self"],
        ScriptValue::Entity(Some(EntityRef::local(copied[&root_id])))
    );
    assert_eq!(
        parameters["child"],
        ScriptValue::Entity(Some(EntityRef::local(copied[&child_id])))
    );
    assert_eq!(
        parameters["external"],
        ScriptValue::Entity(Some(EntityRef::External {
            scene: outside_scene,
            entity: child_id
        }))
    );
    snapshot.remove_subtrees(&[child_id])?;
    let root = snapshot
        .entities
        .iter()
        .find(|entity| entity.id == root_id)
        .unwrap();
    assert_eq!(
        root.script.as_ref().unwrap().parameters["child"],
        ScriptValue::Entity(None)
    );
    snapshot.validate()?;
    let mut stale = snapshot.clone();
    stale
        .entities
        .iter_mut()
        .find(|entity| entity.id == root_id)
        .unwrap()
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert(
            "stale".into(),
            ScriptValue::Entity(Some(EntityRef::local(EntityId::new()))),
        );
    assert!(stale.validate().is_err());
    Ok(())
}

struct Library(AssetId, ContentSource);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        anyhow::ensure!(id == self.0, "missing prefab");
        Ok(self.1.clone())
    }
}

#[test]
fn prefab_script_overrides_remap_parameters_and_keep_explicit_external_dependencies() -> Result<()>
{
    let root = scripted("root");
    let mut child = EntityData::new("child");
    child.parent = Some(root.id);
    let root_id = root.id;
    let child_id = child.id;
    let mut library = Library(
        AssetId::new(),
        ContentSource {
            base: scene(vec![root, child]),
            ..Default::default()
        },
    );
    let instance = InstanceId::new();
    let script_asset = AssetId::new();
    let outside_scene = AssetId::new();
    let parameters = BTreeMap::from([
        (
            "child".into(),
            ScriptValue::Entity(Some(EntityRef::local(child_id))),
        ),
        (
            "external".into(),
            ScriptValue::Entity(Some(EntityRef::External {
                scene: outside_scene,
                entity: EntityId::new(),
            })),
        ),
    ]);
    let mut source = ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: library.0,
            parent: None,
            overrides: vec![
                FieldOverride {
                    entity: root_id,
                    field: schema::SCRIPT_ASSET,
                    value: FieldValue::Asset(script_asset),
                },
                FieldOverride {
                    entity: root_id,
                    field: schema::SCRIPT_ENABLED,
                    value: FieldValue::Bool(false),
                },
                FieldOverride {
                    entity: root_id,
                    field: schema::SCRIPT_PARAMETERS,
                    value: FieldValue::ScriptParameters(parameters.clone()),
                },
            ],
        }],
        ..Default::default()
    };
    assert_eq!(
        source.dependencies(),
        [library.0, script_asset, outside_scene]
            .into_iter()
            .collect()
    );
    let expanded = source.expand(&mut library)?;
    let script = expanded
        .entities
        .iter()
        .find(|entity| entity.id == root_id.in_instance(instance))
        .unwrap()
        .script
        .as_ref()
        .unwrap();
    assert_eq!(script.asset, script_asset);
    assert!(!script.enabled);
    assert_eq!(
        script.parameters["child"],
        ScriptValue::Entity(Some(EntityRef::local(child_id.in_instance(instance))))
    );
    assert_eq!(script.parameters["external"], parameters["external"]);
    source.instances[0].overrides[1].value = FieldValue::Scalar(1.);
    assert!(source.expand(&mut library).is_err());
    Ok(())
}

#[test]
fn live_scene_field_batches_keep_handles_and_publish_only_fully_valid_candidates() -> Result<()> {
    let first = scripted("first");
    let mut second = EntityData::new("second");
    second.model = Some(Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    let (first_id, second_id) = (first.id, second.id);
    let mut world = Scene::open(scene(vec![first, second]))?;
    let first_handle = world.handle(first_id)?;
    let second_handle = world.handle(second_id)?;
    let updates = vec![
        FieldUpdate {
            entity: first_id,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([1., 2., 3.]),
        },
        FieldUpdate {
            entity: second_id,
            field: schema::ROUGHNESS,
            value: FieldValue::Scalar(0.8),
        },
        FieldUpdate {
            entity: first_id,
            field: schema::SCRIPT_ENABLED,
            value: FieldValue::Bool(false),
        },
        FieldUpdate {
            entity: first_id,
            field: schema::SCRIPT_PARAMETERS,
            value: FieldValue::ScriptParameters(BTreeMap::from([(
                "target".into(),
                ScriptValue::Entity(Some(EntityRef::local(second_id))),
            )])),
        },
    ];
    let revision = world.revision();
    world.set_fields(&updates)?;
    assert_eq!(world.revision(), revision + 1);
    assert_eq!(
        world.resolve(first_handle)?.transform.translation,
        [1., 2., 3.]
    );
    assert!(!world.resolve(first_handle)?.script.unwrap().enabled);
    assert_eq!(
        world
            .resolve(second_handle)?
            .model
            .unwrap()
            .surface
            .roughness,
        0.8
    );
    let accepted = world.shared_snapshot();
    let revision = world.revision();
    for last in [
        FieldUpdate {
            entity: second_id,
            field: schema::ROUGHNESS,
            value: FieldValue::Scalar(2.),
        },
        FieldUpdate {
            entity: EntityId::new(),
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([0.; 3]),
        },
        updates[0].clone(),
    ] {
        let mut change = updates[0].clone();
        change.value = FieldValue::Vec3([9.; 3]);
        assert!(world.set_fields(&[change, last]).is_err());
        assert!(Arc::ptr_eq(&accepted, &world.shared_snapshot()));
        assert_eq!(world.revision(), revision);
        assert_eq!(
            world.resolve(first_handle)?.transform.translation,
            [1., 2., 3.]
        );
    }
    world.set_fields(&[])?;
    world.set_fields(&updates)?;
    assert!(Arc::ptr_eq(&accepted, &world.shared_snapshot()));
    world.update_poses(&[comet_scene::PoseUpdate {
        entity: first_id,
        translation: [2., 3., 4.],
        rotation: [0., 0., 0., 1.],
    }])?;
    let script = world.resolve(first_handle)?.script.unwrap();
    assert!(!script.enabled);
    assert_eq!(
        script.parameters["target"],
        ScriptValue::Entity(Some(EntityRef::local(second_id)))
    );
    Ok(())
}
