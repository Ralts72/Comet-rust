use super::*;
use anyhow::Result;
use comet_core::{EntityId, math::Vec3};
use comet_editor_core::Document;
use comet_scene::{BodyMotion, PhysicsBody, SceneSnapshot};
use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};
use std::sync::Arc;

fn fixture() -> Result<(tempfile::TempDir, Editor, AssetStore, EntityId, EntityId)> {
    let (temp, mut editor, assets) = crate::test_support::fixture()?;
    let mut owner = EntityData::new("关节对象");
    owner.transform.translation = [2., 3., 4.];
    owner.transform.rotation = Quat::from_rotation_y(0.3).to_array();
    owner.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    let mut target = EntityData::new("连接对象");
    target.transform.translation = [5., 1., -2.];
    target.transform.scale = [2., 6., 3.];
    target.transform.rotation = Quat::from_rotation_z(0.4).to_array();
    target.physics = Some(PhysicsBody::default());
    let ids = (owner.id, target.id);
    editor.document = Document::open(SceneSnapshot {
        entities: vec![owner, target],
        ..Default::default()
    })?;
    Ok((temp, editor, assets, ids.0, ids.1))
}
fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    id: EntityId,
    events: Vec<Event>,
) -> egui::FullOutput {
    editor.last_snapshot = Arc::new(editor.snapshot(assets).unwrap());
    let entity = editor
        .last_snapshot
        .entities
        .iter()
        .find(|entity| entity.id == id)
        .unwrap()
        .clone();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(1100., 3000.),
            )),
            events,
            ..Default::default()
        },
        |ui| draw(editor, ui, assets, &entity),
    );
    output.textures_delta.clear();
    output
}
fn label(output: &egui::FullOutput, text: &str) -> Pos2 {
    fn find(shape: &egui::epaint::Shape, text: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(value) if value.galley.job.text.starts_with(text) => {
                Some(value.pos + value.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, text)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, text))
        .unwrap_or_else(|| panic!("missing label {text}"))
}
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}
fn click(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    id: EntityId,
    point: Pos2,
) -> egui::FullOutput {
    frame(
        ctx,
        editor,
        assets,
        id,
        vec![Event::PointerMoved(point), button(point, true)],
    );
    frame(ctx, editor, assets, id, vec![button(point, false)])
}
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    ctx
}
fn point(editor: &Editor, id: EntityId) -> PointJoint {
    editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .point_joint
        .clone()
        .unwrap()
}

#[test]
fn authored_joint_picker_connects_to_one_direct_instance_with_atomic_anchor_and_history()
-> Result<()> {
    let (_temp, mut editor, mut assets, owner, target) = fixture()?;
    let mut source = editor.document.source();
    let index = source
        .base
        .entities
        .iter()
        .position(|entity| entity.id == target)
        .unwrap();
    let body = source.base.entities.remove(index);
    let prefab = comet_scene::ContentSource {
        base: SceneSnapshot {
            entities: vec![body.clone()],
            ..Default::default()
        },
        ..Default::default()
    };
    let file = editor.project_root().join("assets/connected.prefab");
    std::fs::write(&file, prefab.to_json()?)?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    let asset = comet_asset_pipeline::SourceMeta::load(&file)?.id;
    let first = comet_core::InstanceId::new();
    let second = comet_core::InstanceId::new();
    source.instances = [first, second]
        .into_iter()
        .map(|id| comet_scene::PrefabInstance {
            id,
            prefab: asset,
            parent: None,
            overrides: vec![],
        })
        .collect();
    editor.document = Document::open_source(source)?;
    let ctx = context();
    frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "添加点约束"),
    );
    let before = editor.document.shared_source();
    let references = editor.cached_field_references(&ctx, owner, &mut assets)?;
    let target_world = target.in_instance(first);
    let name = references
        .targets
        .iter()
        .find(|(id, _)| *id == target_world)
        .unwrap()
        .1
        .clone();
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "世界"),
    );
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    click(&ctx, &mut editor, &mut assets, owner, label(&output, &name));
    assert!(editor.error.is_none(), "{:?}", editor.error);
    let changed = editor.document.shared_source();
    let joint = point(&editor, owner);
    assert_eq!(
        joint.settings.connection,
        JointConnection::Body(EntityRef::InInstance {
            instance: first,
            entity: target,
        })
    );
    let connected_world = Vec3::from_array(body.transform.translation)
        + Quat::from_array(body.transform.rotation)
            * (Vec3::from_array(body.transform.scale) * Vec3::from_array(joint.connected_anchor));
    assert!((connected_world - Vec3::new(2., 3., 4.)).length() < 1e-5);
    assert_eq!(editor.document.history_len(), 2);
    assert_eq!(changed.base.version, 10);
    let snapshot = editor.snapshot(&mut assets)?;
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|entity| entity.id == owner)
            .unwrap()
            .point_joint
            .as_ref()
            .unwrap()
            .settings
            .connection,
        JointConnection::Body(EntityRef::local(target_world))
    );
    editor.save(&mut assets)?;
    assert_eq!(Document::load(&editor.source)?.shared_source(), changed);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), changed);
    editor.start(&mut assets)?;
    assert_eq!(
        editor
            .play
            .as_ref()
            .unwrap()
            .joint_status(owner, JointKind::Point),
        Some(JointStatus::Active)
    );
    editor.stop();
    assert_eq!(editor.document.shared_source(), changed);
    Ok(())
}

#[test]
fn adding_dragging_and_removing_a_joint_use_real_widgets_and_one_undo_step_per_gesture()
-> Result<()> {
    let (_temp, mut editor, mut assets, owner, _) = fixture()?;
    let ctx = context();
    frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "添加点约束"),
    );
    assert_eq!(editor.document.history_len(), 1);
    assert_eq!(point(&editor, owner).connected_anchor, [2., 3., 4.]);
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    let start = label(&output, "X ");
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        vec![Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        vec![Event::PointerMoved(start + Vec2::new(30., 0.))],
    );
    assert!(editor.document.is_editing());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        vec![Event::PointerMoved(start + Vec2::new(55., 0.))],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        vec![button(start + Vec2::new(55., 0.), false)],
    );
    assert!(!editor.document.is_editing());
    assert_eq!(editor.document.history_len(), 2);
    assert!(point(&editor, owner).anchor[0] > 0.);
    let accepted = editor.document.shared_source();
    editor.document.undo()?;
    assert_eq!(point(&editor, owner).anchor[0], 0.);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), accepted);
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    let start = label(&output, "X ");
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        vec![Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        vec![Event::PointerMoved(start + Vec2::new(25., 0.))],
    );
    assert!(editor.document.is_editing());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(!editor.document.is_editing());
    assert_eq!(editor.document.shared_source(), accepted);
    let output = frame(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        vec![button(start + Vec2::new(25., 0.), false)],
    );
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "移除点约束"),
    );
    assert!(
        editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .find(|e| e.id == owner)
            .unwrap()
            .point_joint
            .is_none()
    );
    assert_eq!(editor.document.history_len(), 3);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), accepted);
    Ok(())
}

#[test]
fn prefab_joint_widgets_only_write_existing_field_overrides() -> Result<()> {
    let (_temp, mut editor, mut assets, _, _) = fixture()?;
    let mut entity = EntityData::new("预制体关节");
    entity.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    entity.point_joint = Some(PointJoint::default());
    let mut connected = EntityData::new("继承连接对象");
    connected.physics = Some(PhysicsBody::default());
    let connected_id = connected.id;
    entity.point_joint.as_mut().unwrap().settings.connection =
        JointConnection::Body(EntityRef::local(connected_id));
    let source_entity = entity.id;
    let source = comet_scene::ContentSource {
        base: SceneSnapshot {
            entities: vec![entity, connected],
            ..Default::default()
        },
        ..Default::default()
    };
    let file = editor.project_root().join("assets/joints.prefab");
    let bytes = source.to_json()?;
    std::fs::write(&file, &bytes)?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    let asset = comet_asset_pipeline::SourceMeta::load(&file)?.id;
    let instance = comet_core::InstanceId::new();
    editor.document = Document::open_source(comet_scene::ContentSource {
        instances: vec![comet_scene::PrefabInstance {
            id: instance,
            prefab: asset,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    let id = source_entity.in_instance(instance);
    let original = editor.document.shared_source();
    let ctx = context();
    frame(&ctx, &mut editor, &mut assets, id, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, id, vec![]);
    click(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        label(&output, "启用关节"),
    );
    assert_eq!(editor.document.history_len(), 1);
    let source = editor.document.shared_source();
    assert!(source.base.entities.is_empty());
    let overrides = &source.instances[0].overrides;
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].entity, source_entity);
    assert_eq!(overrides[0].field, schema::POINT_ENABLED);
    assert_eq!(overrides[0].value, schema::FieldValue::Bool(false));
    let expanded = source.expand(&mut assets)?;
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .point_joint
            .as_ref()
            .unwrap()
            .settings
            .connection,
        JointConnection::Body(EntityRef::local(connected_id.in_instance(instance)))
    );
    assert_eq!(std::fs::read(&file)?, bytes);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), original);

    let output = frame(&ctx, &mut editor, &mut assets, id, vec![]);
    let start = label(&output, "X ");
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        vec![Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        vec![Event::PointerMoved(start + Vec2::new(40., 0.))],
    );
    assert!(editor.document.is_editing());
    assert_ne!(editor.document.shared_source(), original);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        vec![Event::PointerMoved(start)],
    );
    assert_eq!(editor.document.shared_source(), original);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        vec![button(start, false)],
    );
    assert!(!editor.document.is_editing());
    assert!(!editor.document.is_dirty());
    assert_eq!(editor.document.history_len(), 0);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), source);
    assert_eq!(std::fs::read(&file)?, bytes);
    Ok(())
}

#[test]
fn prefab_connection_selection_stores_source_identity_and_remaps_on_save_and_expansion()
-> Result<()> {
    let (_temp, mut editor, mut assets, _, _) = fixture()?;
    let mut owner = EntityData::new("Prefab 连接起点");
    owner.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    owner.point_joint = Some(PointJoint::default());
    let raw_owner = owner.id;
    let mut target = EntityData::new("Prefab 连接终点");
    target.physics = Some(PhysicsBody::default());
    let raw_target = target.id;
    let prefab = comet_scene::ContentSource {
        base: SceneSnapshot {
            entities: vec![owner, target],
            ..Default::default()
        },
        ..Default::default()
    };
    let file = editor.project_root().join("assets/joint-connection.prefab");
    let bytes = prefab.to_json()?;
    std::fs::write(&file, &bytes)?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    let asset = comet_asset_pipeline::SourceMeta::load(&file)?.id;
    let instance = comet_core::InstanceId::new();
    let mut outside = EntityData::new("作者对象不应成为 Prefab 局部连接目标");
    outside.physics = Some(PhysicsBody::default());
    editor.document = Document::open_source(comet_scene::ContentSource {
        base: SceneSnapshot {
            entities: vec![outside],
            ..Default::default()
        },
        instances: vec![comet_scene::PrefabInstance {
            id: instance,
            prefab: asset,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    let before = editor.document.shared_source();
    let id = raw_owner.in_instance(instance);
    let ctx = context();
    frame(&ctx, &mut editor, &mut assets, id, vec![]);
    let owner = editor
        .last_snapshot
        .entities
        .iter()
        .find(|entity| entity.id == id)
        .unwrap();
    assert_eq!(
        editor
            .joint_targets(owner, &egui::Context::default(), &mut assets)?
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![raw_target.in_instance(instance)]
    );
    let output = frame(&ctx, &mut editor, &mut assets, id, vec![]);
    click(&ctx, &mut editor, &mut assets, id, label(&output, "世界"));
    let output = frame(&ctx, &mut editor, &mut assets, id, vec![]);
    click(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        label(&output, "Prefab 连接终点"),
    );
    assert!(!editor.document.is_editing());
    let accepted = editor.document.shared_source();
    let connection = accepted.instances[0]
        .overrides
        .iter()
        .find(|value| value.field == schema::POINT_CONNECTION)
        .unwrap();
    assert_eq!(
        connection.value,
        schema::FieldValue::JointConnection(JointConnection::Body(EntityRef::local(raw_target)))
    );
    let saved = editor.project_root().join("assets/connection.scene");
    editor.document.save(&saved)?;
    let reopened = Document::load(&saved)?.source().expand(&mut assets)?;
    assert_eq!(
        reopened
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .point_joint
            .as_ref()
            .unwrap()
            .settings
            .connection,
        JointConnection::Body(EntityRef::local(raw_target.in_instance(instance)))
    );
    assert_eq!(std::fs::read(file)?, bytes);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn non_dynamic_owners_add_unbound_joints_and_only_offer_dynamic_body_targets() -> Result<()> {
    let (_temp, mut editor, mut assets, owner, target) = fixture()?;
    let mut source = editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == owner)
        .unwrap()
        .physics
        .as_mut()
        .unwrap()
        .motion = BodyMotion::Fixed;
    editor.document = Document::open_source(source)?;
    let entity = editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .find(|entity| entity.id == owner)
        .unwrap()
        .clone();
    for kind in [JointKind::Point, JointKind::Hinge, JointKind::Slider] {
        let connection = match initial_joint(kind, &entity) {
            JointEdit::Point(Some(joint)) => joint.settings.connection,
            JointEdit::Hinge(Some(joint)) => joint.settings.connection,
            JointEdit::Slider(Some(joint)) => joint.settings.connection,
            _ => unreachable!(),
        };
        assert_eq!(connection, JointConnection::Unbound);
    }
    let ctx = context();
    frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    assert!(
        editor
            .joint_targets(&entity, &egui::Context::default(), &mut assets)?
            .is_empty()
    );
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "添加点约束"),
    );
    assert_eq!(
        point(&editor, owner).settings.connection,
        JointConnection::Unbound
    );
    assert_eq!(
        editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .find(|entity| entity.id == owner)
            .unwrap()
            .physics
            .as_ref()
            .unwrap()
            .motion,
        BodyMotion::Fixed
    );
    let mut source = editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == target)
        .unwrap()
        .physics
        .as_mut()
        .unwrap()
        .motion = BodyMotion::Dynamic;
    editor.document = Document::open_source(source)?;
    frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    assert_eq!(
        editor
            .joint_targets(&entity, &egui::Context::default(), &mut assets)?
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![target]
    );
    Ok(())
}

#[test]
fn changing_connection_rebases_scaled_rotated_target_anchors_and_passive_draw_preserves_rotation()
-> Result<()> {
    let (_temp, mut editor, mut assets, owner, target) = fixture()?;
    editor.edit_joint(
        owner,
        initial_joint(
            JointKind::Hinge,
            &editor
                .document
                .shared_source()
                .base
                .entities
                .iter()
                .find(|e| e.id == owner)
                .unwrap()
                .clone(),
        ),
        &mut assets,
    )?;
    editor.commit()?;
    let ctx = context();
    frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    let before = editor.document.shared_source();
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    assert!(Arc::ptr_eq(&before, &editor.document.shared_source()));
    assert!(!editor.document.is_editing());
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "世界"),
    );
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "连接对象"),
    );
    let source = editor.document.shared_source();
    let joint = source
        .base
        .entities
        .iter()
        .find(|e| e.id == owner)
        .unwrap()
        .hinge_joint
        .as_ref()
        .unwrap();
    assert_eq!(
        joint.settings.connection,
        JointConnection::Body(EntityRef::local(target))
    );
    let target = source
        .base
        .entities
        .iter()
        .find(|e| e.id == target)
        .unwrap();
    let q = Quat::from_array(target.transform.rotation);
    let anchor = comet_core::math::Vec3::from_array(target.transform.translation)
        + q * (comet_core::math::Vec3::from_array(target.transform.scale)
            * comet_core::math::Vec3::from_array(joint.connected_frame.anchor));
    assert!((anchor - comet_core::math::Vec3::new(2., 3., 4.)).length() < 1e-5);
    let rotation = q * Quat::from_array(joint.connected_frame.rotation);
    assert!(rotation.dot(Quat::from_rotation_y(0.3)).abs() > 0.99999);
    assert_eq!(editor.document.history_len(), 2);
    assert!(!editor.document.is_editing());
    Ok(())
}

#[test]
fn play_keeps_joint_widgets_read_only_and_editor_commands_reject_changes() -> Result<()> {
    let (_temp, mut editor, mut assets, owner, _) = fixture()?;
    editor.edit_joint(
        owner,
        JointEdit::Point(Some(PointJoint::default())),
        &mut assets,
    )?;
    editor.commit()?;
    editor.start(&mut assets)?;
    let before = editor.document.shared_source();
    let history = editor.document.history_len();
    let ctx = context();
    frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    label(&output, "运行状态：已连接");
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "启用关节"),
    );
    click(
        &ctx,
        &mut editor,
        &mut assets,
        owner,
        label(&output, "移除点约束"),
    );
    assert!(
        editor
            .edit_joint(owner, JointEdit::Point(None), &mut assets)
            .is_err()
    );
    assert_eq!(editor.document.history_len(), history);
    assert!(Arc::ptr_eq(&before, &editor.document.shared_source()));
    assert!(editor.play.is_some());
    Ok(())
}

#[test]
fn passive_hinge_controls_preserve_quaternion_sign_and_legal_offsets_beyond_pi() -> Result<()> {
    let (_temp, mut editor, mut assets, owner, _) = fixture()?;
    let rotation = -Quat::from_euler(EulerRot::XYZ, 0.2, -0.4, 0.7);
    let joint = HingeJoint {
        frame: JointFrame {
            rotation: rotation.to_array(),
            ..Default::default()
        },
        limits: Some(JointLimits { min: 2., max: 4. }),
        motor: Some(JointMotor::Position {
            target: 3e38,
            frequency_hz: 2.,
            damping_ratio: 1.,
            max_effort: 2.,
        }),
        ..Default::default()
    };
    editor.edit_joint(owner, JointEdit::Hinge(Some(joint)), &mut assets)?;
    editor.commit()?;
    let before = editor.document.shared_source();
    let ctx = context();
    frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    frame(&ctx, &mut editor, &mut assets, owner, vec![]);
    assert!(Arc::ptr_eq(&before, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), 1);
    assert!(!editor.document.is_editing());
    Ok(())
}
