use anyhow::Result;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{
    BodyMotion, Character, ColliderShape, ContentSource, EntityData, EntityRef, FieldOverride,
    FieldUpdate, HingeJoint, JointConnection, JointLimits, JointMotor, PhysicsBody, PointJoint,
    PoseUpdate, PrefabInstance, PrefabResolver, Scene, SceneSnapshot, SliderJoint,
    schema::{self, FieldValue},
};
use std::{collections::BTreeMap, sync::Arc};

fn body(name: &str) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.physics = Some(PhysicsBody::default());
    entity
}
fn pair() -> SceneSnapshot {
    let target = body("target");
    let mut owner = body("owner");
    owner.physics.as_mut().unwrap().motion = BodyMotion::Dynamic;
    let mut point = PointJoint::default();
    point.settings.connection = JointConnection::Body(EntityRef::local(target.id));
    owner.point_joint = Some(point);
    owner.hinge_joint = Some(HingeJoint::default());
    owner.slider_joint = Some(SliderJoint::default());
    SceneSnapshot {
        entities: vec![owner, target],
        ..Default::default()
    }
}

#[test]
fn joint_protocol_preserves_legacy_versions_and_all_three_ecs_components() -> Result<()> {
    for version in 1..=4 {
        let legacy = SceneSnapshot {
            version,
            entities: vec![EntityData::new("legacy")],
        };
        let bytes = legacy.to_json()?;
        assert!(!String::from_utf8_lossy(&bytes).contains("joint"));
        assert_eq!(SceneSnapshot::from_json(&bytes)?, legacy);
    }
    let mut original = pair();
    original.entities[0].hinge_joint.as_mut().unwrap().motor = Some(JointMotor::Position {
        target: 0.5,
        frequency_hz: 2.,
        damping_ratio: 1.,
        max_effort: 10.,
    });
    let id = original.entities[0].id;
    let loaded = SceneSnapshot::from_json(&original.to_json()?)?;
    let mut world = Scene::open(loaded)?;
    let handle = world.handle(id)?;
    world.update_poses(&[PoseUpdate {
        entity: id,
        translation: [1., 2., 3.],
        rotation: [0., 0., 0., 1.],
    }])?;
    original.entities[0].transform.translation = [1., 2., 3.];
    original.canonicalize();
    assert_eq!(world.snapshot(), original);
    let resolved = world.resolve(handle)?;
    assert!(resolved.point_joint.is_some());
    assert!(resolved.hinge_joint.is_some());
    assert!(resolved.slider_joint.is_some());
    let mut old_version = original.clone();
    old_version.version = 4;
    assert!(old_version.validate().is_err());
    let mut json: serde_json::Value = serde_json::from_slice(&original.to_json()?)?;
    let owner = json["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entity| entity["id"] == id.to_string())
        .unwrap();
    owner["hinge_joint"]["limits_frequency"] = 2.into();
    assert!(SceneSnapshot::from_json(&serde_json::to_vec(&json)?).is_err());
    Ok(())
}

#[test]
fn cloned_connections_remap_and_deleted_bodies_become_unbound() -> Result<()> {
    let mut snapshot = pair();
    let owner = snapshot.entities[0].id;
    let target = snapshot.entities[1].id;
    let external = EntityRef::External {
        scene: AssetId::new(),
        entity: target,
    };
    snapshot.entities[0]
        .slider_joint
        .as_mut()
        .unwrap()
        .settings
        .connection = JointConnection::Body(external.clone());
    let before = snapshot.clone();
    let map = snapshot.clone_subtrees(&[owner, target])?;
    let copied = snapshot
        .entities
        .iter()
        .find(|e| e.id == map[&owner])
        .unwrap();
    assert_eq!(
        copied.point_joint.as_ref().unwrap().settings.connection,
        JointConnection::Body(EntityRef::local(map[&target]))
    );
    assert_eq!(copied.hinge_joint, before.entities[0].hinge_joint);
    assert_eq!(copied.slider_joint, before.entities[0].slider_joint);
    snapshot.remove_subtrees(&[target])?;
    let surviving = snapshot.entities.iter().find(|e| e.id == owner).unwrap();
    assert!(surviving.point_joint.as_ref().unwrap().settings.enabled);
    assert_eq!(
        surviving.point_joint.as_ref().unwrap().settings.connection,
        JointConnection::Unbound
    );
    assert_eq!(
        surviving.slider_joint.as_ref().unwrap().settings.connection,
        JointConnection::Body(external.clone())
    );
    let copied = snapshot
        .entities
        .iter()
        .find(|e| e.id == map[&owner])
        .unwrap();
    assert_eq!(
        copied.point_joint.as_ref().unwrap().settings.connection,
        JointConnection::Body(EntityRef::local(map[&target]))
    );
    let EntityRef::External { scene, .. } = external else {
        unreachable!()
    };
    assert!(snapshot.asset_dependencies().contains(&scene));
    Ok(())
}

#[test]
fn invalid_field_candidates_preserve_ecs_identity_revision_and_shared_snapshot() -> Result<()> {
    let snapshot = pair();
    let id = snapshot.entities[0].id;
    let mut scene = Scene::open(snapshot)?;
    let before = scene.shared_snapshot();
    let handle = scene.handle(id)?;
    let revision = scene.revision();
    let invalid = [
        (schema::HINGE_ROTATION, FieldValue::Quat([0.; 4])),
        (
            schema::HINGE_LIMITS,
            FieldValue::JointLimits(Some(JointLimits { min: -4., max: 4. })),
        ),
        (
            schema::SLIDER_LIMITS,
            FieldValue::JointLimits(Some(JointLimits { min: 2., max: 1. })),
        ),
        (
            schema::HINGE_MOTOR,
            FieldValue::JointMotor(Some(JointMotor::Position {
                target: f32::INFINITY,
                frequency_hz: 2.,
                damping_ratio: 1.,
                max_effort: 1.,
            })),
        ),
        (
            schema::SLIDER_MOTOR,
            FieldValue::JointMotor(Some(JointMotor::Position {
                target: 0.,
                frequency_hz: f32::MAX,
                damping_ratio: 1.,
                max_effort: 1.,
            })),
        ),
        (
            schema::SLIDER_MOTOR,
            FieldValue::JointMotor(Some(JointMotor::Velocity {
                speed: f32::NAN,
                max_effort: 1.,
            })),
        ),
        (
            schema::POINT_CONNECTION,
            FieldValue::JointConnection(JointConnection::Body(EntityRef::local(id))),
        ),
        (
            schema::POINT_CONNECTION,
            FieldValue::JointConnection(JointConnection::Body(EntityRef::local(EntityId::new()))),
        ),
    ];
    for (field, value) in invalid {
        assert!(
            scene
                .set_fields(&[
                    FieldUpdate {
                        entity: id,
                        field: schema::POINT_ANCHOR,
                        value: FieldValue::Vec3([0.5, 0., 0.])
                    },
                    FieldUpdate {
                        entity: id,
                        field,
                        value
                    },
                ])
                .is_err()
        );
        assert_eq!(scene.revision(), revision);
        assert!(Arc::ptr_eq(&before, &scene.shared_snapshot()));
        assert_eq!(
            scene.resolve(handle)?,
            *before.entities.iter().find(|e| e.id == id).unwrap()
        );
    }
    scene.set_fields(&[
        FieldUpdate {
            entity: id,
            field: schema::HINGE_LIMITS,
            value: FieldValue::JointLimits(Some(JointLimits {
                min: -0.2,
                max: 0.4,
            })),
        },
        FieldUpdate {
            entity: id,
            field: schema::HINGE_MOTOR,
            value: FieldValue::JointMotor(Some(JointMotor::Velocity {
                speed: 1.,
                max_effort: 5.,
            })),
        },
        FieldUpdate {
            entity: id,
            field: schema::POINT_CONTACTS_ENABLED,
            value: FieldValue::Bool(true),
        },
    ])?;
    let accepted = scene.resolve(handle)?;
    assert!(
        accepted
            .point_joint
            .as_ref()
            .unwrap()
            .settings
            .contacts_enabled
    );
    assert!(accepted.hinge_joint.as_ref().unwrap().limits.is_some());
    scene.set_fields(&[
        FieldUpdate {
            entity: id,
            field: schema::HINGE_LIMITS,
            value: FieldValue::JointLimits(None),
        },
        FieldUpdate {
            entity: id,
            field: schema::HINGE_MOTOR,
            value: FieldValue::JointMotor(None),
        },
    ])?;
    let accepted = scene.resolve(handle)?;
    assert!(accepted.hinge_joint.as_ref().unwrap().limits.is_none());
    assert!(accepted.hinge_joint.as_ref().unwrap().motor.is_none());
    Ok(())
}

#[test]
fn hard_limits_accept_shifted_ranges_and_locked_axes_without_rewriting_authored_values()
-> Result<()> {
    use std::f32::consts::{PI, TAU};
    let snapshot = pair();
    let id = snapshot.entities[0].id;
    let mut scene = Scene::open(snapshot)?;
    let handle = scene.handle(id)?;
    for (field, ranges) in [
        (
            schema::HINGE_LIMITS,
            vec![
                (0.2, 1.),
                (-2., -0.2),
                (PI, 1.5 * PI),
                (-0.5 * PI, 1.5 * PI),
                (-9., -8.),
                (2., 2.),
                (f32::MAX, f32::MAX),
            ],
        ),
        (
            schema::SLIDER_LIMITS,
            vec![(1., 2.), (-3., -2.), (5., 5.), (0., f32::MAX)],
        ),
    ] {
        for (min, max) in ranges {
            let value = FieldValue::JointLimits(Some(JointLimits { min, max }));
            scene.set_fields(&[FieldUpdate {
                entity: id,
                field,
                value: value.clone(),
            }])?;
            let accepted = scene.resolve(handle)?;
            assert_eq!(schema::descriptor(field)?.read(&accepted), Some(value));
        }
    }
    for target in [4., -9., 100. * TAU, f32::MAX] {
        let value = FieldValue::JointMotor(Some(JointMotor::Position {
            target,
            frequency_hz: 2.,
            damping_ratio: 1.,
            max_effort: 1.,
        }));
        scene.set_fields(&[FieldUpdate {
            entity: id,
            field: schema::HINGE_MOTOR,
            value: value.clone(),
        }])?;
        let accepted = scene.resolve(handle)?;
        assert_eq!(
            schema::descriptor(schema::HINGE_MOTOR)?.read(&accepted),
            Some(value)
        );
    }
    let before = scene.shared_snapshot();
    let revision = scene.revision();
    for limits in [
        JointLimits {
            min: 0.,
            max: TAU + 0.1,
        },
        JointLimits { min: 1., max: 0. },
        JointLimits {
            min: f32::NEG_INFINITY,
            max: 1.,
        },
        JointLimits {
            min: 0.,
            max: f32::NAN,
        },
    ] {
        assert!(
            scene
                .set_fields(&[FieldUpdate {
                    entity: id,
                    field: schema::HINGE_LIMITS,
                    value: FieldValue::JointLimits(Some(limits))
                }])
                .is_err()
        );
        assert_eq!(scene.revision(), revision);
        assert!(Arc::ptr_eq(&before, &scene.shared_snapshot()));
    }
    assert_eq!(SceneSnapshot::from_json(&before.to_json()?)?, *before);
    Ok(())
}

#[test]
fn position_motor_frequency_accepts_each_representable_response() -> Result<()> {
    let snapshot = pair();
    let id = snapshot.entities[0].id;
    let mut scene = Scene::open(snapshot)?;
    for frequency_hz in [0.0001, 1e-23, 1e18] {
        scene.set_fields(&[FieldUpdate {
            entity: id,
            field: schema::HINGE_MOTOR,
            value: FieldValue::JointMotor(Some(JointMotor::Position {
                target: 0.,
                frequency_hz,
                damping_ratio: 1.,
                max_effort: 1.,
            })),
        }])?;
    }
    let before = scene.shared_snapshot();
    let revision = scene.revision();
    for frequency_hz in [
        0.,
        -1.,
        f32::MIN_POSITIVE,
        f32::MAX,
        f32::INFINITY,
        f32::NAN,
    ] {
        assert!(
            scene
                .set_fields(&[FieldUpdate {
                    entity: id,
                    field: schema::HINGE_MOTOR,
                    value: FieldValue::JointMotor(Some(JointMotor::Position {
                        target: 0.,
                        frequency_hz,
                        damping_ratio: 1.,
                        max_effort: 1.,
                    })),
                }])
                .is_err()
        );
        assert_eq!(scene.revision(), revision);
        assert!(Arc::ptr_eq(&before, &scene.shared_snapshot()));
    }
    assert!(
        scene
            .set_fields(&[FieldUpdate {
                entity: id,
                field: schema::HINGE_MOTOR,
                value: FieldValue::JointMotor(Some(JointMotor::Position {
                    target: 0.,
                    frequency_hz: 1.,
                    damping_ratio: f32::MAX,
                    max_effort: 1.,
                })),
            }])
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &scene.shared_snapshot()));
    Ok(())
}

#[test]
fn active_connections_require_a_dynamic_body_without_rejecting_unbound_configuration() -> Result<()>
{
    let target = body("fixed target");
    let target_id = target.id;
    let mut owner = body("fixed owner");
    let owner_id = owner.id;
    let mut point = PointJoint::default();
    point.settings.enabled = false;
    owner.point_joint = Some(point);
    let mut scene = Scene::open(SceneSnapshot {
        entities: vec![owner, target],
        ..Default::default()
    })?;
    let handle = scene.handle(owner_id)?;
    for connection in [
        JointConnection::World,
        JointConnection::Body(EntityRef::local(target_id)),
    ] {
        scene.set_fields(&[FieldUpdate {
            entity: owner_id,
            field: schema::POINT_CONNECTION,
            value: FieldValue::JointConnection(connection),
        }])?;
        let before = scene.shared_snapshot();
        let revision = scene.revision();
        assert!(
            scene
                .set_fields(&[FieldUpdate {
                    entity: owner_id,
                    field: schema::POINT_ENABLED,
                    value: FieldValue::Bool(true)
                }])
                .is_err()
        );
        assert_eq!(scene.revision(), revision);
        assert!(Arc::ptr_eq(&before, &scene.shared_snapshot()));
        assert!(!scene.resolve(handle)?.point_joint.unwrap().settings.enabled);
    }
    scene.set_fields(&[
        FieldUpdate {
            entity: owner_id,
            field: schema::POINT_ENABLED,
            value: FieldValue::Bool(true),
        },
        FieldUpdate {
            entity: owner_id,
            field: schema::POINT_CONNECTION,
            value: FieldValue::JointConnection(JointConnection::Unbound),
        },
    ])?;
    scene.set_fields(&[FieldUpdate {
        entity: owner_id,
        field: schema::POINT_CONNECTION,
        value: FieldValue::JointConnection(JointConnection::Body(EntityRef::External {
            scene: AssetId::new(),
            entity: target_id,
        })),
    }])?;
    let mut candidate = scene.snapshot();
    candidate
        .entities
        .iter_mut()
        .find(|e| e.id == target_id)
        .unwrap()
        .physics
        .as_mut()
        .unwrap()
        .motion = BodyMotion::Dynamic;
    candidate
        .entities
        .iter_mut()
        .find(|e| e.id == owner_id)
        .unwrap()
        .point_joint
        .as_mut()
        .unwrap()
        .settings
        .connection = JointConnection::Body(EntityRef::local(target_id));
    scene.replace(candidate)?;
    let before = scene.shared_snapshot();
    let mut invalid = before.as_ref().clone();
    invalid
        .entities
        .iter_mut()
        .find(|e| e.id == target_id)
        .unwrap()
        .physics
        .as_mut()
        .unwrap()
        .motion = BodyMotion::Fixed;
    assert!(scene.replace(invalid).is_err());
    assert!(Arc::ptr_eq(&before, &scene.shared_snapshot()));
    let mut removed = before.as_ref().clone();
    removed.remove_subtrees(&[target_id])?;
    scene.replace(removed)?;
    let handle = scene.handle(owner_id)?;
    assert_eq!(
        scene
            .resolve(handle)?
            .point_joint
            .unwrap()
            .settings
            .connection,
        JointConnection::Unbound
    );
    Ok(())
}

#[test]
fn owner_connected_body_and_scaled_anchor_rules_are_checked_before_publication() -> Result<()> {
    let snapshot = pair();
    let mut invalid = snapshot.clone();
    invalid.entities[0].physics = None;
    assert!(invalid.validate().is_err());
    invalid = snapshot.clone();
    invalid.entities[1].physics = None;
    assert!(invalid.validate().is_err());
    invalid = snapshot.clone();
    invalid.entities[0].point_joint.as_mut().unwrap().anchor = [f32::MAX, 0., 0.];
    assert!(invalid.validate().is_err());
    invalid = snapshot.clone();
    invalid.entities[1].transform.scale = [f32::MAX; 3];
    invalid.entities[0]
        .point_joint
        .as_mut()
        .unwrap()
        .connected_anchor = [2., 0., 0.];
    invalid.entities[1].physics.as_mut().unwrap().shape = ColliderShape::Box {
        half_extents: [f32::MIN_POSITIVE; 3],
    };
    assert!(invalid.validate().is_err());
    for index in [0, 1] {
        invalid = snapshot.clone();
        let target = &mut invalid.entities[index];
        target.physics.as_mut().unwrap().motion = BodyMotion::Kinematic;
        target.physics.as_mut().unwrap().shape = ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.25,
        };
        target.character = Some(Character::default());
        assert!(invalid.validate().is_err());
    }
    let mut world = Scene::open(snapshot.clone())?;
    let before = world.shared_snapshot();
    let mut invalid = snapshot.clone();
    invalid.entities[1].physics = None;
    assert!(world.replace(invalid).is_err());
    assert!(Arc::ptr_eq(&before, &world.shared_snapshot()));
    Ok(())
}

struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        Ok(self.0[&id].clone())
    }
}

#[test]
fn prefab_joint_overrides_keep_scoped_connections_and_inherit_unmodified_fields() -> Result<()> {
    let asset = AssetId::new();
    let instance = InstanceId::new();
    let prefab = ContentSource {
        base: pair(),
        ..Default::default()
    };
    let owner = prefab.base.entities[0].id;
    let target = prefab.base.entities[1].id;
    let mut library = Library(BTreeMap::from([(asset, prefab)]));
    let mut source = ContentSource {
        instances: vec![PrefabInstance {
            id: instance,
            prefab: asset,
            parent: None,
            overrides: vec![FieldOverride {
                entity: owner,
                field: schema::POINT_CONTACTS_ENABLED,
                value: FieldValue::Bool(true),
            }],
        }],
        ..Default::default()
    };
    source.base.version = 4;
    assert!(source.validate().is_err());
    source.base.version = 5;
    let first = source.expand(&mut library)?;
    let entity = first
        .entities
        .iter()
        .find(|e| e.id == owner.in_instance(instance))
        .unwrap();
    assert!(
        entity
            .point_joint
            .as_ref()
            .unwrap()
            .settings
            .contacts_enabled
    );
    assert_eq!(
        entity.point_joint.as_ref().unwrap().settings.connection,
        JointConnection::Body(EntityRef::local(target.in_instance(instance)))
    );
    library.0.get_mut(&asset).unwrap().base.entities[0]
        .point_joint
        .as_mut()
        .unwrap()
        .anchor = [0., 2., 0.];
    let second = source.expand(&mut library)?;
    let entity = second
        .entities
        .iter()
        .find(|e| e.id == owner.in_instance(instance))
        .unwrap();
    assert!(
        entity
            .point_joint
            .as_ref()
            .unwrap()
            .settings
            .contacts_enabled
    );
    assert_eq!(entity.point_joint.as_ref().unwrap().anchor, [0., 2., 0.]);
    let external_scene = AssetId::new();
    source.instances[0].overrides.push(FieldOverride {
        entity: owner,
        field: schema::POINT_CONNECTION,
        value: FieldValue::JointConnection(JointConnection::Body(EntityRef::External {
            scene: external_scene,
            entity: target,
        })),
    });
    assert!(source.dependencies().contains(&external_scene));
    let expanded = source.expand(&mut library)?;
    assert!(expanded.asset_dependencies().contains(&external_scene));
    assert_eq!(SceneSnapshot::from_json(&expanded.to_json()?)?, expanded);
    Ok(())
}
