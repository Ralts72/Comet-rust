use anyhow::{Context, Result};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::{Document, FieldEdit, FieldTarget, JointEdit};
use comet_scene::{
    BodyMotion, ContentSource, EntityData, FieldOverride, Model, PhysicsBody, PointJoint,
    PrefabInstance, PrefabResolver, SceneSnapshot, Script, Surface,
    schema::{self, FieldValue},
};
use std::{collections::BTreeMap, fs, sync::Arc};

struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.0.get(&id).cloned().context("missing prefab")
    }
}
fn instance(prefab: AssetId) -> PrefabInstance {
    PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab,
        parent: None,
        overrides: vec![],
    }
}
fn source(entity: EntityData) -> ContentSource {
    ContentSource {
        base: SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn root_scoped_structural_failures_preserve_source_history_savepoint_and_redo() -> Result<()> {
    let root = AssetId::new();
    let branch = AssetId::new();
    let entity = EntityData::new("source root");
    let id = entity.id;
    let source = source(entity);
    let mut library = Library(BTreeMap::from([
        (root, source.clone()),
        (
            branch,
            ContentSource {
                instances: vec![instance(root)],
                ..Default::default()
            },
        ),
    ]));
    let mut document = Document::open_prefab(root, source.clone(), &mut library)?;
    assert_eq!(document.prefab_asset(), Some(root));
    document.begin()?;
    document.preview_name(id, "accepted edit".into())?;
    document.commit()?;
    let redo = document.shared_source();
    document.undo()?;
    let before = document.shared_source();
    let revision = document.revision();
    let state = document.state_id();
    let world = document.world_id();
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("source.prefab");
    document.save(&path)?;
    let saved = fs::read(&path)?;
    for target in [root, branch] {
        let mut invalid = source.clone();
        invalid.instances.push(instance(target));
        assert!(invalid.expand(&mut library).is_ok());
        assert!(Document::open_prefab(root, invalid.clone(), &mut library).is_err());
        let scene = Document::open_source(invalid)?;
        assert_eq!(scene.prefab_asset(), None);
        scene.expand(&mut library)?;
        for container in [None, Some(EntityData::new("candidate container"))] {
            let error = document
                .insert_prefab(container, target, [0.; 3], &mut library)
                .unwrap_err();
            assert!(format!("{error:#}").contains("cycle"));
            assert!(Arc::ptr_eq(&before, &document.shared_source()));
            assert_eq!(
                (
                    document.revision(),
                    document.state_id(),
                    document.world_id()
                ),
                (revision, state, world)
            );
            assert_eq!(document.history_len(), 0);
            assert!(!document.is_dirty() && !document.is_editing());
            assert_eq!(document.expand(&mut library)?, source.base);
            assert_eq!(fs::read(&path)?, saved);
        }
    }
    document.redo()?;
    assert_eq!(document.shared_source(), redo);
    assert!(document.is_dirty());
    assert_eq!(document.prefab_asset(), Some(root));
    Ok(())
}

#[test]
fn every_resolved_preview_keeps_its_accepted_candidate_when_the_root_would_recur() -> Result<()> {
    let root = AssetId::new();
    let branch = AssetId::new();
    let mut entity = EntityData::new("source body");
    entity.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    let id = entity.id;
    let initial = source(entity);
    let mut old_product = initial.clone();
    old_product.base.entities[0].physics = None;
    let mut library = Library(BTreeMap::from([
        (root, old_product),
        (branch, ContentSource::default()),
    ]));
    let mut document = Document::open_prefab(root, initial, &mut library)?;
    document.insert_prefab(
        Some(EntityData::new("child mount")),
        branch,
        [0.; 3],
        &mut library,
    )?;
    let committed = document.shared_source();
    document.begin()?;
    document.preview_name(id, "accepted preview".into())?;
    let accepted = document.shared_source();
    let revision = document.revision();
    let state = document.state_id();
    library
        .0
        .get_mut(&branch)
        .unwrap()
        .instances
        .push(instance(root));
    assert!(
        accepted.expand(&mut library).is_ok(),
        "old root product would mask this cycle"
    );
    assert!(document.expand(&mut library).is_err());
    type Preview = fn(&mut Document, EntityId, &mut Library) -> Result<()>;
    let edits: [Preview; 7] = [
        |doc, id, resolver| {
            doc.preview_fields(
                &[FieldEdit {
                    target: FieldTarget::Authored(id),
                    field: schema::TRANSLATION,
                    value: FieldValue::Vec3([2., 0., 0.]),
                }],
                resolver,
            )
        },
        |doc, id, resolver| {
            doc.preview_field_values(
                &[FieldEdit {
                    target: FieldTarget::Authored(id),
                    field: schema::TRANSLATION,
                    value: FieldValue::Vec3([3., 0., 0.]),
                }],
                resolver,
            )
        },
        |doc, id, resolver| {
            doc.preview_fields_from_baseline(
                &[FieldEdit {
                    target: FieldTarget::Authored(id),
                    field: schema::TRANSLATION,
                    value: FieldValue::Vec3([4., 0., 0.]),
                }],
                resolver,
            )
        },
        |doc, id, resolver| {
            doc.preview_physics(
                id,
                Some(PhysicsBody {
                    motion: BodyMotion::Dynamic,
                    ..Default::default()
                }),
                None,
                resolver,
            )
        },
        |doc, id, resolver| {
            doc.preview_model(
                id,
                Some(Model {
                    mesh: AssetId::new(),
                    surface: Surface {
                        material: AssetId::new(),
                        roughness: 0.5,
                    },
                }),
                resolver,
            )
        },
        |doc, id, resolver| doc.preview_script(id, Some(Script::new(AssetId::new())), resolver),
        |doc, id, resolver| {
            doc.preview_joint(id, JointEdit::Point(Some(PointJoint::default())), resolver)
        },
    ];
    for preview in edits {
        let error = preview(&mut document, id, &mut library).unwrap_err();
        assert!(format!("{error:#}").contains("cycle"));
        assert!(Arc::ptr_eq(&accepted, &document.shared_source()));
        assert_eq!(
            (document.revision(), document.state_id()),
            (revision, state)
        );
        assert_eq!(document.history_len(), 1);
        assert!(document.is_dirty() && document.is_editing());
    }
    let child = committed.instances[0].id;
    assert!(document.detach_instance(child, &mut library).is_err());
    assert!(Arc::ptr_eq(&accepted, &document.shared_source()));
    document.cancel()?;
    let error = document.detach_instance(child, &mut library).unwrap_err();
    assert!(format!("{error:#}").contains("cycle"));
    assert_eq!(document.shared_source(), committed);
    assert_eq!(document.history_len(), 1);
    library.0.get_mut(&branch).unwrap().instances.clear();
    document.undo()?;
    assert!(!document.is_dirty());
    document.redo()?;
    assert_eq!(document.shared_source(), committed);
    assert_eq!(document.prefab_asset(), Some(root));
    document.expand(&mut library)?;
    Ok(())
}

#[test]
fn multiple_root_prefab_documents_save_and_reopen_without_remapping_source_identity() -> Result<()>
{
    let root = AssetId::new();
    let child = AssetId::new();
    let first = EntityData::new("first root");
    let second = EntityData::new("second root");
    let nested = EntityData::new("nested source");
    let (first_id, second_id, nested_id) = (first.id, second.id, nested.id);
    let mount = instance(child);
    let instance_id = mount.id;
    let content = ContentSource {
        base: SceneSnapshot {
            version: comet_scene::scene::SCENE_VERSION,
            entities: vec![first, second],
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            parent: Some(first_id),
            overrides: vec![FieldOverride {
                entity: nested_id,
                field: schema::TRANSLATION,
                value: FieldValue::Vec3([2., 0., 0.]),
            }],
            ..mount
        }],
        ..Default::default()
    };
    let mut library = Library(BTreeMap::from([(child, source(nested))]));
    let mut document = Document::open_prefab(root, content, &mut library)?;
    let world = document.world_id();
    document.begin()?;
    document.preview_name(first_id, "edited root".into())?;
    document.commit()?;
    let expected = document.shared_source();
    let expanded = document.expand(&mut library)?;
    assert_eq!(expanded.entities.len(), 3);
    for id in [first_id, second_id] {
        assert!(
            expanded
                .entities
                .iter()
                .any(|entity| entity.id == id && entity.parent.is_none())
        );
    }
    let nested = expanded
        .entities
        .iter()
        .find(|entity| entity.id == nested_id.in_instance(instance_id))
        .unwrap();
    assert_eq!(nested.parent, Some(first_id));
    assert_eq!(nested.transform.translation, [2., 0., 0.]);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("source.prefab");
    document.save(&path)?;
    assert!(!document.is_dirty());
    let bytes = fs::read(&path)?;
    assert!(
        !String::from_utf8(bytes.clone())?.contains(&root.to_string()),
        "validation context is not serialized"
    );
    let reopened = Document::open_prefab(root, ContentSource::from_json(&bytes)?, &mut library)?;
    assert_eq!(reopened.prefab_asset(), Some(root));
    assert_ne!(reopened.world_id(), world);
    assert_eq!(reopened.shared_source(), expected);
    assert_eq!(reopened.expand(&mut library)?, expanded);
    assert!(!reopened.is_dirty());
    assert_eq!(reopened.history_len(), 0);
    assert!(
        Document::load(&path)?.prefab_asset().is_none(),
        "generic load retains its Scene contract"
    );
    Ok(())
}

#[test]
fn detaching_preserves_expanded_identity_overrides_and_nested_source_scopes_in_one_history_change()
-> Result<()> {
    use comet_scene::{Animator, EntityRef, ScriptValue, SkinBinding};
    let outer_asset = AssetId::new();
    let inner_asset = AssetId::new();
    let empty_asset = AssetId::new();
    let outer = InstanceId::new();
    let mut first = EntityData::new("direct root");
    let mut second = EntityData::new("direct sibling");
    let first_id = first.id;
    let second_id = second.id;
    let bone = EntityId::new();
    let mut leaf = EntityData::new("nested root");
    // Equal UUIDs in different source namespaces must not become the same authored reference.
    leaf.id = second_id;
    leaf.links.target = Some(EntityRef::local(second_id));
    let mut nested = instance(inner_asset);
    nested.id = outer;
    nested.parent = Some(first_id);
    nested.overrides.push(FieldOverride {
        entity: second_id,
        field: schema::TARGET,
        value: FieldValue::Entity(Some(EntityRef::local(second_id))),
    });
    let nested_reference = EntityRef::in_instance(nested.id, second_id);
    first.links.target = Some(EntityRef::local(second_id));
    first.script = Some(Script::new(AssetId::new()));
    first.script.as_mut().unwrap().parameters.insert(
        "target".into(),
        ScriptValue::Entity(Some(nested_reference.clone())),
    );
    let mut animator = Animator::new(AssetId::new());
    animator.bindings.insert(bone, Some(nested_reference));
    first.animator = Some(animator);
    second.model = Some(Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    second.skin = Some(SkinBinding {
        skin: AssetId::new(),
        animator: Some(EntityRef::local(first_id)),
    });
    let mut parent = EntityData::new("author mount");
    parent.transform.translation = [10., 20., 30.];
    parent.transform.scale = [2., 3., 4.];
    let mut selected = instance(outer_asset);
    selected.id = outer;
    selected.parent = Some(parent.id);
    selected.overrides.push(FieldOverride {
        entity: first_id,
        field: schema::TRANSLATION,
        value: FieldValue::Vec3([3., 4., 5.]),
    });
    let mut observer = EntityData::new("author observer");
    observer.links.target = Some(EntityRef::in_instance(outer, first_id));
    let external = EntityRef::External {
        scene: AssetId::new(),
        entity: first_id,
    };
    observer.links.connections = vec![EntityRef::in_instance(outer, second_id), external.clone()];
    let observer_id = observer.id;
    let mut library = Library(BTreeMap::from([
        (inner_asset, source(leaf)),
        (
            outer_asset,
            ContentSource {
                base: SceneSnapshot {
                    version: comet_scene::scene::SCENE_VERSION,
                    entities: vec![first, second],
                },
                instances: vec![nested.clone()],
                ..Default::default()
            },
        ),
        (empty_asset, ContentSource::default()),
    ]));
    let library_before = library.0.clone();
    let authoring = ContentSource {
        base: SceneSnapshot {
            version: comet_scene::scene::SCENE_VERSION,
            entities: vec![parent, observer],
        },
        instances: vec![selected, instance(outer_asset)],
        ..Default::default()
    };
    let promoted = nested.id.in_instance(outer);
    let mut collision = authoring.clone();
    collision.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: promoted,
        prefab: empty_asset,
        parent: None,
        overrides: vec![],
    });
    let mut rejected = Document::open_source(collision)?;
    rejected.expand(&mut library)?;
    let rejected_before = rejected.shared_source();
    assert!(rejected.detach_instance(outer, &mut library).is_err());
    assert!(Arc::ptr_eq(&rejected_before, &rejected.shared_source()));
    assert_eq!(rejected.history_len(), 0);

    let mut document = Document::open_source(authoring)?;
    let before = document.shared_source();
    let expanded = document.expand(&mut library)?;
    let world = document.world_id();
    document.detach_instance(outer, &mut library)?;
    let after = document.shared_source();
    assert_eq!(document.expand(&mut library)?, expanded);
    assert_eq!(document.world_id(), world);
    assert_eq!(document.history_len(), 1);
    let retained = after
        .instances
        .iter()
        .find(|entry| entry.id == promoted)
        .unwrap();
    assert_eq!(retained.prefab, inner_asset);
    assert_eq!(retained.parent, Some(first_id.in_instance(outer)));
    assert_eq!(retained.overrides, nested.overrides);
    assert_eq!(after.instances.len(), 2);
    let observer = after
        .base
        .entities
        .iter()
        .find(|entity| entity.id == observer_id)
        .unwrap();
    assert_eq!(
        observer.links.target,
        Some(EntityRef::local(first_id.in_instance(outer)))
    );
    assert_eq!(
        observer.links.connections,
        [EntityRef::local(second_id.in_instance(outer)), external]
    );
    let detached = after
        .base
        .entities
        .iter()
        .find(|entity| entity.id == first_id.in_instance(outer))
        .unwrap();
    assert_eq!(detached.transform.translation, [3., 4., 5.]);
    assert_eq!(
        detached.animator.as_ref().unwrap().bindings[&bone],
        Some(EntityRef::in_instance(promoted, second_id))
    );
    assert_eq!(library.0, library_before);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("detached.scene");
    document.save(&path)?;
    assert_eq!(Document::load(&path)?.expand(&mut library)?, expanded);
    document.undo()?;
    assert_eq!(document.shared_source(), before);
    assert!(document.is_dirty());
    document.redo()?;
    assert_eq!(document.shared_source(), after);
    assert!(!document.is_dirty());
    // Only the lifted level is detached. Updating the nested source still reaches its instance.
    library.0.get_mut(&inner_asset).unwrap().base.entities[0].name = "updated nested root".into();
    assert_eq!(
        document
            .expand(&mut library)?
            .entities
            .iter()
            .find(|entity| entity.id == second_id.in_instance(promoted))
            .unwrap()
            .name,
        "updated nested root"
    );
    Ok(())
}
