use super::*;
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::{InstanceId, math::Vec3};
use comet_runtime::JointKind;
use comet_scene::{
    BodyMotion, ContentSource, EntityRef, HingeJoint, JointConnection, JointFrame, JointLimits,
    JointMotor, PointJoint, PrefabInstance, SliderJoint,
    schema::{self, FieldKey, FieldValue},
};

fn fixture(kind: JointKind) -> Result<Group> {
    let mut f = Group::new()?;
    f.physics(false)?;
    let mut source = f.editor.document.source();
    for (index, id) in f.ids.iter().enumerate() {
        let entity = source
            .base
            .entities
            .iter_mut()
            .find(|e| e.id == *id)
            .unwrap();
        entity.physics.as_mut().unwrap().motion = BodyMotion::Dynamic;
        let frame = JointFrame {
            anchor: [0.2 + index as f32, 0.3, 0.4],
            rotation: (-Quat::from_euler(EulerRot::XYZ, 0.1, 0.2 + index as f32 * 0.1, 0.3))
                .to_array(),
        };
        match kind {
            JointKind::Point => {
                entity.point_joint = Some(PointJoint {
                    anchor: frame.anchor,
                    ..Default::default()
                })
            }
            JointKind::Hinge => {
                entity.hinge_joint = Some(HingeJoint {
                    frame,
                    limits: Some(JointLimits {
                        min: -1. - index as f32,
                        max: 1. + index as f32,
                    }),
                    motor: Some(JointMotor::Position {
                        target: 0.1,
                        frequency_hz: 2. + index as f32,
                        damping_ratio: 0.5 + index as f32,
                        max_effort: 4. + index as f32,
                    }),
                    ..Default::default()
                })
            }
            JointKind::Slider => {
                entity.slider_joint = Some(SliderJoint {
                    frame,
                    limits: Some(JointLimits {
                        min: -1. - index as f32,
                        max: 1. + index as f32,
                    }),
                    motor: Some(JointMotor::Velocity {
                        speed: 1. + index as f32,
                        max_effort: 4. + index as f32,
                    }),
                    ..Default::default()
                })
            }
        }
    }
    f.editor.document = Document::open_source(source)?;
    Ok(f)
}
fn values(f: &mut Group, field: FieldKey) -> Result<[FieldValue; 2]> {
    let snapshot = f.editor.snapshot(&mut f.assets)?;
    Ok(f.ids.map(|id| {
        schema::descriptor(field)
            .unwrap()
            .read(snapshot.entities.iter().find(|e| e.id == id).unwrap())
            .unwrap()
    }))
}
fn axis(f: &mut Group, heading: &str, axis: &str) -> Result<Pos2> {
    f.reveal(|output| {
        let values = texts(output);
        let heading = values
            .iter()
            .find(|(text, pos)| text.starts_with(heading) && pos.x > 970.)?
            .1;
        values
            .into_iter()
            .filter(|(text, pos)| text.starts_with(axis) && pos.x > 970. && pos.y > heading.y)
            .min_by(|(_, a), (_, b)| a.y.total_cmp(&b.y))
            .map(|(_, pos)| pos)
    })
}

#[test]
fn common_joint_axes_limits_and_drives_preserve_other_parameters_and_atomic_gestures() -> Result<()>
{
    let mut f = fixture(JointKind::Hinge)?;
    let original = f.editor.document.shared_source();
    let before = values(&mut f, schema::HINGE_ANCHOR)?;
    let pos = axis(&mut f, "本地锚点", "X")?;
    f.type_at(pos, "1.25");
    f.confirm();
    for (initial, actual) in before
        .into_iter()
        .zip(values(&mut f, schema::HINGE_ANCHOR)?)
    {
        let FieldValue::Vec3(mut expected) = initial else {
            unreachable!()
        };
        expected[0] = 1.25;
        assert_eq!(actual, FieldValue::Vec3(expected));
    }
    assert_eq!(f.editor.document.history_len(), 1);
    let rotations = values(&mut f, schema::HINGE_ROTATION)?;
    let pos = axis(&mut f, "自身坐标朝向", "X")?;
    f.type_at(pos, "30");
    f.confirm();
    for (before, after) in rotations
        .into_iter()
        .zip(values(&mut f, schema::HINGE_ROTATION)?)
    {
        let (FieldValue::Quat(before), FieldValue::Quat(after)) = (before, after) else {
            unreachable!()
        };
        let (_, y, z) = Quat::from_array(before).to_euler(EulerRot::XYZ);
        assert!(
            Quat::from_array(after)
                .dot(Quat::from_euler(EulerRot::XYZ, 30f32.to_radians(), y, z))
                .abs()
                > 0.99999
        );
    }
    f.type_scalar("最小角度", "-30")?;
    f.confirm();
    assert_eq!(
        values(&mut f, schema::HINGE_LIMITS)?,
        [
            FieldValue::JointLimits(Some(JointLimits {
                min: -30f32.to_radians(),
                max: 1.
            })),
            FieldValue::JointLimits(Some(JointLimits {
                min: -30f32.to_radians(),
                max: 2.
            }))
        ]
    );
    f.type_scalar("驱动频率", "3.5")?;
    f.confirm();
    assert_eq!(
        values(&mut f, schema::HINGE_MOTOR)?,
        [
            FieldValue::JointMotor(Some(JointMotor::Position {
                target: 0.1,
                frequency_hz: 3.5,
                damping_ratio: 0.5,
                max_effort: 4.
            })),
            FieldValue::JointMotor(Some(JointMotor::Position {
                target: 0.1,
                frequency_hz: 3.5,
                damping_ratio: 1.5,
                max_effort: 5.
            }))
        ]
    );
    let accepted = f.editor.document.shared_source();
    let end = f.drag_scalar("最大力矩", 20.)?;
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![
        button(end, false),
        key(egui::Key::Escape, false, Default::default()),
    ]);
    assert_eq!(f.editor.document.shared_source(), accepted);
    f.type_scalar("最小角度", "90")?;
    f.confirm();
    assert!(f.editor.error.is_some());
    assert_eq!(f.editor.document.shared_source(), accepted);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), accepted);
    for _ in 0..4 {
        f.editor.document.undo()?;
    }
    assert_eq!(f.editor.document.shared_source(), original);
    for _ in 0..4 {
        f.editor.document.redo()?;
    }
    assert_eq!(f.editor.document.shared_source(), accepted);

    let mut f = fixture(JointKind::Slider)?;
    f.editor.edit(
        f.ids[1],
        schema::SLIDER_MOTOR,
        FieldValue::JointMotor(Some(JointMotor::Position {
            target: 0.7,
            frequency_hz: 3.,
            damping_ratio: 0.8,
            max_effort: 7.,
        })),
        &mut f.assets,
    )?;
    f.editor.commit()?;
    f.select_reference("驱动模式", "位置驱动")?;
    assert_eq!(
        values(&mut f, schema::SLIDER_MOTOR)?,
        [
            FieldValue::JointMotor(Some(JointMotor::Position {
                target: 0.,
                frequency_hz: 2.,
                damping_ratio: 1.,
                max_effort: 10.
            })),
            FieldValue::JointMotor(Some(JointMotor::Position {
                target: 0.7,
                frequency_hz: 3.,
                damping_ratio: 0.8,
                max_effort: 7.
            }))
        ]
    );
    f.type_scalar("目标行程", "0.4")?;
    f.confirm();
    let before = values(&mut f, schema::SLIDER_MOTOR)?;
    let history = f.editor.document.history_len();
    f.select_reference("驱动模式", "位置驱动")?;
    assert_eq!(values(&mut f, schema::SLIDER_MOTOR)?, before);
    assert_eq!(f.editor.document.history_len(), history);
    f.editor.edit(
        f.ids[0],
        schema::SLIDER_LIMITS,
        FieldValue::JointLimits(None),
        &mut f.assets,
    )?;
    f.editor.commit()?;
    f.editor.selection.activate(f.ids[0]);
    f.click_flag("行程限位")?;
    assert_eq!(
        values(&mut f, schema::SLIDER_LIMITS)?,
        [
            FieldValue::JointLimits(Some(JointLimits { min: -1., max: 1. })),
            FieldValue::JointLimits(Some(JointLimits { min: -2., max: 2. }))
        ]
    );
    Ok(())
}

#[test]
fn reconnecting_mixed_author_and_prefab_joints_rebases_each_frame_and_keeps_source_identity()
-> Result<()> {
    for (kind, field, anchor, rotation) in [
        (
            JointKind::Point,
            schema::POINT_CONNECTION,
            schema::POINT_CONNECTED_ANCHOR,
            None,
        ),
        (
            JointKind::Hinge,
            schema::HINGE_CONNECTION,
            schema::HINGE_CONNECTED_ANCHOR,
            Some(schema::HINGE_CONNECTED_ROTATION),
        ),
        (
            JointKind::Slider,
            schema::SLIDER_CONNECTION,
            schema::SLIDER_CONNECTED_ANCHOR,
            Some(schema::SLIDER_CONNECTED_ROTATION),
        ),
    ] {
        let mut f = fixture(kind)?;
        let mut source = f.editor.document.source();
        let prefab_owner = source.base.entities.remove(
            source
                .base
                .entities
                .iter()
                .position(|e| e.id == f.ids[1])
                .unwrap(),
        );
        let raw = prefab_owner.id;
        let mut target = EntityData::new("共同连接刚体");
        target.physics = Some(Default::default());
        target.transform.translation = [3., 4., 5.];
        target.transform.scale = [2., 3., 4.];
        target.transform.rotation = Quat::from_rotation_z(0.4).to_array();
        let target_id = target.id;
        let file = f.editor.project_root().join("assets/common-joints.prefab");
        let bytes = ContentSource {
            base: SceneSnapshot {
                entities: vec![prefab_owner, target.clone()],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?;
        std::fs::write(&file, &bytes)?;
        prepare(f.editor.project_root(), &f.editor.products)?;
        f.assets.refresh()?;
        let metadata = std::fs::read(file.with_extension("prefab.meta"))?;
        let instance = InstanceId::new();
        source.instances.push(PrefabInstance {
            id: instance,
            prefab: SourceMeta::load(&file)?.id,
            translation: [1., 2., 3.],
            parent: None,
            overrides: vec![],
        });
        f.editor.document = Document::open_source(source)?;
        f.ids[1] = raw.in_instance(instance);
        f.editor.selection.replace(f.ids);
        let before = f.editor.document.shared_source();
        if kind != JointKind::Point {
            f.select_reference(
                "驱动模式",
                if kind == JointKind::Hinge {
                    "位置驱动"
                } else {
                    "速度驱动"
                },
            )?;
            assert_eq!(f.editor.document.shared_source(), before);
            assert_eq!(f.editor.document.history_len(), 0);
        }
        let expanded = f.editor.snapshot(&mut f.assets)?;
        let target_world = target_id.in_instance(instance);
        // The displayed target is valid in both the author's and the direct Prefab's scope.
        f.select_reference("连接刚体", "共同连接刚体")?;
        assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
        let changed = f.editor.document.shared_source();
        assert_eq!(f.editor.document.history_len(), 1);
        let authored = changed
            .base
            .entities
            .iter()
            .find(|e| e.id == f.ids[0])
            .unwrap();
        assert_eq!(
            schema::descriptor(field)?.read(authored),
            Some(FieldValue::JointConnection(JointConnection::Body(
                EntityRef::InInstance {
                    instance,
                    entity: target_id
                }
            )))
        );
        assert!(
            changed.instances[0]
                .overrides
                .iter()
                .any(|edit| edit.entity == raw
                    && edit.field == field
                    && edit.value
                        == FieldValue::JointConnection(JointConnection::Body(EntityRef::local(
                            target_id
                        ))))
        );
        let snapshot = f.editor.snapshot(&mut f.assets)?;
        let destination = snapshot
            .entities
            .iter()
            .find(|e| e.id == target_world)
            .unwrap();
        for id in f.ids {
            let owner = expanded.entities.iter().find(|e| e.id == id).unwrap();
            let local = match kind {
                JointKind::Point => owner.point_joint.as_ref().unwrap().anchor,
                JointKind::Hinge => owner.hinge_joint.as_ref().unwrap().frame.anchor,
                JointKind::Slider => owner.slider_joint.as_ref().unwrap().frame.anchor,
            };
            let expected = Vec3::from_array(owner.transform.translation)
                + Quat::from_array(owner.transform.rotation)
                    * (Vec3::from_array(owner.transform.scale) * Vec3::from_array(local));
            let actual = snapshot.entities.iter().find(|e| e.id == id).unwrap();
            let FieldValue::Vec3(connected) = schema::descriptor(anchor)?.read(actual).unwrap()
            else {
                unreachable!()
            };
            let q = Quat::from_array(destination.transform.rotation);
            let actual_point = Vec3::from_array(destination.transform.translation)
                + q * (Vec3::from_array(destination.transform.scale) * Vec3::from_array(connected));
            assert!((actual_point - expected).length() < 1e-5);
            if let Some(rotation) = rotation {
                let local = match kind {
                    JointKind::Hinge => owner.hinge_joint.as_ref().unwrap().frame.rotation,
                    JointKind::Slider => owner.slider_joint.as_ref().unwrap().frame.rotation,
                    _ => unreachable!(),
                };
                let FieldValue::Quat(connected) =
                    schema::descriptor(rotation)?.read(actual).unwrap()
                else {
                    unreachable!()
                };
                assert!(
                    (q * Quat::from_array(connected))
                        .dot(Quat::from_array(owner.transform.rotation) * Quat::from_array(local))
                        .abs()
                        > 0.99999
                );
            }
        }
        f.editor.replace_joint_connections(
            &f.ids,
            field,
            JointConnection::Body(EntityRef::local(target_world)),
            &mut f.assets,
        )?;
        assert!(!f.editor.document.is_editing());
        assert_eq!(f.editor.document.shared_source(), changed);
        f.editor.save(&mut f.assets)?;
        assert_eq!(Document::load(&f.editor.source)?.shared_source(), changed);
        f.editor.document.undo()?;
        assert_eq!(f.editor.document.shared_source(), before);
        f.editor.document.redo()?;
        assert_eq!(f.editor.document.shared_source(), changed);
        assert_eq!(std::fs::read(&file)?, bytes);
        assert_eq!(std::fs::read(file.with_extension("prefab.meta"))?, metadata);
    }
    Ok(())
}

#[test]
fn joint_connection_failures_preserve_the_complete_selection_and_an_existing_preview() -> Result<()>
{
    let mut f = fixture(JointKind::Point)?;
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .physics
        .as_mut()
        .unwrap()
        .motion = BodyMotion::Fixed;
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .point_joint
        .as_mut()
        .unwrap()
        .settings
        .connection = JointConnection::Unbound;
    f.editor.document = Document::open_source(source)?;
    f.editor.edit(
        f.ids[0],
        schema::POINT_ANCHOR,
        FieldValue::Vec3([0.8, 0.2, 0.3]),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    for ids in [
        f.ids.to_vec(),
        vec![f.ids[0], f.ids[0]],
        vec![f.ids[0], EntityId::new()],
    ] {
        assert!(
            f.editor
                .replace_joint_connections(
                    &ids,
                    schema::POINT_CONNECTION,
                    JointConnection::World,
                    &mut f.assets
                )
                .is_err()
        );
        assert!(f.editor.document.is_editing());
        assert_eq!(f.editor.document.shared_source(), preview);
    }
    assert!(
        f.editor
            .replace_joint_connections(
                &f.ids,
                schema::POINT_CONNECTION,
                JointConnection::Body(EntityRef::local(f.ids[0])),
                &mut f.assets
            )
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), preview);
    f.editor.cancel()?;
    let before = f.editor.document.shared_source();
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    assert!(
        f.editor
            .replace_joint_connections(
                &f.ids,
                schema::POINT_CONNECTION,
                JointConnection::Unbound,
                &mut f.assets
            )
            .is_err()
    );
    f.click_flag("启用关节")?;
    assert_eq!(f.editor.document.shared_source(), before);
    f.editor.stop();
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_common_joint_fields_preserve_authoring() -> Result<()> {
    let mut f = fixture(JointKind::Point)?;
    let mut source = f.editor.document.source();
    for entity in &mut source.base.entities {
        entity.model = None;
    }
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .point_joint
        .as_mut()
        .unwrap()
        .settings
        .connection = JointConnection::Unbound;
    f.editor.document = Document::open_source(source)?;
    let output = f.frame(vec![]);
    let fields = ["点关节", "启用关节", "连接刚体（多值）", "本地锚点（米）"];
    for label in fields {
        assert!(
            texts(&output).iter().any(|(text, _)| text == label),
            "missing {label}"
        );
    }
    capture_common(
        f,
        "editor-common-joints",
        serde_json::json!({"joint_fields":fields,"source_unchanged":true}),
    )
}
