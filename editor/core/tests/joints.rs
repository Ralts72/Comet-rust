use anyhow::{Result, bail};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::{Document, FieldEdit, FieldTarget, JointEdit};
use comet_scene::{
    BodyMotion, ContentSource, EntityData, EntityRef, HingeJoint, JointConnection, JointLimits,
    JointMotor, PhysicsBody, PointJoint, PrefabInstance, PrefabResolver, SceneSnapshot,
    SliderJoint,
    schema::{self, FieldValue},
};
use std::sync::Arc;

struct NoPrefabs;
impl PrefabResolver for NoPrefabs {
    fn prefab(&mut self, _: AssetId) -> Result<ContentSource> {
        bail!("unexpected prefab")
    }
}
fn body(name: &str) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    entity
}

#[test]
fn joint_components_share_transactions_and_preserve_history_savepoints_and_failed_previews()
-> Result<()> {
    let entity = body("owner");
    let id = entity.id;
    let mut doc = Document::open(SceneSnapshot {
        version: 2,
        entities: vec![entity],
    })?;
    let original = doc.shared_source();
    assert!(
        doc.preview_joint(
            id,
            JointEdit::Point(Some(PointJoint::default())),
            &mut NoPrefabs
        )
        .is_err()
    );
    doc.begin()?;
    doc.preview_joint(
        id,
        JointEdit::Point(Some(PointJoint::default())),
        &mut NoPrefabs,
    )?;
    doc.preview_joint(
        id,
        JointEdit::Hinge(Some(HingeJoint::default())),
        &mut NoPrefabs,
    )?;
    doc.preview_joint(
        id,
        JointEdit::Slider(Some(SliderJoint::default())),
        &mut NoPrefabs,
    )?;
    let accepted = doc.shared_source();
    let revision = doc.revision();
    let mut bad = PointJoint::default();
    bad.settings.connection = JointConnection::Body(EntityRef::local(id));
    assert!(
        doc.preview_joint(id, JointEdit::Point(Some(bad)), &mut NoPrefabs)
            .is_err()
    );
    let bad = PointJoint {
        anchor: [f32::NAN, 0., 0.],
        ..Default::default()
    };
    assert!(
        doc.preview_joint(id, JointEdit::Point(Some(bad)), &mut NoPrefabs)
            .is_err()
    );
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    assert_eq!(doc.revision(), revision);
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);
    assert_eq!(doc.source().base.version, 5);
    doc.undo()?;
    assert_eq!(doc.shared_source(), original);
    assert!(!doc.is_dirty());
    doc.redo()?;
    let temp = tempfile::tempdir()?;
    let file = temp.path().join("joints.scene");
    doc.save(&file)?;
    assert_eq!(Document::load(&file)?.shared_source(), accepted);
    doc.begin()?;
    doc.preview_joint(id, JointEdit::Point(None), &mut NoPrefabs)?;
    doc.commit()?;
    let source = doc.shared_source();
    let entity = &source.base.entities[0];
    assert!(
        entity.point_joint.is_none()
            && entity.hinge_joint.is_some()
            && entity.slider_joint.is_some()
    );
    doc.undo()?;
    assert_eq!(doc.shared_source(), accepted);
    assert!(!doc.is_dirty());
    Ok(())
}

#[test]
fn joint_candidates_reject_missing_bodies_stale_selection_and_invalid_targets_atomically()
-> Result<()> {
    let owner = body("owner");
    let id = owner.id;
    let other = EntityData::new("non-body");
    let other_id = other.id;
    let mut doc = Document::open(SceneSnapshot {
        entities: vec![owner, other],
        ..Default::default()
    })?;
    doc.begin()?;
    let original = doc.shared_source();
    for target in [other_id, EntityId::new()] {
        assert!(
            doc.preview_joint(
                target,
                JointEdit::Point(Some(PointJoint::default())),
                &mut NoPrefabs
            )
            .is_err()
        );
    }
    let mut joint = PointJoint::default();
    joint.settings.connection = JointConnection::Body(EntityRef::local(other_id));
    assert!(
        doc.preview_joint(id, JointEdit::Point(Some(joint)), &mut NoPrefabs)
            .is_err()
    );
    assert!(Arc::ptr_eq(&original, &doc.shared_source()));
    assert_eq!(doc.history_len(), 0);
    doc.cancel()?;
    Ok(())
}

#[test]
fn prefab_joint_fields_use_one_validated_override_candidate_and_do_not_change_structure()
-> Result<()> {
    struct Resolver(AssetId, ContentSource);
    impl PrefabResolver for Resolver {
        fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
            anyhow::ensure!(id == self.0, "unknown prefab");
            Ok(self.1.clone())
        }
    }
    let mut entity = body("prefab hinge");
    entity.hinge_joint = Some(HingeJoint::default());
    let entity_id = entity.id;
    let asset = AssetId::new();
    let instance = InstanceId::new();
    let mut resolver = Resolver(
        asset,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![entity],
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let source = ContentSource {
        base: SceneSnapshot {
            version: 2,
            entities: vec![],
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: asset,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    let mut doc = Document::open_source(source)?;
    let before = doc.shared_source();
    doc.begin()?;
    assert!(
        doc.preview_joint(
            entity_id.in_instance(instance),
            JointEdit::Hinge(None),
            &mut resolver
        )
        .is_err()
    );
    let target = FieldTarget::Prefab {
        instance,
        entity: entity_id,
    };
    doc.preview_fields(
        &[
            FieldEdit {
                target,
                field: schema::HINGE_LIMITS,
                value: FieldValue::JointLimits(Some(JointLimits {
                    min: -0.5,
                    max: 0.5,
                })),
            },
            FieldEdit {
                target,
                field: schema::HINGE_MOTOR,
                value: FieldValue::JointMotor(Some(JointMotor::Position {
                    target: 0.25,
                    frequency_hz: 2.,
                    damping_ratio: 1.,
                    max_effort: 10.,
                })),
            },
        ],
        &mut resolver,
    )?;
    let accepted = doc.shared_source();
    let invalid = [FieldEdit {
        target,
        field: schema::HINGE_LIMITS,
        value: FieldValue::JointLimits(Some(JointLimits { min: 2., max: 1. })),
    }];
    assert!(doc.preview_fields(&invalid, &mut resolver).is_err());
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);
    assert_eq!(doc.source().base.version, 5);
    let expanded = doc.source().expand(&mut resolver)?;
    let joint = expanded.entities[0].hinge_joint.as_ref().unwrap();
    assert_eq!(joint.limits.as_ref().unwrap().max, 0.5);
    assert!(joint.motor.is_some());
    assert!(doc.source().base.entities.is_empty());
    assert_eq!(doc.source().instances[0].overrides.len(), 2);
    doc.undo()?;
    assert_eq!(doc.shared_source(), before);
    Ok(())
}

#[test]
fn deleting_a_connected_body_becomes_unbound_and_undo_restores_the_body_reference() -> Result<()> {
    let target = body("target");
    let target_id = target.id;
    let mut owner = body("owner");
    owner.point_joint = Some(PointJoint::default());
    owner.point_joint.as_mut().unwrap().settings.connection =
        JointConnection::Body(EntityRef::local(target_id));
    let mut doc = Document::open(SceneSnapshot {
        entities: vec![owner, target],
        ..Default::default()
    })?;
    let before = doc.shared_source();
    doc.remove_subtrees(&[target_id], &mut NoPrefabs)?;
    assert_eq!(
        doc.source().base.entities[0]
            .point_joint
            .as_ref()
            .unwrap()
            .settings
            .connection,
        JointConnection::Unbound
    );
    doc.undo()?;
    assert_eq!(doc.shared_source(), before);
    Ok(())
}
