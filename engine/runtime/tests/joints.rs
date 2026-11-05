use anyhow::Result;
use comet_core::{
    AssetId, EntityId,
    math::{Quat, Vec3},
};
use comet_runtime::{FIXED_DELTA, JointKind, JointStatus, PlaySession};
use comet_scene::{
    BodyMotion, ColliderShape, EntityData, EntityRef, HingeJoint, JointConnection, JointFrame,
    JointLimits, JointMotor, JointSettings, PhysicsBody, PointJoint, SceneSnapshot, SliderJoint,
    schema::{self, FieldValue},
};

fn body(name: &str, motion: BodyMotion, position: [f32; 3]) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.transform.translation = position;
    entity.physics = Some(PhysicsBody {
        motion,
        shape: ColliderShape::Box {
            half_extents: [0.2; 3],
        },
        mask: 0,
        ..Default::default()
    });
    entity
}

fn scene(entities: Vec<EntityData>) -> SceneSnapshot {
    SceneSnapshot {
        entities,
        ..Default::default()
    }
}

fn ticks(play: &mut PlaySession, count: usize) -> Result<()> {
    for _ in 0..count {
        play.advance(FIXED_DELTA)?;
    }
    Ok(())
}

fn entity(play: &mut PlaySession, id: EntityId) -> EntityData {
    play.snapshot()
        .entities
        .into_iter()
        .find(|entity| entity.id == id)
        .unwrap()
}

fn anchor(entity: &EntityData, local: [f32; 3]) -> Vec3 {
    Vec3::from_array(entity.transform.translation)
        + Quat::from_array(entity.transform.rotation)
            * (Vec3::from_array(local) * Vec3::from_array(entity.transform.scale))
}

fn angle_x(entity: &EntityData) -> f32 {
    let rotation = Quat::from_array(entity.transform.rotation);
    2. * rotation.x.atan2(rotation.w)
}

fn hinge(motor: Option<JointMotor>) -> EntityData {
    let mut entity = body("hinge", BodyMotion::Dynamic, [0., 2., 0.]);
    entity.hinge_joint = Some(HingeJoint {
        connected_frame: JointFrame {
            anchor: entity.transform.translation,
            ..Default::default()
        },
        motor,
        ..Default::default()
    });
    entity
}

fn slider(motor: Option<JointMotor>) -> EntityData {
    let mut entity = body("slider", BodyMotion::Dynamic, [0., 2., 0.]);
    entity.slider_joint = Some(SliderJoint {
        connected_frame: JointFrame {
            anchor: entity.transform.translation,
            ..Default::default()
        },
        motor,
        ..Default::default()
    });
    entity
}

#[test]
fn point_attachments_apply_each_bodys_scale_and_rotation_to_its_local_anchor() -> Result<()> {
    for to_world in [true, false] {
        let mut fixed = body("support", BodyMotion::Fixed, [3., 5., -2.]);
        fixed.transform.scale = [3., 2., 0.5];
        fixed.transform.rotation = Quat::from_rotation_z(0.4).to_array();
        let connected = [0.3, -0.2, 0.4];
        let target = anchor(&fixed, connected);
        let local = [0.4, 0.6, -0.1];
        let mut owner = body("pendulum", BodyMotion::Dynamic, [0.; 3]);
        owner.transform.scale = [0.5, 2., 3.];
        owner.transform.rotation = Quat::from_rotation_y(-0.7).to_array();
        owner.transform.translation = (target - anchor(&owner, local)).to_array();
        owner.point_joint = Some(PointJoint {
            settings: JointSettings {
                connection: if to_world {
                    JointConnection::World
                } else {
                    JointConnection::Body(EntityRef::local(fixed.id))
                },
                ..Default::default()
            },
            anchor: local,
            connected_anchor: if to_world {
                target.to_array()
            } else {
                connected
            },
        });
        let id = owner.id;
        let mut play = PlaySession::start(scene(vec![owner, fixed]))?;
        ticks(&mut play, 120)?;
        assert_eq!(
            play.joint_status(id, JointKind::Point),
            Some(JointStatus::Active)
        );
        let actual = anchor(&entity(&mut play, id), local);
        assert!(
            (actual - target).length() < 0.025,
            "world={to_world} target={target:?} actual={actual:?}"
        );
    }
    Ok(())
}

#[test]
fn axial_frames_keep_the_authored_zero_reference_and_rotate_the_principal_axis() -> Result<()> {
    let world_rotation = Quat::from_rotation_z(0.6) * Quat::from_rotation_y(-0.4);
    let local_rotation = Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-0.2);
    let mut owner = hinge(Some(JointMotor::Position {
        target: 0.45,
        frequency_hz: 5.,
        damping_ratio: 1.,
        max_effort: 100.,
    }));
    owner.transform.rotation = (world_rotation * local_rotation.inverse()).to_array();
    owner.transform.scale = [2., 0.5, 3.];
    let config = owner.hinge_joint.as_mut().unwrap();
    config.frame.rotation = local_rotation.to_array();
    config.connected_frame.rotation = world_rotation.to_array();
    let id = owner.id;
    let mut play = PlaySession::start(scene(vec![owner]))?;
    ticks(&mut play, 120)?;
    let actual = entity(&mut play, id);
    let relative =
        world_rotation.inverse() * Quat::from_array(actual.transform.rotation) * local_rotation;
    let expected = Quat::from_rotation_x(0.45);
    assert!(
        relative.dot(expected).abs() > 0.9999,
        "relative={relative:?}"
    );
    assert!(
        (Vec3::from_array(actual.transform.translation) - Vec3::new(0., 2., 0.)).length() < 0.025
    );
    Ok(())
}

#[test]
fn a_body_connected_slider_uses_both_scaled_anchors_and_local_frame_orientations() -> Result<()> {
    let mut support = body("support", BodyMotion::Fixed, [3., 4., -2.]);
    support.transform.scale = [2., 0.5, 4.];
    support.transform.rotation = Quat::from_rotation_y(-0.4).to_array();
    let connected_local = Quat::from_rotation_y(1.);
    let connected_rotation = Quat::from_array(support.transform.rotation) * connected_local;
    let connected_anchor = [0.4, -0.3, 0.2];
    let origin = anchor(&support, connected_anchor);
    let owner_local = Quat::from_rotation_z(0.3) * Quat::from_rotation_x(-0.2);
    let mut owner = body("slider", BodyMotion::Dynamic, [0.; 3]);
    owner.transform.scale = [0.7, 2., 3.];
    owner.transform.rotation = (connected_rotation * owner_local.inverse()).to_array();
    let local_anchor = [0.3, 0.2, -0.1];
    owner.transform.translation = (origin - anchor(&owner, local_anchor)).to_array();
    owner.slider_joint = Some(SliderJoint {
        settings: JointSettings {
            connection: JointConnection::Body(EntityRef::local(support.id)),
            ..Default::default()
        },
        frame: JointFrame {
            anchor: local_anchor,
            rotation: owner_local.to_array(),
        },
        connected_frame: JointFrame {
            anchor: connected_anchor,
            rotation: connected_local.to_array(),
        },
        motor: Some(JointMotor::Position {
            target: 0.45,
            frequency_hz: 5.,
            damping_ratio: 1.,
            max_effort: 100.,
        }),
        ..Default::default()
    });
    let id = owner.id;
    let mut play = PlaySession::start(scene(vec![owner, support]))?;
    ticks(&mut play, 120)?;
    let actual = entity(&mut play, id);
    let expected_anchor = origin + connected_rotation * Vec3::X * 0.45;
    assert!((anchor(&actual, local_anchor) - expected_anchor).length() < 0.025);
    let frame_rotation = Quat::from_array(actual.transform.rotation) * owner_local;
    assert!(frame_rotation.dot(connected_rotation).abs() > 0.9999);
    Ok(())
}

#[test]
fn hinge_and_slider_hard_limits_stop_velocity_motors_in_both_directions() -> Result<()> {
    for sign in [-1., 1.] {
        let motor = Some(JointMotor::Velocity {
            speed: 2. * sign,
            max_effort: 10.,
        });
        let mut hinge = hinge(motor.clone());
        hinge.hinge_joint.as_mut().unwrap().limits = Some(JointLimits {
            min: -0.3,
            max: 0.55,
        });
        let mut slider = slider(motor);
        slider.transform.translation[2] = 4.;
        let config = slider.slider_joint.as_mut().unwrap();
        config.connected_frame.anchor[2] = 4.;
        config.limits = Some(JointLimits {
            min: -0.3,
            max: 0.55,
        });
        let (hinge_id, slider_id) = (hinge.id, slider.id);
        let mut play = PlaySession::start(scene(vec![hinge, slider]))?;
        ticks(&mut play, 120)?;
        let expected = if sign < 0. { -0.3 } else { 0.55 };
        let angular = angle_x(&entity(&mut play, hinge_id));
        let linear = entity(&mut play, slider_id).transform.translation[0];
        assert!(
            (angular - expected).abs() < 0.025,
            "sign={sign} angular={angular}"
        );
        assert!(
            (linear - expected).abs() < 0.025,
            "sign={sign} linear={linear}"
        );
    }
    Ok(())
}

#[test]
fn motors_follow_signed_targets_and_zero_effort_cannot_accelerate_a_body() -> Result<()> {
    for angular in [false, true] {
        for position_motor in [false, true] {
            for sign in [-1., 1.] {
                let mut results = vec![];
                for max_effort in [0., 0.001, 100.] {
                    let motor = if position_motor {
                        JointMotor::Position {
                            target: sign * 0.6,
                            frequency_hz: 4.,
                            damping_ratio: 1.,
                            max_effort,
                        }
                    } else {
                        JointMotor::Velocity {
                            speed: sign,
                            max_effort,
                        }
                    };
                    let owner = if angular {
                        hinge(Some(motor))
                    } else {
                        slider(Some(motor))
                    };
                    let id = owner.id;
                    let mut play = PlaySession::start(scene(vec![owner]))?;
                    ticks(&mut play, 30)?;
                    let actual = entity(&mut play, id);
                    let value = if angular {
                        angle_x(&actual)
                    } else {
                        actual.transform.translation[0]
                    };
                    results.push(sign * value);
                }
                assert!(
                    results[0].abs() < 1e-5,
                    "angular={angular} position={position_motor} values={results:?}"
                );
                assert!(
                    results[1] >= -1e-5 && results[1] < 0.01,
                    "weak motor exceeded its effort: {results:?}"
                );
                let target = if position_motor { 0.6 } else { 0.5 };
                assert!(
                    (results[2] - target).abs() < 0.04,
                    "angular={angular} position={position_motor} values={results:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn shifted_and_zero_width_limits_clamp_position_targets() -> Result<()> {
    for angular in [false, true] {
        for (min, max, target, expected) in [
            (0.4, 0.9, -0.5, 0.4),
            (-0.9, -0.4, 0.5, -0.4),
            (0.35, 0.35, 2., 0.35),
        ] {
            let motor = Some(JointMotor::Position {
                target,
                frequency_hz: 5.,
                damping_ratio: 1.,
                max_effort: 100.,
            });
            let mut owner = if angular { hinge(motor) } else { slider(motor) };
            let limits = Some(JointLimits { min, max });
            if angular {
                owner.hinge_joint.as_mut().unwrap().limits = limits;
            } else {
                owner.slider_joint.as_mut().unwrap().limits = limits;
            }
            let id = owner.id;
            let mut play = PlaySession::start(scene(vec![owner]))?;
            ticks(&mut play, 120)?;
            let actual = entity(&mut play, id);
            if angular {
                assert!(
                    Quat::from_array(actual.transform.rotation)
                        .dot(Quat::from_rotation_x(expected))
                        .abs()
                        > 0.9999,
                    "range={min}..{max} actual={:?}",
                    actual.transform.rotation
                );
            } else {
                assert!(
                    (actual.transform.translation[0] - expected).abs() < 0.025,
                    "range={min}..{max} actual={:?}",
                    actual.transform.translation
                );
            }
        }
    }
    Ok(())
}

#[test]
fn hinge_intervals_crossing_pi_and_multi_turn_targets_use_equivalent_orientations() -> Result<()> {
    let many_turns = (std::f64::consts::TAU * 10_000. + 0.4) as f32;
    for (limits, target, start, expected) in [
        (Some(JointLimits { min: 2.8, max: 3.5 }), 3.4, 3., 3.4),
        (Some(JointLimits { min: 2.8, max: 3.5 }), 10., 3., 3.5),
        (None, many_turns, 0., many_turns),
        (None, -many_turns, 0., -many_turns),
    ] {
        let mut owner = hinge(Some(JointMotor::Position {
            target,
            frequency_hz: 5.,
            damping_ratio: 1.,
            max_effort: 100.,
        }));
        owner.transform.rotation = Quat::from_rotation_x(start).to_array();
        owner.hinge_joint.as_mut().unwrap().limits = limits;
        let id = owner.id;
        let mut play = PlaySession::start(scene(vec![owner]))?;
        ticks(&mut play, 120)?;
        let actual = entity(&mut play, id);
        assert!(
            Quat::from_array(actual.transform.rotation)
                .dot(Quat::from_rotation_x(expected))
                .abs()
                > 0.9999,
            "target={target} expected={expected} actual={:?}",
            actual.transform.rotation
        );
        assert_eq!(
            play.joint_status(id, JointKind::Hinge),
            Some(JointStatus::Active)
        );
    }
    // Very large finite author values must not overflow Rapier's f32 interval
    // centre calculation or produce a non-finite solver pose.
    for target in [f32::MAX, -f32::MAX] {
        let mut owner = hinge(Some(JointMotor::Position {
            target,
            frequency_hz: 5.,
            damping_ratio: 1.,
            max_effort: 100.,
        }));
        owner.hinge_joint.as_mut().unwrap().limits = Some(JointLimits {
            min: target,
            max: target,
        });
        let id = owner.id;
        let mut play = PlaySession::start(scene(vec![owner]))?;
        ticks(&mut play, 120)?;
        play.snapshot().validate()?;
        assert_eq!(
            play.joint_status(id, JointKind::Hinge),
            Some(JointStatus::Active)
        );
    }
    Ok(())
}

#[test]
fn disabled_and_unbound_joints_do_not_attach_to_world_and_external_bodies_are_rejected()
-> Result<()> {
    let mut owner = body("disconnected", BodyMotion::Dynamic, [0., 2., 0.]);
    owner.point_joint = Some(PointJoint {
        settings: JointSettings {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    });
    owner.hinge_joint = Some(HingeJoint {
        settings: JointSettings {
            connection: JointConnection::Unbound,
            ..Default::default()
        },
        ..Default::default()
    });
    let id = owner.id;
    let mut play = PlaySession::start(scene(vec![owner.clone()]))?;
    ticks(&mut play, 30)?;
    assert_eq!(
        play.joint_status(id, JointKind::Point),
        Some(JointStatus::Disabled)
    );
    assert_eq!(
        play.joint_status(id, JointKind::Hinge),
        Some(JointStatus::Unbound)
    );
    assert_eq!(play.joint_status(id, JointKind::Slider), None);
    assert!(entity(&mut play, id).transform.translation[1] < 1.);
    owner.hinge_joint.as_mut().unwrap().settings.connection =
        JointConnection::Body(EntityRef::External {
            scene: AssetId::new(),
            entity: EntityId::new(),
        });
    let source = scene(vec![owner]);
    source.validate()?;
    let error = PlaySession::start(source)
        .err()
        .expect("external body must not attach to this runtime world");
    assert!(
        error.to_string().contains("different runtime worlds"),
        "{error:#}"
    );
    Ok(())
}

#[test]
fn coexisting_joint_configuration_is_immutable_and_restart_rebuilds_simulation() -> Result<()> {
    let mut owner = hinge(Some(JointMotor::Velocity {
        speed: 0.7,
        max_effort: 2.,
    }));
    owner.point_joint = Some(PointJoint {
        settings: JointSettings {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    });
    owner.slider_joint = Some(SliderJoint {
        settings: JointSettings {
            connection: JointConnection::Unbound,
            ..Default::default()
        },
        ..Default::default()
    });
    let id = owner.id;
    let source = scene(vec![owner]);
    let before = source.to_json()?;
    let mut play = PlaySession::start(source.clone())?;
    let initial = play.snapshot();
    for field in [
        schema::POINT_ENABLED,
        schema::HINGE_ENABLED,
        schema::SLIDER_ENABLED,
    ] {
        assert!(
            play.set_field(&[id], field, FieldValue::Bool(false))
                .is_err()
        );
        assert_eq!(play.snapshot(), initial);
    }
    ticks(&mut play, 3)?;
    assert!(angle_x(&entity(&mut play, id)) > 0.);
    play.set_paused(true);
    let paused = play.snapshot();
    play.advance(5.)?;
    assert_eq!(play.snapshot(), paused);
    let old_world = play.world_id();
    play.restart()?;
    assert_ne!(play.world_id(), old_world);
    assert!(play.is_paused());
    assert_eq!(play.snapshot(), initial);
    assert_eq!(
        play.joint_status(id, JointKind::Point),
        Some(JointStatus::Disabled)
    );
    assert_eq!(
        play.joint_status(id, JointKind::Hinge),
        Some(JointStatus::Active)
    );
    assert_eq!(
        play.joint_status(id, JointKind::Slider),
        Some(JointStatus::Unbound)
    );
    play.single_step()?;
    assert_eq!(play.steps(), 1);
    assert!(angle_x(&entity(&mut play, id)) > 0.);
    assert_eq!(source.to_json()?, before);
    Ok(())
}

#[test]
fn all_three_joint_types_can_simultaneously_constrain_the_same_body() -> Result<()> {
    let mut owner = hinge(None);
    let origin = owner.transform.translation;
    owner.point_joint = Some(PointJoint {
        connected_anchor: origin,
        ..Default::default()
    });
    owner.slider_joint = Some(SliderJoint {
        connected_frame: JointFrame {
            anchor: origin,
            ..Default::default()
        },
        ..Default::default()
    });
    let id = owner.id;
    let mut play = PlaySession::start(scene(vec![owner]))?;
    play.apply_impulse(id, [1., 2., 3.])?;
    ticks(&mut play, 120)?;
    for kind in [JointKind::Point, JointKind::Hinge, JointKind::Slider] {
        assert_eq!(play.joint_status(id, kind), Some(JointStatus::Active));
    }
    let actual = entity(&mut play, id);
    assert!(
        (Vec3::from_array(actual.transform.translation) - Vec3::from_array(origin)).length()
            < 0.025
    );
    assert!(
        Quat::from_array(actual.transform.rotation)
            .dot(Quat::IDENTITY)
            .abs()
            > 0.9999
    );
    Ok(())
}

#[test]
fn connected_contact_setting_controls_real_collision_response_along_the_free_slider_axis()
-> Result<()> {
    for contacts_enabled in [false, true] {
        let mut support = body("support", BodyMotion::Fixed, [0., 2., 0.]);
        let mut owner = body("overlapping slider", BodyMotion::Dynamic, [0.2, 2., 0.]);
        for entity in [&mut support, &mut owner] {
            let physics = entity.physics.as_mut().unwrap();
            physics.shape = ColliderShape::Box {
                half_extents: [0.5; 3],
            };
            physics.mask = u32::MAX;
        }
        owner.slider_joint = Some(SliderJoint {
            settings: JointSettings {
                connection: JointConnection::Body(EntityRef::local(support.id)),
                contacts_enabled,
                ..Default::default()
            },
            ..Default::default()
        });
        let id = owner.id;
        let mut play = PlaySession::start(scene(vec![support, owner]))?;
        ticks(&mut play, 90)?;
        assert_eq!(
            play.joint_status(id, JointKind::Slider),
            Some(JointStatus::Active)
        );
        let position = entity(&mut play, id).transform.translation;
        assert!(
            if contacts_enabled {
                position[0] > 0.98
            } else {
                (position[0] - 0.2).abs() < 1e-4
            },
            "contacts_enabled={contacts_enabled} position={position:?}"
        );
        assert!((position[1] - 2.).abs() < 0.025 && position[2].abs() < 0.025);
    }
    Ok(())
}

#[test]
fn joint_simulation_is_independent_of_render_rate_and_source_order() -> Result<()> {
    let mut support = body("point", BodyMotion::Dynamic, [0., 4., 0.]);
    support.point_joint = Some(PointJoint {
        connected_anchor: support.transform.translation,
        ..Default::default()
    });
    let hinge = hinge(Some(JointMotor::Velocity {
        speed: 0.7,
        max_effort: 2.,
    }));
    let slider = slider(Some(JointMotor::Position {
        target: -0.5,
        frequency_hz: 3.,
        damping_ratio: 1.,
        max_effort: 20.,
    }));
    let (point_id, hinge_id, slider_id) = (support.id, hinge.id, slider.id);
    let source = scene(vec![support, hinge, slider]);
    let mut expected = None;
    for fps in [30, 60, 120] {
        let mut candidate = source.clone();
        if fps == 120 {
            candidate.entities.reverse();
        }
        let mut play = PlaySession::start(candidate)?;
        play.apply_impulse(point_id, [2., 0., 0.])?;
        for _ in 0..fps {
            play.advance(1. / f64::from(fps))?;
        }
        assert_eq!(play.steps(), 60);
        for (id, kind) in [
            (point_id, JointKind::Point),
            (hinge_id, JointKind::Hinge),
            (slider_id, JointKind::Slider),
        ] {
            assert_eq!(play.joint_status(id, kind), Some(JointStatus::Active));
        }
        let actual = play.snapshot().to_json()?;
        if let Some(expected) = &expected {
            assert_eq!(&actual, expected);
        } else {
            expected = Some(actual);
        }
    }
    Ok(())
}
