use super::*;
use crate::ui::inspector::{
    common,
    properties::{PropertyGesture, shared_value},
};

fn editable(editor: &Editor, ui: &egui::Ui) -> bool {
    editor.play.is_none() && !editor.is_busy() && !editor.documents.is_blocking() && ui.is_enabled()
}
fn motion_name(motion: BodyMotion) -> &'static str {
    match motion {
        BodyMotion::Fixed => "静态",
        BodyMotion::Dynamic => "动态",
        BodyMotion::Kinematic => "运动学",
    }
}
fn shape_name(kind: ColliderKind) -> &'static str {
    match kind {
        ColliderKind::Box => "盒体",
        ColliderKind::Sphere => "球体",
        ColliderKind::Capsule => "胶囊",
    }
}
pub(in crate::ui::inspector) fn body_fields(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
) -> bool {
    let Some(primary) = entities.first() else {
        return false;
    };
    if !entities.iter().all(|entity| entity.physics.is_some()) {
        return false;
    }
    let enabled = editable(editor, ui);
    let unrestricted = !entities.iter().any(|entity| entity.character.is_some());
    let ids: Vec<_> = entities.iter().map(|entity| entity.id).collect();
    let revision = editor.document.revision();
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
    if let Some((FieldValue::BodyMotion(current), mixed)) =
        shared_value(entities, schema::BODY_MOTION)
    {
        let mut selected = None;
        ui.add_enabled_ui(enabled && unrestricted, |ui| {
            ui.horizontal(|ui| {
                ui.label(if mixed {
                    "运动类型（多值）"
                } else {
                    "运动类型"
                });
                egui::ComboBox::from_id_salt((
                    "body-motion",
                    editor.world(),
                    primary.id,
                    editor.selection.generation(),
                ))
                .selected_text(if mixed {
                    "多值"
                } else {
                    motion_name(current)
                })
                .show_ui(ui, |ui| {
                    for motion in [
                        BodyMotion::Fixed,
                        BodyMotion::Dynamic,
                        BodyMotion::Kinematic,
                    ] {
                        if ui
                            .selectable_label(!mixed && motion == current, motion_name(motion))
                            .clicked()
                        {
                            selected = Some(motion);
                        }
                    }
                });
            });
        });
        if enabled
            && unrestricted
            && !cancel
            && let Some(motion) = selected
        {
            let updates = ids
                .iter()
                .map(|id| FieldUpdate {
                    entity: *id,
                    field: schema::BODY_MOTION,
                    value: FieldValue::BodyMotion(motion),
                })
                .collect::<Vec<_>>();
            let result = editor
                .commit()
                .and_then(|_| editor.edit_fields(&updates, assets))
                .and_then(|_| editor.commit());
            editor.notice(result);
        }
    }
    if editor.document.revision() != revision {
        return true;
    }
    let kind = ColliderKind::of(&primary.physics.as_ref().unwrap().shape);
    let mixed = entities
        .iter()
        .any(|entity| ColliderKind::of(&entity.physics.as_ref().unwrap().shape) != kind);
    let mut selected = None;
    ui.add_enabled_ui(enabled && unrestricted, |ui| {
        ui.horizontal(|ui| {
            ui.label(if mixed {
                "形状（多值）"
            } else {
                "形状"
            });
            egui::ComboBox::from_id_salt((
                "body-shape",
                editor.world(),
                primary.id,
                editor.selection.generation(),
            ))
            .selected_text(if mixed { "多值" } else { shape_name(kind) })
            .show_ui(ui, |ui| {
                for choice in [
                    ColliderKind::Box,
                    ColliderKind::Sphere,
                    ColliderKind::Capsule,
                ] {
                    if ui
                        .selectable_label(!mixed && choice == kind, shape_name(choice))
                        .clicked()
                    {
                        selected = Some(choice);
                    }
                }
            });
        });
    });
    if enabled
        && unrestricted
        && !cancel
        && let Some(kind) = selected
    {
        let result = editor
            .commit()
            .and_then(|_| editor.change_collider_kind(&ids, kind, assets))
            .and_then(|_| editor.commit());
        editor.notice(result);
    }
    if editor.document.revision() != revision {
        return true;
    }
    if mixed {
        ui.weak("先统一碰撞形状，再编辑尺寸；已有相同形状的尺寸会保留。");
    } else {
        let dimensions: &[(usize, &str, f32)] = match kind {
            ColliderKind::Box => &[
                (0, "盒体半尺寸 X", 0.001),
                (1, "盒体半尺寸 Y", 0.001),
                (2, "盒体半尺寸 Z", 0.001),
            ],
            ColliderKind::Sphere => &[(0, "球／胶囊半径", 0.001)],
            ColliderKind::Capsule => &[
                (0, "胶囊半高（不含端部，米）", 0.),
                (1, "球／胶囊半径", 0.001),
            ],
        };
        for &(axis, label, min) in dimensions {
            dimension(editor, ui, assets, entities, (axis, label, min), enabled);
        }
    }
    ui.add_enabled_ui(enabled && unrestricted, |ui| {
        common::draw_flags(editor, ui, assets, entities, common::BODY_FLAGS)
    });
    if !unrestricted {
        ui.weak("角色使用非触发器的运动学胶囊刚体。");
    }
    common::draw_fields(editor, ui, assets, entities, common::BODY, enabled);
    ui.collapsing("碰撞过滤", |ui| {
        ui.weak("十六进制位掩码；双方过滤规则同时满足才发生碰撞。");
        for (field, label) in [
            (schema::COLLISION_LAYER, "所属碰撞层"),
            (schema::COLLISION_MASK, "检测碰撞层"),
        ] {
            let Some((FieldValue::Unsigned(mut value), mixed)) = shared_value(entities, field)
            else {
                continue;
            };
            ui.push_id(
                (
                    "collision-filter",
                    editor.world(),
                    primary.id,
                    editor.selection.generation(),
                    field.field,
                ),
                |ui| {
                    let mut gesture = PropertyGesture::new(ui, enabled, field);
                    let label = if mixed {
                        format!("{label}（多值）")
                    } else {
                        label.into()
                    };
                    let response = gesture.mask(ui, &mut value, &label);
                    let updates = if gesture.observe(&response, mixed) {
                        ids.iter()
                            .map(|id| FieldUpdate {
                                entity: *id,
                                field,
                                value: FieldValue::Unsigned(value),
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    gesture.apply(editor, ui, assets, updates);
                },
            );
        }
    });
    false
}
fn dimension(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    property: (usize, &str, f32),
    enabled: bool,
) {
    let (axis, label, min) = property;
    let component = |shape: &ColliderShape| match shape {
        ColliderShape::Box { half_extents } => half_extents[axis],
        ColliderShape::Sphere { radius } => *radius,
        ColliderShape::Capsule {
            half_height,
            radius,
        } => {
            if axis == 0 {
                *half_height
            } else {
                *radius
            }
        }
    };
    let primary = entities[0];
    let mut value = component(&primary.physics.as_ref().unwrap().shape);
    let mixed = entities
        .iter()
        .any(|entity| component(&entity.physics.as_ref().unwrap().shape) != value);
    ui.push_id(
        (
            "collider-dimension",
            editor.world(),
            primary.id,
            editor.selection.generation(),
            axis,
        ),
        |ui| {
            let mut gesture = PropertyGesture::new(ui, enabled, schema::COLLIDER_SHAPE);
            let label = if mixed {
                format!("{label}（多值）")
            } else {
                label.into()
            };
            let minimum = min.min(value);
            let response = gesture.number(
                ui,
                egui::DragValue::new(&mut value)
                    .prefix(format!("{label} "))
                    .range(minimum..=f32::MAX)
                    .clamp_existing_to_range(false)
                    .speed(0.05)
                    .max_decimals(3),
            );
            let updates = if gesture.observe(&response, mixed) {
                entities
                    .iter()
                    .map(|entity| {
                        let mut shape = entity.physics.as_ref().unwrap().shape.clone();
                        match &mut shape {
                            ColliderShape::Box { half_extents } => half_extents[axis] = value,
                            ColliderShape::Sphere { radius } => *radius = value,
                            ColliderShape::Capsule {
                                half_height,
                                radius,
                            } => {
                                if axis == 0 {
                                    *half_height = value;
                                } else {
                                    *radius = value;
                                }
                            }
                        }
                        FieldUpdate {
                            entity: entity.id,
                            field: schema::COLLIDER_SHAPE,
                            value: FieldValue::ColliderShape(shape),
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            gesture.apply(editor, ui, assets, updates);
        },
    );
}
pub(in crate::ui::inspector) fn character_fields(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
) {
    let enabled = editable(editor, ui);
    ui.add_enabled_ui(enabled, |ui| {
        common::draw_flags(editor, ui, assets, entities, common::CHARACTER_FLAGS)
    });
    if entities.len() > 1 {
        ui.weak("场景只允许一个玩家控制角色；统一启用需要单选。");
    }
    common::draw_fields(editor, ui, assets, entities, common::CHARACTER, enabled);
}
