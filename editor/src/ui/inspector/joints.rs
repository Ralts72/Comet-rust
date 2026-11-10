use super::properties::ComponentEdit;
use crate::session::Editor;
use comet_asset::AssetStore;
use comet_core::math::{EulerRot, Quat};
use comet_editor_core::JointEdit;
use comet_runtime::{JointKind, JointStatus};
use comet_scene::{
    BodyMotion, EntityData, EntityRef, HingeJoint, JointConnection, JointFrame, JointLimits,
    JointMotor, JointSettings, PointJoint, SliderJoint,
    schema::{self, FieldKey},
};

fn vector(edit: &mut ComponentEdit, ui: &mut egui::Ui, label: &str, value: &mut [f32; 3]) {
    ui.label(label);
    ui.horizontal(|ui| {
        for (axis, v) in ["X", "Y", "Z"].into_iter().zip(value) {
            edit.number(ui, axis, v, f32::MIN, f32::MAX);
        }
    });
}
fn rotation(edit: &mut ComponentEdit, ui: &mut egui::Ui, label: &str, rotation: &mut [f32; 4]) {
    let (x, y, z) = Quat::from_array(*rotation).to_euler(EulerRot::XYZ);
    let mut degrees = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
    let before = degrees;
    vector(edit, ui, label, &mut degrees);
    // A passive draw must preserve the source quaternion, including its sign and rounding.
    if degrees != before {
        *rotation = Quat::from_euler(
            EulerRot::XYZ,
            degrees[0].to_radians(),
            degrees[1].to_radians(),
            degrees[2].to_radians(),
        )
        .to_array();
    }
}
fn angle(edit: &mut ComponentEdit, ui: &mut egui::Ui, label: &str, value: &mut f32) {
    let mut degrees = f64::from(*value).to_degrees();
    let before = degrees;
    let response = ui.add(
        egui::DragValue::new(&mut degrees)
            .prefix(format!("{label} "))
            .range(f64::from(f32::MIN).to_degrees()..=f64::from(f32::MAX).to_degrees())
            .speed(0.5)
            .max_decimals(3),
    );
    edit.finish |= response.drag_stopped() || response.lost_focus();
    if degrees != before {
        *value = degrees.to_radians() as f32;
    }
}

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.separator();
    ui.strong("关节");
    if entity.physics.is_none() || entity.character.is_some() || entity.parent.is_some() {
        ui.weak("关节需要根实体上的非角色刚体。");
        return;
    }
    let authored = editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .any(|e| e.id == entity.id);
    if entity.physics.as_ref().unwrap().motion != BodyMotion::Dynamic {
        ui.weak("启用关节至少需要一端为动态刚体；世界连接需要自身为动态刚体。");
    }
    for kind in [JointKind::Point, JointKind::Hinge, JointKind::Slider] {
        ui.push_id(("joint", editor.world(), entity.id, name(kind)), |ui| {
            let present = match kind {
                JointKind::Point => entity.point_joint.is_some(),
                JointKind::Hinge => entity.hinge_joint.is_some(),
                JointKind::Slider => entity.slider_joint.is_some(),
            };
            if !present {
                if authored
                    && ui
                        .add_enabled(
                            editor.play.is_none(),
                            egui::Button::new(format!("添加{}", name(kind))),
                        )
                        .clicked()
                {
                    replace(editor, assets, entity.id, initial_joint(kind, entity));
                }
                return;
            }
            egui::CollapsingHeader::new(name(kind))
                .default_open(true)
                .show(ui, |ui| {
                    if let Some(play) = &editor.play {
                        let state = match play.joint_status(entity.id, kind) {
                            Some(JointStatus::Active) => "已连接",
                            Some(JointStatus::Disabled) => "已禁用",
                            Some(JointStatus::Unbound) => "未绑定",
                            None => "无运行实例",
                        };
                        ui.label(format!("运行状态：{state}"));
                    }
                    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
                    let mut next = entity.clone();
                    let mut edit = ComponentEdit::default();
                    let mut remove = false;
                    ui.add_enabled_ui(editor.play.is_none(), |ui| {
                        if authored {
                            remove = ui.button(format!("移除{}", name(kind))).clicked();
                        }
                        match kind {
                            JointKind::Point => {
                                let joint = next.point_joint.as_mut().unwrap();
                                if settings(
                                    ui,
                                    editor,
                                    assets,
                                    entity,
                                    &mut joint.settings,
                                    &mut edit,
                                ) {
                                    let mut connected = JointFrame {
                                        anchor: joint.connected_anchor,
                                        ..Default::default()
                                    };
                                    rebase_connection(
                                        editor,
                                        entity,
                                        &joint.settings.connection,
                                        &JointFrame {
                                            anchor: joint.anchor,
                                            ..Default::default()
                                        },
                                        &mut connected,
                                    );
                                    joint.connected_anchor = connected.anchor;
                                }
                                vector(&mut edit, ui, "本地锚点（米）", &mut joint.anchor);
                                vector(
                                    &mut edit,
                                    ui,
                                    connected_anchor_label(&joint.settings.connection),
                                    &mut joint.connected_anchor,
                                );
                            }
                            JointKind::Hinge => {
                                let joint = next.hinge_joint.as_mut().unwrap();
                                if settings(
                                    ui,
                                    editor,
                                    assets,
                                    entity,
                                    &mut joint.settings,
                                    &mut edit,
                                ) {
                                    rebase_connection(
                                        editor,
                                        entity,
                                        &joint.settings.connection,
                                        &joint.frame,
                                        &mut joint.connected_frame,
                                    );
                                }
                                frames(
                                    ui,
                                    &joint.settings,
                                    &mut joint.frame,
                                    &mut joint.connected_frame,
                                    &mut edit,
                                );
                                axial(ui, true, &mut joint.limits, &mut joint.motor, &mut edit);
                            }
                            JointKind::Slider => {
                                let joint = next.slider_joint.as_mut().unwrap();
                                if settings(
                                    ui,
                                    editor,
                                    assets,
                                    entity,
                                    &mut joint.settings,
                                    &mut edit,
                                ) {
                                    rebase_connection(
                                        editor,
                                        entity,
                                        &joint.settings.connection,
                                        &joint.frame,
                                        &mut joint.connected_frame,
                                    );
                                }
                                frames(
                                    ui,
                                    &joint.settings,
                                    &mut joint.frame,
                                    &mut joint.connected_frame,
                                    &mut edit,
                                );
                                axial(ui, false, &mut joint.limits, &mut joint.motor, &mut edit);
                            }
                        }
                    });
                    if editor.play.is_some() || !ui.is_enabled() {
                        return;
                    }
                    if cancel {
                        let result = editor.cancel();
                        editor.notice(result);
                    } else if remove {
                        let value = match kind {
                            JointKind::Point => JointEdit::Point(None),
                            JointKind::Hinge => JointEdit::Hinge(None),
                            JointKind::Slider => JointEdit::Slider(None),
                        };
                        replace(editor, assets, entity.id, value);
                    } else {
                        if next != *entity {
                            let result =
                                editor.edit_joint_fields(entity, &next, fields(kind), assets);
                            editor.notice(result);
                        }
                        if edit.finish {
                            let result = editor.commit();
                            editor.notice(result);
                        }
                    }
                });
        });
    }
    if !authored {
        ui.weak("Prefab 关节结构请在源 Prefab 中编辑；此处仅覆盖已有字段。");
    }
}

fn name(kind: JointKind) -> &'static str {
    match kind {
        JointKind::Point => "点关节",
        JointKind::Hinge => "铰链关节",
        JointKind::Slider => "滑动关节",
    }
}
fn fields(kind: JointKind) -> &'static [FieldKey] {
    match kind {
        JointKind::Point => &[
            schema::POINT_ENABLED,
            schema::POINT_CONNECTION,
            schema::POINT_ANCHOR,
            schema::POINT_CONNECTED_ANCHOR,
            schema::POINT_CONTACTS_ENABLED,
        ],
        JointKind::Hinge => &[
            schema::HINGE_ENABLED,
            schema::HINGE_CONNECTION,
            schema::HINGE_ANCHOR,
            schema::HINGE_CONNECTED_ANCHOR,
            schema::HINGE_CONTACTS_ENABLED,
            schema::HINGE_ROTATION,
            schema::HINGE_CONNECTED_ROTATION,
            schema::HINGE_LIMITS,
            schema::HINGE_MOTOR,
        ],
        JointKind::Slider => &[
            schema::SLIDER_ENABLED,
            schema::SLIDER_CONNECTION,
            schema::SLIDER_ANCHOR,
            schema::SLIDER_CONNECTED_ANCHOR,
            schema::SLIDER_CONTACTS_ENABLED,
            schema::SLIDER_ROTATION,
            schema::SLIDER_CONNECTED_ROTATION,
            schema::SLIDER_LIMITS,
            schema::SLIDER_MOTOR,
        ],
    }
}
fn initial_joint(kind: JointKind, entity: &EntityData) -> JointEdit {
    let connected_frame = JointFrame {
        anchor: entity.transform.translation,
        rotation: entity.transform.rotation,
    };
    let settings = JointSettings {
        connection: if entity
            .physics
            .as_ref()
            .is_some_and(|body| body.motion == BodyMotion::Dynamic)
        {
            JointConnection::World
        } else {
            JointConnection::Unbound
        },
        ..Default::default()
    };
    match kind {
        JointKind::Point => JointEdit::Point(Some(PointJoint {
            settings,
            connected_anchor: entity.transform.translation,
            ..Default::default()
        })),
        JointKind::Hinge => JointEdit::Hinge(Some(HingeJoint {
            settings,
            connected_frame,
            ..Default::default()
        })),
        JointKind::Slider => JointEdit::Slider(Some(SliderJoint {
            settings,
            connected_frame,
            ..Default::default()
        })),
    }
}
fn replace(
    editor: &mut Editor,
    assets: &mut AssetStore,
    id: comet_core::EntityId,
    value: JointEdit,
) {
    let result = editor
        .commit()
        .and_then(|_| editor.edit_joint(id, value, assets))
        .and_then(|_| editor.commit());
    editor.notice(result);
}
fn connected_anchor_label(connection: &JointConnection) -> &'static str {
    if matches!(connection, JointConnection::Body(_)) {
        "连接锚点（目标本地，米）"
    } else {
        "连接锚点（世界，米）"
    }
}
fn settings(
    ui: &mut egui::Ui,
    editor: &Editor,
    assets: &mut AssetStore,
    entity: &EntityData,
    settings: &mut JointSettings,
    edit: &mut ComponentEdit,
) -> bool {
    edit.finish |= ui.checkbox(&mut settings.enabled, "启用关节").changed();
    let before = settings.connection.clone();
    let references = if editor.play.is_none() && ui.is_enabled() {
        editor
            .cached_field_references(ui.ctx(), entity.id, assets)
            .ok()
    } else {
        None
    };
    let selected = match &settings.connection {
        JointConnection::World => "世界".into(),
        JointConnection::Unbound => "未绑定".into(),
        JointConnection::Body(EntityRef::Local { entity: id }) => references
            .as_ref()
            .and_then(|references| references.targets.iter().find(|(entity, _)| entity == id))
            .map(|(_, name)| name.clone())
            .or_else(|| {
                editor
                    .last_snapshot
                    .entities
                    .iter()
                    .find(|entity| entity.id == *id)
                    .map(|entity| entity.name.clone())
            })
            .unwrap_or_else(|| format!("缺失实体 {id}")),
        JointConnection::Body(EntityRef::External { .. }) => "外部场景引用".into(),
        JointConnection::Body(EntityRef::InInstance { .. }) => "实例引用".into(),
    };
    egui::ComboBox::from_id_salt("connection")
        .selected_text(selected)
        .show_ui(ui, |ui| {
            ui.add_enabled_ui(
                entity.physics.as_ref().unwrap().motion == BodyMotion::Dynamic,
                |ui| {
                    ui.selectable_value(&mut settings.connection, JointConnection::World, "世界");
                },
            );
            ui.selectable_value(&mut settings.connection, JointConnection::Unbound, "未绑定");
            match editor.joint_targets(entity, ui.ctx(), assets) {
                Ok(targets) => {
                    for (id, name) in targets {
                        ui.selectable_value(
                            &mut settings.connection,
                            JointConnection::Body(EntityRef::local(id)),
                            name,
                        );
                    }
                }
                Err(error) => {
                    ui.weak(error.to_string());
                }
            }
        });
    edit.finish |= before != settings.connection;
    edit.finish |= ui
        .checkbox(&mut settings.contacts_enabled, "连接刚体之间允许碰撞")
        .changed();
    before != settings.connection
}
fn rebase_connection(
    editor: &Editor,
    entity: &EntityData,
    connection: &JointConnection,
    frame: &JointFrame,
    connected: &mut JointFrame,
) {
    use comet_core::math::Vec3;
    if matches!(connection, JointConnection::Unbound) {
        return;
    }
    let owner = &entity.transform;
    let rotation = Quat::from_array(owner.rotation).normalize();
    let point = Vec3::from_array(owner.translation)
        + rotation * (Vec3::from_array(owner.scale) * Vec3::from_array(frame.anchor));
    let orientation = rotation * Quat::from_array(frame.rotation).normalize();
    match connection {
        JointConnection::World => {
            connected.anchor = point.to_array();
            connected.rotation = orientation.to_array();
        }
        JointConnection::Body(EntityRef::Local { entity: id }) => {
            if let Some(target) = editor.last_snapshot.entities.iter().find(|e| e.id == *id) {
                let target_rotation = Quat::from_array(target.transform.rotation).normalize();
                connected.anchor = (target_rotation.inverse()
                    * (point - Vec3::from_array(target.transform.translation))
                    / Vec3::from_array(target.transform.scale))
                .to_array();
                connected.rotation = (target_rotation.inverse() * orientation).to_array();
            }
        }
        _ => {}
    }
}
fn frames(
    ui: &mut egui::Ui,
    settings: &JointSettings,
    frame: &mut JointFrame,
    connected: &mut JointFrame,
    edit: &mut ComponentEdit,
) {
    vector(edit, ui, "本地锚点（米）", &mut frame.anchor);
    vector(
        edit,
        ui,
        connected_anchor_label(&settings.connection),
        &mut connected.anchor,
    );
    ui.weak("X 为主轴，Y 定义零角参考；朝向不继承缩放。");
    rotation(edit, ui, "自身坐标朝向（XYZ，度）", &mut frame.rotation);
    rotation(
        edit,
        ui,
        "连接端坐标朝向（XYZ，度）",
        &mut connected.rotation,
    );
}
fn axial(
    ui: &mut egui::Ui,
    angular: bool,
    limits: &mut Option<JointLimits>,
    motor: &mut Option<JointMotor>,
    edit: &mut ComponentEdit,
) {
    let mut enabled = limits.is_some();
    if ui
        .checkbox(
            &mut enabled,
            if angular {
                "角度限位"
            } else {
                "行程限位"
            },
        )
        .changed()
    {
        *limits = enabled.then_some(JointLimits {
            min: if angular {
                -std::f32::consts::FRAC_PI_2
            } else {
                -1.
            },
            max: if angular {
                std::f32::consts::FRAC_PI_2
            } else {
                1.
            },
        });
        edit.finish = true;
    }
    if let Some(limits) = limits {
        if angular {
            ui.weak("铰链限位跨度不超过 360°；上下限相同会锁定该轴。");
            angle(edit, ui, "最小角度 (°)", &mut limits.min);
            angle(edit, ui, "最大角度 (°)", &mut limits.max);
        } else {
            edit.number(ui, "最小行程 (m)", &mut limits.min, f32::MIN, f32::MAX);
            edit.number(ui, "最大行程 (m)", &mut limits.max, f32::MIN, f32::MAX);
        }
    }
    let before_mode = match motor {
        None => 0,
        Some(JointMotor::Velocity { .. }) => 1,
        Some(JointMotor::Position { .. }) => 2,
    };
    let mut mode = before_mode;
    egui::ComboBox::from_id_salt("motor")
        .selected_text(["关闭驱动", "速度驱动", "位置驱动"][mode])
        .show_ui(ui, |ui| {
            for (i, name) in ["关闭驱动", "速度驱动", "位置驱动"].into_iter().enumerate()
            {
                ui.selectable_value(&mut mode, i, name);
            }
        });
    if mode != before_mode {
        *motor = match mode {
            1 => Some(JointMotor::Velocity {
                speed: 0.,
                max_effort: 10.,
            }),
            2 => Some(JointMotor::Position {
                target: 0.,
                frequency_hz: 2.,
                damping_ratio: 1.,
                max_effort: 10.,
            }),
            _ => None,
        };
        edit.finish = true;
    }
    if let Some(motor) = motor {
        let effort = match motor {
            JointMotor::Velocity { speed, max_effort } => {
                if angular {
                    angle(edit, ui, "目标角速度 (°/s)", speed);
                } else {
                    edit.number(ui, "目标速度 (m/s)", speed, f32::MIN, f32::MAX);
                }
                max_effort
            }
            JointMotor::Position {
                target,
                frequency_hz,
                damping_ratio,
                max_effort,
            } => {
                if angular {
                    angle(edit, ui, "目标角度 (°)", target);
                } else {
                    edit.number(ui, "目标行程 (m)", target, f32::MIN, f32::MAX);
                }
                edit.number(ui, "驱动频率 (Hz)", frequency_hz, 0.001, f32::MAX);
                edit.number(ui, "驱动阻尼比", damping_ratio, 0., f32::MAX);
                max_effort
            }
        };
        edit.number(
            ui,
            if angular {
                "最大力矩 (N·m)"
            } else {
                "最大驱动力 (N)"
            },
            effort,
            0.,
            f32::MAX,
        );
    }
}

#[cfg(test)]
mod tests;
