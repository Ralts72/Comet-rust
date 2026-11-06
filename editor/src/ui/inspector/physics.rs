use crate::session::Editor;
use comet_asset::AssetStore;
use comet_core::EntityId;
use comet_scene::{BodyMotion, Character, ColliderShape, EntityData, PhysicsBody};

#[derive(Default)]
struct Edit {
    finish: bool,
}
impl Edit {
    fn number(&mut self, ui: &mut egui::Ui, label: &str, value: &mut f32, min: f32, max: f32) {
        let response = ui.add(
            egui::DragValue::new(value)
                .prefix(format!("{label} "))
                .range(min..=max)
                .speed(0.05)
                .max_decimals(3),
        );
        self.finish |= response.drag_stopped() || response.lost_focus();
    }
    fn checkbox(&mut self, ui: &mut egui::Ui, value: &mut bool, label: &str) {
        self.finish |= ui.checkbox(value, label).changed();
    }
}

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.separator();
    ui.strong("物理");
    let authored = editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .any(|e| e.id == entity.id);
    if !authored {
        prefab_fields(editor, ui, assets, entity);
        return;
    }
    if entity.parent.is_some() {
        ui.weak("物理组件仅支持场景根实体。");
        return;
    }
    if editor.play.is_some() {
        ui.weak("停止运行后可修改物理组件。");
    }
    ui.push_id(("physics", entity.id.to_string()), |ui| {
        ui.add_enabled_ui(editor.play.is_none(), |ui| {
            let Some(mut body) = entity.physics.clone() else {
                if ui.button("添加刚体").clicked() {
                    replace(
                        editor,
                        assets,
                        entity.id,
                        Some(PhysicsBody::default()),
                        None,
                    );
                }
                if ui.button("添加角色控制器").clicked() {
                    replace(
                        editor,
                        assets,
                        entity.id,
                        Some(PhysicsBody {
                            motion: BodyMotion::Kinematic,
                            shape: ColliderShape::Capsule {
                                half_height: 0.5,
                                radius: 0.3,
                            },
                            ..Default::default()
                        }),
                        Some(new_character(editor)),
                    );
                }
                return;
            };
            let mut character = entity.character.clone();
            if ui
                .button(if character.is_some() {
                    "移除刚体及角色控制器"
                } else {
                    "移除刚体"
                })
                .clicked()
            {
                replace(editor, assets, entity.id, None, None);
                return;
            }
            let mut edit = Edit::default();
            ui.add_enabled_ui(character.is_none(), |ui| {
                egui::ComboBox::from_id_salt("motion")
                    .selected_text(motion_name(body.motion))
                    .show_ui(ui, |ui| {
                        for motion in [
                            BodyMotion::Fixed,
                            BodyMotion::Dynamic,
                            BodyMotion::Kinematic,
                        ] {
                            edit.finish |= ui
                                .selectable_value(&mut body.motion, motion, motion_name(motion))
                                .changed();
                        }
                    });
                egui::ComboBox::from_id_salt("shape")
                    .selected_text(shape_name(&body.shape))
                    .show_ui(ui, |ui| {
                        for shape in [
                            ColliderShape::Box {
                                half_extents: [0.5; 3],
                            },
                            ColliderShape::Sphere { radius: 0.5 },
                            ColliderShape::Capsule {
                                half_height: 0.5,
                                radius: 0.3,
                            },
                        ] {
                            let selected = std::mem::discriminant(&body.shape)
                                == std::mem::discriminant(&shape);
                            if ui.selectable_label(selected, shape_name(&shape)).clicked()
                                && !selected
                            {
                                body.shape = shape;
                                edit.finish = true;
                            }
                        }
                    });
                edit.checkbox(ui, &mut body.sensor, "触发器（不阻挡物体）");
            });
            if character.is_some() {
                ui.weak("角色使用非触发器的运动学胶囊刚体。");
            }
            match &mut body.shape {
                ColliderShape::Box { half_extents } => {
                    for (axis, value) in ["半尺寸 X", "半尺寸 Y", "半尺寸 Z"]
                        .into_iter()
                        .zip(half_extents)
                    {
                        edit.number(ui, axis, value, 0.001, f32::MAX);
                    }
                }
                ColliderShape::Sphere { radius } => {
                    edit.number(ui, "半径", radius, 0.001, f32::MAX)
                }
                ColliderShape::Capsule {
                    half_height,
                    radius,
                } => {
                    edit.number(ui, "胶囊半高（不含端部，米）", half_height, 0., f32::MAX);
                    edit.number(ui, "半径", radius, 0.001, f32::MAX);
                }
            }
            edit.number(ui, "质量 (kg)", &mut body.mass, 0.001, f32::MAX);
            edit.number(ui, "摩擦系数", &mut body.friction, 0., f32::MAX);
            edit.number(ui, "弹性系数", &mut body.restitution, 0., 1.);
            ui.collapsing("碰撞过滤", |ui| {
                ui.weak("十六进制位掩码；双方过滤规则同时满足才发生碰撞。");
                for (label, value) in [("所属层", &mut body.layer), ("碰撞掩码", &mut body.mask)]
                {
                    ui.label(label);
                    let response = ui.add(egui::DragValue::new(value).hexadecimal(8, false, true));
                    edit.finish |= response.drag_stopped() || response.lost_focus();
                }
            });
            ui.separator();
            if let Some(controller) = &mut character {
                ui.strong("角色控制器");
                if ui.button("移除角色控制器").clicked() {
                    character = None;
                    edit.finish = true;
                } else {
                    edit.checkbox(ui, &mut controller.player_controlled, "玩家控制");
                    edit.number(
                        ui,
                        "移动速度 (m/s)",
                        &mut controller.move_speed,
                        0.001,
                        f32::MAX,
                    );
                    edit.number(
                        ui,
                        "蹲伏速度 (m/s)",
                        &mut controller.crouch_speed,
                        0.001,
                        f32::MAX,
                    );
                    edit.number(
                        ui,
                        "蹲伏高度比例",
                        &mut controller.crouch_height_ratio,
                        0.001,
                        1.,
                    );
                    edit.number(
                        ui,
                        "跑步倍率",
                        &mut controller.run_multiplier,
                        0.001,
                        f32::MAX,
                    );
                    edit.number(
                        ui,
                        "跳跃速度 (m/s)",
                        &mut controller.jump_speed,
                        0.001,
                        f32::MAX,
                    );
                    edit.number(ui, "重力 (m/s²)", &mut controller.gravity, 0.001, f32::MAX);
                    edit.number(
                        ui,
                        "最大坡度 (°)",
                        &mut controller.max_slope_degrees,
                        0.,
                        89.9,
                    );
                    edit.number(
                        ui,
                        "台阶高度 (m)",
                        &mut controller.step_height,
                        0.,
                        f32::MAX,
                    );
                    edit.number(
                        ui,
                        "地面吸附距离 (m)",
                        &mut controller.snap_distance,
                        0.,
                        f32::MAX,
                    );
                }
            } else {
                let compatible = body.motion == BodyMotion::Kinematic
                    && !body.sensor
                    && matches!(body.shape, ColliderShape::Capsule { .. });
                if ui
                    .add_enabled(compatible, egui::Button::new("添加角色控制器"))
                    .on_hover_text("需要非触发器的运动学胶囊刚体、统一正缩放及直立朝向。")
                    .clicked()
                {
                    character = Some(new_character(editor));
                    edit.finish = true;
                }
            }
            if entity.physics.as_ref() != Some(&body) || entity.character != character {
                let result = editor.edit_physics(entity.id, Some(body), character, assets);
                editor.notice(result);
            }
            if edit.finish && ui.is_enabled() {
                let result = editor.commit();
                editor.notice(result);
            }
        });
    });
}

fn replace(
    editor: &mut Editor,
    assets: &mut AssetStore,
    id: EntityId,
    body: Option<PhysicsBody>,
    character: Option<Character>,
) {
    let result = editor
        .commit()
        .and_then(|_| editor.edit_physics(id, body, character, assets))
        .and_then(|_| editor.commit());
    editor.notice(result);
}

fn new_character(editor: &Editor) -> Character {
    Character {
        player_controlled: !editor
            .last_snapshot
            .entities
            .iter()
            .any(|e| e.character.as_ref().is_some_and(|c| c.player_controlled)),
        ..Default::default()
    }
}

fn motion_name(motion: BodyMotion) -> &'static str {
    match motion {
        BodyMotion::Fixed => "静态",
        BodyMotion::Dynamic => "动态",
        BodyMotion::Kinematic => "运动学",
    }
}
fn shape_name(shape: &ColliderShape) -> &'static str {
    match shape {
        ColliderShape::Box { .. } => "盒体",
        ColliderShape::Sphere { .. } => "球体",
        ColliderShape::Capsule { .. } => "胶囊",
    }
}

fn prefab_fields(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.weak("Prefab 实例的组件结构请在源 Prefab 中编辑。");
    let target = if editor.play.is_none() {
        Some(super::overrides::target(editor, ui, assets, entity.id))
    } else {
        None
    };
    if let Some(Err(error)) = &target {
        ui.weak("嵌套实例的物理配置请在源 Prefab 中编辑。")
            .on_hover_text(error);
    }
    let editable = matches!(
        target,
        Some(Ok(comet_editor_core::FieldTarget::Prefab { .. }))
    ) && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    if let Some(body) = &entity.physics {
        ui.label(format!(
            "{} · {}",
            motion_name(body.motion),
            shape_name(&body.shape)
        ));
    }
    super::common::draw_fields(editor, ui, assets, &[entity], super::common::BODY, editable);
    super::common::draw_fields(
        editor,
        ui,
        assets,
        &[entity],
        super::common::CHARACTER,
        editable,
    );
}
