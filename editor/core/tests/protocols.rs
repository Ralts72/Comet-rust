use anyhow::{Context, Result};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::{Document, FieldEdit, FieldTarget, JointEdit};
use comet_scene::{
    Animator, BodyMotion, Character, ColliderShape, ContentSource, EntityData, EntityRef,
    JointConnection, JointSettings, Model, NavigationAgent, PhysicsBody, PointJoint,
    PrefabInstance, PrefabResolver, SceneSnapshot, Script, ScriptValue, SkinBinding, Surface,
    schema::{self, FieldValue},
};
use std::sync::Arc;

struct Library(AssetId, ContentSource);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        (id == self.0)
            .then(|| self.1.clone())
            .context("missing prefab")
    }
}

fn fixture(owner: EntityData) -> Result<(Document, Library, EntityId, InstanceId, EntityId)> {
    let asset = AssetId::new();
    let instance = InstanceId::new();
    let mut target = EntityData::new("prefab target");
    target.physics = Some(PhysicsBody::default());
    target.animator = Some(Animator::new(AssetId::new()));
    let target_id = target.id;
    let owner_id = owner.id;
    let source = ContentSource {
        base: SceneSnapshot {
            version: 2,
            entities: vec![owner],
        },
        instances: vec![PrefabInstance {
            id: instance,
            prefab: asset,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    let library = Library(
        asset,
        ContentSource {
            base: SceneSnapshot {
                version: 9,
                entities: vec![target],
            },
            ..Default::default()
        },
    );
    Ok((
        Document::open_source(source)?,
        library,
        owner_id,
        instance,
        target_id,
    ))
}

fn component(
    doc: &mut Document,
    owner: EntityId,
    kind: usize,
    target: EntityRef,
    library: &mut Library,
) -> Result<()> {
    match kind {
        0 => {
            let mut script = Script::new(AssetId::new());
            script
                .parameters
                .insert("target".into(), ScriptValue::Entity(Some(target)));
            doc.preview_script(owner, Some(script), library)
        }
        1 => doc.preview_navigation(
            owner,
            None,
            Some(NavigationAgent {
                target: Some(target),
                ..Default::default()
            }),
            library,
        ),
        2 => doc.preview_joint(
            owner,
            JointEdit::Point(Some(PointJoint {
                settings: JointSettings {
                    connection: JointConnection::Body(target),
                    ..Default::default()
                },
                ..Default::default()
            })),
            library,
        ),
        3 => {
            let mut animator = Animator::new(AssetId::new());
            animator.bindings.insert(EntityId::new(), Some(target));
            doc.preview_animation(owner, Some(animator), None, library)
        }
        4 => doc.preview_animation(
            owner,
            None,
            Some(SkinBinding {
                skin: AssetId::new(),
                animator: Some(target),
            }),
            library,
        ),
        _ => unreachable!(),
    }
}

#[test]
fn component_references_upgrade_by_value_and_preserve_history_on_failed_candidates() -> Result<()> {
    for kind in 0..5 {
        let mut owner = EntityData::new("owner");
        if kind == 1 {
            owner.physics = Some(PhysicsBody {
                motion: BodyMotion::Kinematic,
                shape: ColliderShape::Capsule {
                    half_height: 0.5,
                    radius: 0.3,
                },
                ..Default::default()
            });
            owner.character = Some(Character {
                player_controlled: false,
                ..Default::default()
            });
        } else if kind == 2 {
            owner.physics = Some(PhysicsBody {
                motion: BodyMotion::Dynamic,
                ..Default::default()
            });
        } else if kind == 4 {
            owner.model = Some(Model {
                mesh: AssetId::new(),
                surface: Surface {
                    material: AssetId::new(),
                    roughness: 0.5,
                },
            });
        }
        let (mut doc, mut library, owner, instance, target) = fixture(owner)?;
        let original = doc.shared_source();
        doc.begin()?;
        component(
            &mut doc,
            owner,
            kind,
            EntityRef::in_instance(instance, target),
            &mut library,
        )?;
        assert_eq!(doc.shared_source().base.version, 10);
        doc.expand(&mut library)?.validate_expanded()?;
        doc.commit()?;
        assert_eq!(doc.history_len(), 1);
        doc.undo()?;
        assert!(Arc::ptr_eq(&original, &doc.shared_source()));
        doc.redo()?;
        let accepted = doc.shared_source();
        let revision = doc.revision();
        doc.begin()?;
        assert!(
            component(
                &mut doc,
                owner,
                kind,
                EntityRef::in_instance(instance, EntityId::new()),
                &mut library
            )
            .is_err()
        );
        assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
        assert_eq!(doc.revision(), revision);
        doc.cancel()?;
        let mut legacy = doc.source();
        legacy.base.version = 9;
        assert!(Document::open_source(legacy).is_err());
    }
    Ok(())
}

#[test]
fn field_and_insert_commands_share_reference_protocol_without_upgrading_on_open() -> Result<()> {
    let (mut doc, mut library, owner, instance, target) = fixture(EntityData::new("owner"))?;
    let original = doc.shared_source();
    let reference = EntityRef::in_instance(instance, target);
    doc.begin()?;
    doc.preview_fields(
        &[FieldEdit {
            target: FieldTarget::Authored(owner),
            field: schema::TARGET,
            value: FieldValue::Entity(Some(reference.clone())),
        }],
        &mut library,
    )?;
    assert_eq!(doc.shared_source().base.version, 10);
    doc.commit()?;
    doc.undo()?;
    assert!(Arc::ptr_eq(&original, &doc.shared_source()));
    let mut inserted = EntityData::new("linked object");
    inserted.links.target = Some(reference);
    doc.insert_entity(inserted, &mut library)?;
    assert_eq!(doc.shared_source().base.version, 10);
    doc.undo()?;
    assert!(Arc::ptr_eq(&original, &doc.shared_source()));
    Ok(())
}
