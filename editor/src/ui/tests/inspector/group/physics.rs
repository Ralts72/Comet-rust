use super::*;
use crate::fields::ColliderKind;
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::InstanceId;
use comet_scene::{
    BodyMotion, ColliderShape, ContentSource, FieldUpdate, PointJoint, PrefabInstance,
    schema::{self, FieldValue},
};

#[test]
fn shape_dimensions_and_kind_changes_preserve_each_body_and_form_atomic_history() -> Result<()> {
    let mut f = Group::new()?;
    f.physics(false)?;
    let mut source = f.editor.document.source();
    for (index, &id) in f.ids.iter().enumerate() {
        source
            .base
            .entities
            .iter_mut()
            .find(|e| e.id == id)
            .unwrap()
            .physics
            .as_mut()
            .unwrap()
            .shape = ColliderShape::Box {
            half_extents: [
                0.2 + index as f32 * 0.1,
                0.4 + index as f32 * 0.2,
                0.6 + index as f32 * 0.3,
            ],
        };
    }
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    let before = f.bodies()?;
    f.type_scalar("盒体半尺寸 X", "1.25")?;
    f.confirm();
    let after = f.bodies()?;
    for (mut expected, actual) in before.clone().into_iter().zip(after) {
        let ColliderShape::Box { half_extents } = &mut expected.shape else {
            unreachable!()
        };
        half_extents[0] = 1.25;
        assert_eq!(actual, expected);
    }
    assert_eq!(f.editor.document.history_len(), 1);
    let edited = f.editor.document.shared_source();
    let end = f.drag_scalar("盒体半尺寸 Y", 20.)?;
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![
        button(end, false),
        key(egui::Key::Escape, false, Default::default()),
    ]);
    assert_eq!(f.editor.document.shared_source(), edited);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    f.editor.edit(
        f.ids[1],
        schema::COLLIDER_SHAPE,
        FieldValue::ColliderShape(ColliderShape::Sphere { radius: 1.3 }),
        &mut f.assets,
    )?;
    f.editor.commit()?;
    f.select_reference("形状", "球体")?;
    let bodies = f.bodies()?;
    assert_eq!(bodies[0].shape, ColliderShape::Sphere { radius: 0.5 });
    assert_eq!(bodies[1].shape, ColliderShape::Sphere { radius: 1.3 });
    for (expected, actual) in before.into_iter().zip(&bodies) {
        assert_eq!(expected.mass, actual.mass);
        assert_eq!(expected.friction, actual.friction);
        assert_eq!(expected.mask, actual.mask);
    }
    assert_eq!(f.editor.document.history_len(), 3);
    f.select_reference("形状", "球体")?;
    assert_eq!(f.editor.document.history_len(), 3);
    f.select_reference("运动类型", "动态")?;
    f.click_flag("触发器")?;
    assert!(
        f.bodies()?
            .iter()
            .all(|body| body.motion == BodyMotion::Dynamic && body.sensor)
    );
    assert_eq!(f.editor.document.history_len(), 5);
    Ok(())
}

#[test]
fn mixed_prefab_physics_edits_roundtrip_typed_overrides_and_restore_shape_inheritance() -> Result<()>
{
    let mut f = Group::new()?;
    f.physics(false)?;
    let original = f.editor.document.shared_source();
    let mut node = original
        .base
        .entities
        .iter()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .clone();
    node.name = "Prefab 刚体".into();
    node.physics.as_mut().unwrap().motion = BodyMotion::Kinematic;
    node.physics.as_mut().unwrap().shape = ColliderShape::Capsule {
        half_height: 0.8,
        radius: 0.7,
    };
    let source_node = node.id;
    let path = f.editor.project_root().join("assets/physics.prefab");
    std::fs::write(
        &path,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![node],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let metadata = std::fs::read(SourceMeta::path(&path))?;
    let prefab_bytes = std::fs::read(&path)?;
    let instance = InstanceId::new();
    let mut source = f.editor.document.source();
    source.base.entities.retain(|e| e.id == f.ids[0]);
    source.instances.push(PrefabInstance {
        id: instance,
        prefab: SourceMeta::load(&path)?.id,
        parent: None,
        translation: [0.; 3],
        overrides: vec![],
    });
    f.ids[1] = source_node.in_instance(instance);
    f.editor.document = Document::open_source(source)?;
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    let before = f.bodies()?;
    f.select_reference("形状", "胶囊")?;
    assert_eq!(f.bodies()?[1].shape, before[1].shape);
    f.type_scalar("胶囊半高", "0.9")?;
    f.confirm();
    f.type_scalar("球／胶囊半径", "0.45")?;
    f.confirm();
    f.select_reference("运动类型", "动态")?;
    f.click_text("碰撞过滤")?;
    f.type_scalar("检测碰撞层", "FEDCBA98")?;
    f.confirm();
    assert!(
        f.bodies()?.iter().all(|body| body.mask == 0xFEDC_BA98),
        "mask edit did not publish: {:?}",
        texts(&f.frame(vec![]))
    );
    f.type_scalar("弹性系数", "0.7")?;
    f.confirm();
    for (mut expected, actual) in before.into_iter().zip(f.bodies()?) {
        expected.shape = ColliderShape::Capsule {
            half_height: 0.9,
            radius: 0.45,
        };
        expected.motion = BodyMotion::Dynamic;
        expected.mask = 0xFEDC_BA98;
        expected.restitution = 0.7;
        assert_eq!(actual, expected);
    }
    assert_eq!(f.editor.document.history_len(), 6);
    let edited = f.editor.document.shared_source();
    let overrides = &edited.instances[0].overrides;
    assert_eq!(overrides.len(), 4);
    for field in [
        schema::COLLIDER_SHAPE,
        schema::BODY_MOTION,
        schema::COLLISION_MASK,
        schema::RESTITUTION,
    ] {
        assert!(
            overrides
                .iter()
                .any(|v| v.entity == source_node && v.field == field)
        );
    }
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    for _ in 0..6 {
        f.editor.document.undo()?;
    }
    assert_eq!(f.editor.document.shared_source(), original);
    for _ in 0..6 {
        f.editor.document.redo()?;
    }
    assert_eq!(f.editor.document.shared_source(), edited);
    assert!(!f.editor.document.is_dirty());
    f.editor
        .revert_prefab_fields(&f.ids[1..], Some(&[schema::COLLIDER_SHAPE]), &mut f.assets)?;
    assert_eq!(
        f.bodies()?[1].shape,
        ColliderShape::Capsule {
            half_height: 0.8,
            radius: 0.7
        }
    );
    assert_eq!(f.bodies()?[1].mask, 0xFEDC_BA98);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), edited);
    assert_eq!(std::fs::read(&path)?, prefab_bytes);
    assert_eq!(std::fs::read(SourceMeta::path(&path))?, metadata);
    Ok(())
}

#[test]
fn joint_and_character_constraints_reject_whole_batches_and_preserve_valid_previews() -> Result<()>
{
    let mut f = Group::new()?;
    f.physics(false)?;
    let mut source = f.editor.document.source();
    for entity in &mut source.base.entities {
        entity.physics.as_mut().unwrap().motion = BodyMotion::Dynamic;
    }
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .point_joint = Some(PointJoint::default());
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.editor.edit(
        f.ids[0],
        schema::MASS,
        FieldValue::Scalar(2.5),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    let updates: Vec<_> = f
        .ids
        .iter()
        .map(|id| FieldUpdate {
            entity: *id,
            field: schema::BODY_MOTION,
            value: FieldValue::BodyMotion(BodyMotion::Fixed),
        })
        .collect();
    assert!(f.editor.edit_fields(&updates, &mut f.assets).is_err());
    assert_eq!(f.editor.document.shared_source(), preview);
    assert!(f.editor.document.is_editing());
    f.editor.cancel()?;
    f.select_reference("运动类型", "静态")?;
    assert!(f.editor.error.is_some());
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);

    let mut f = Group::new()?;
    f.physics(true)?;
    f.editor.edit(
        f.ids[0],
        schema::CROUCH_SPEED,
        FieldValue::Scalar(1.5),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    for (field, value) in [
        (
            schema::BODY_MOTION,
            FieldValue::BodyMotion(BodyMotion::Fixed),
        ),
        (schema::BODY_SENSOR, FieldValue::Bool(true)),
        (
            schema::COLLIDER_SHAPE,
            FieldValue::ColliderShape(ColliderShape::Sphere { radius: 0.5 }),
        ),
        (schema::PLAYER_CONTROLLED, FieldValue::Bool(true)),
    ] {
        let updates: Vec<_> = f
            .ids
            .iter()
            .map(|id| FieldUpdate {
                entity: *id,
                field,
                value: value.clone(),
            })
            .collect();
        assert!(f.editor.edit_fields(&updates, &mut f.assets).is_err());
        assert_eq!(f.editor.document.shared_source(), preview);
        assert!(f.editor.document.is_editing());
    }
    f.editor.cancel()?;
    assert!(
        f.editor
            .change_collider_kind(&f.ids, ColliderKind::Sphere, &mut f.assets)
            .is_err()
    );
    assert!(!f.editor.document.is_editing());
    f.editor.start(&mut f.assets)?;
    assert!(
        f.editor
            .change_collider_kind(&f.ids, ColliderKind::Capsule, &mut f.assets)
            .is_err()
    );
    f.editor.stop();
    Ok(())
}

#[test]
fn hexadecimal_masks_keep_all_32_bits_and_rejected_text_never_unifies_fallback_values() -> Result<()>
{
    let mut f = Group::new()?;
    f.physics(false)?;
    let mut source = f.editor.document.source();
    for (&id, mask) in f.ids.iter().zip([0x8000_0001, u32::MAX]) {
        source
            .base
            .entities
            .iter_mut()
            .find(|e| e.id == id)
            .unwrap()
            .physics
            .as_mut()
            .unwrap()
            .mask = mask;
    }
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.click_text("碰撞过滤")?;
    f.frame(vec![]);
    assert_eq!(f.editor.document.shared_source(), original);
    f.type_scalar("检测碰撞层", "80000001")?;
    f.confirm();
    assert!(f.bodies()?.iter().all(|body| body.mask == 0x8000_0001));
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.document.undo()?;
    for text in ["100000000", "GGGG"] {
        f.type_scalar("检测碰撞层", text)?;
        f.confirm();
        assert_eq!(f.editor.document.shared_source(), original);
        assert_eq!(f.editor.document.history_len(), 0);
    }
    f.type_scalar("所属碰撞层", "FFFFFFFF")?;
    f.confirm();
    assert!(f.bodies()?.iter().all(|body| body.layer == u32::MAX));
    let accepted = f.editor.document.shared_source();
    let end = f.drag_scalar("检测碰撞层", -20.)?;
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![
        button(end, false),
        key(egui::Key::Escape, false, Default::default()),
    ]);
    assert_eq!(f.editor.document.shared_source(), accepted);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), accepted);
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_common_physics_fields_preserve_authoring() -> Result<()> {
    let mut f = Group::new()?;
    f.physics(false)?;
    let mut source = f.editor.document.source();
    let body = source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .physics
        .as_mut()
        .unwrap();
    body.motion = BodyMotion::Dynamic;
    body.sensor = true;
    f.editor.document = Document::open_source(source)?;
    let output = f.frame(vec![]);
    let fields = ["运动类型（多值）", "形状", "盒体", "触发器（多值）"];
    for label in fields {
        assert!(texts(&output).iter().any(|(text, _)| text == label));
    }
    capture_common(
        f,
        "editor-common-physics",
        serde_json::json!({"physics_fields": fields, "source_unchanged":true}),
    )
}
