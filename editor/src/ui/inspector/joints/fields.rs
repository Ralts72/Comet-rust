use super::super::{common, properties::shared_value, references};
use crate::session::Editor;
use comet_asset::AssetStore;
use comet_runtime::JointKind;
use comet_scene::{
    BodyMotion, EntityData, EntityRef, FieldUpdate, JointConnection,
    schema::{self, FieldKey, FieldValue},
};

mod axial;
mod numbers;
use numbers::vector;

pub(super) fn connection(kind: JointKind) -> FieldKey {
    match kind {
        JointKind::Point => schema::POINT_CONNECTION,
        JointKind::Hinge => schema::HINGE_CONNECTION,
        JointKind::Slider => schema::SLIDER_CONNECTION,
    }
}
fn keys(kind: JointKind) -> &'static [FieldKey] {
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
fn editable(editor: &Editor, ui: &egui::Ui) -> bool {
    editor.play.is_none() && !editor.is_busy() && !editor.documents.is_blocking() && ui.is_enabled()
}
fn assign(
    editor: &mut Editor,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    field: FieldKey,
    value: impl Fn(FieldValue) -> FieldValue,
) {
    let descriptor = schema::descriptor(field).unwrap();
    let updates: Vec<_> = entities
        .iter()
        .filter_map(|entity| {
            let before = descriptor.read(entity).unwrap();
            let after = value(before.clone());
            (before != after).then_some(FieldUpdate {
                entity: entity.id,
                field,
                value: after,
            })
        })
        .collect();
    let result = editor
        .commit()
        .and_then(|_| editor.edit_fields(&updates, assets))
        .and_then(|_| editor.commit());
    editor.notice(result);
}

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    kind: JointKind,
) {
    let primary = entities[0];
    let fields = keys(kind);
    let enabled = editable(editor, ui);
    let revision = editor.document.revision();
    ui.push_id(
        (
            "joint-fields",
            editor.world(),
            primary.id,
            editor.selection.generation(),
            fields[0].component,
        ),
        |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                common::draw_flags(
                    editor,
                    ui,
                    assets,
                    entities,
                    &[common::FlagField {
                        key: fields[0],
                        label: "启用关节",
                    }],
                )
            });
            if editor.document.revision() != revision {
                return;
            }
            let Some((FieldValue::JointConnection(current), mixed)) =
                shared_value(entities, fields[1])
            else {
                return;
            };
            let choices = if enabled {
                references::choices(editor, ui, assets, entities, fields[1])
            } else {
                Ok(Default::default())
            };
            let selected_name = match &current {
                JointConnection::World => "世界".to_owned(),
                JointConnection::Unbound => "未绑定".to_owned(),
                JointConnection::Body(EntityRef::Local { entity: id }) => choices
                    .as_ref()
                    .ok()
                    .and_then(|choices| choices.iter().find(|(entity, _)| entity == id))
                    .map(|(_, label)| label.clone())
                    .or_else(|| {
                        editor
                            .last_snapshot
                            .entities
                            .iter()
                            .find(|entity| entity.id == *id)
                            .map(|entity| entity.name.clone())
                    })
                    .unwrap_or_else(|| format!("缺失实体 {id}")),
                _ => "当前连接（只读）".to_owned(),
            };
            let mut selected = None;
            ui.add_enabled_ui(enabled && choices.is_ok(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(if mixed {
                        "连接刚体（多值）"
                    } else {
                        "连接刚体"
                    });
                    egui::ComboBox::from_id_salt("connection")
                        .selected_text(if mixed { "多值" } else { &selected_name })
                        .width(ui.available_width().min(230.))
                        .truncate()
                        .show_ui(ui, |ui| {
                            ui.add_enabled_ui(
                                entities.iter().all(|entity| {
                                    entity
                                        .physics
                                        .as_ref()
                                        .is_some_and(|body| body.motion == BodyMotion::Dynamic)
                                }),
                                |ui| {
                                    if ui
                                        .selectable_label(
                                            !mixed && current == JointConnection::World,
                                            "世界",
                                        )
                                        .clicked()
                                    {
                                        selected = Some(JointConnection::World);
                                    }
                                },
                            );
                            if ui
                                .selectable_label(
                                    !mixed && current == JointConnection::Unbound,
                                    "未绑定",
                                )
                                .clicked()
                            {
                                selected = Some(JointConnection::Unbound);
                            }
                            if let Ok(choices) = &choices {
                                for (id, name) in choices.iter() {
                                    let value = JointConnection::Body(EntityRef::local(*id));
                                    if ui
                                        .selectable_label(!mixed && current == value, name)
                                        .clicked()
                                    {
                                        selected = Some(value);
                                    }
                                }
                            }
                        });
                })
            });
            if let Err(error) = choices {
                ui.weak(error);
            }
            let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
            if let Some(connection) = selected.filter(|_| enabled && !cancel) {
                let ids: Vec<_> = entities.iter().map(|entity| entity.id).collect();
                let result = editor
                    .commit()
                    .and_then(|_| {
                        editor.replace_joint_connections(&ids, fields[1], connection, assets)
                    })
                    .and_then(|_| editor.commit());
                editor.notice(result);
            }
            if editor.document.revision() != revision {
                return;
            }
            ui.add_enabled_ui(enabled, |ui| {
                common::draw_flags(
                    editor,
                    ui,
                    assets,
                    entities,
                    &[common::FlagField {
                        key: fields[4],
                        label: "连接刚体之间允许碰撞",
                    }],
                )
            });
            if editor.document.revision() != revision {
                return;
            }
            vector(editor, ui, assets, entities, fields[2], "本地锚点（米）");
            vector(
                editor,
                ui,
                assets,
                entities,
                fields[3],
                if mixed {
                    "连接锚点（各连接端坐标，米）"
                } else if matches!(current, JointConnection::Body(_)) {
                    "连接锚点（目标本地，米）"
                } else {
                    "连接锚点（世界，米）"
                },
            );
            if fields.len() > 5 {
                ui.weak("X 为主轴，Y 定义零角参考；朝向不继承缩放。");
                vector(
                    editor,
                    ui,
                    assets,
                    entities,
                    fields[5],
                    "自身坐标朝向（XYZ，度）",
                );
                vector(
                    editor,
                    ui,
                    assets,
                    entities,
                    fields[6],
                    "连接端坐标朝向（XYZ，度）",
                );
                axial::draw(editor, ui, assets, entities, kind);
            }
        },
    );
}
