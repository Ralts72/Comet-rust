use super::properties::{PropertyGesture, shared_value};
use crate::session::Editor;
use comet_asset::AssetStore;
use comet_scene::{
    EntityData, FieldUpdate,
    schema::{self, FieldKey, FieldValue},
};

pub(super) struct ScalarField {
    key: FieldKey,
    label: &'static str,
    min: f32,
    max: f32,
}
struct FlagField {
    key: FieldKey,
    label: &'static str,
}
const MODEL: &[ScalarField] = &[ScalarField {
    key: schema::ROUGHNESS,
    label: "粗糙度",
    min: 0.,
    max: 1.,
}];
pub(super) const BODY: &[ScalarField] = &[
    ScalarField {
        key: schema::MASS,
        label: "质量 (kg)",
        min: 0.001,
        max: f32::MAX,
    },
    ScalarField {
        key: schema::FRICTION,
        label: "摩擦系数",
        min: 0.,
        max: f32::MAX,
    },
];
pub(super) const CHARACTER: &[ScalarField] = &[
    ScalarField {
        key: schema::MOVE_SPEED,
        label: "移动速度 (m/s)",
        min: 0.001,
        max: f32::MAX,
    },
    ScalarField {
        key: schema::JUMP_SPEED,
        label: "跳跃速度 (m/s)",
        min: 0.001,
        max: f32::MAX,
    },
    ScalarField {
        key: schema::CROUCH_SPEED,
        label: "蹲伏速度 (m/s)",
        min: 0.001,
        max: f32::MAX,
    },
    ScalarField {
        key: schema::CROUCH_HEIGHT_RATIO,
        label: "蹲伏高度比例",
        min: 0.001,
        max: 1.,
    },
];
const AUDIO: &[ScalarField] = &[ScalarField {
    key: schema::AUDIO_GAIN,
    label: "音量",
    min: 0.,
    max: 1.,
}];
const AUDIO_DISTANCES: &[ScalarField] = &[
    ScalarField {
        key: schema::AUDIO_MIN_DISTANCE,
        label: "开始衰减距离（米）",
        min: 0.,
        max: f32::MAX,
    },
    ScalarField {
        key: schema::AUDIO_MAX_DISTANCE,
        label: "最大距离（米）",
        min: 0.,
        max: f32::MAX,
    },
];
const AUDIO_FLAGS: &[FlagField] = &[
    FlagField {
        key: schema::AUDIO_LOOPING,
        label: "循环",
    },
    FlagField {
        key: schema::AUDIO_AUTOPLAY,
        label: "启动时播放",
    },
    FlagField {
        key: schema::AUDIO_SPATIAL,
        label: "空间音效",
    },
];
const SCRIPT_FLAGS: &[FlagField] = &[FlagField {
    key: schema::SCRIPT_ENABLED,
    label: "启用脚本",
}];
const NAVIGATION: &[ScalarField] = &[ScalarField {
    key: schema::NAVIGATION_ARRIVAL_DISTANCE,
    label: "到达距离",
    min: 0.001,
    max: f32::MAX,
}];
const NAVIGATION_FLAGS: &[FlagField] = &[FlagField {
    key: schema::NAVIGATION_ENABLED,
    label: "启用导航代理",
}];
const ANIMATOR: &[ScalarField] = &[ScalarField {
    key: schema::ANIMATOR_SPEED,
    label: "播放速度",
    min: 0.,
    max: f32::MAX,
}];
const ANIMATOR_FLAGS: &[FlagField] = &[FlagField {
    key: schema::ANIMATOR_LOOPING,
    label: "循环",
}];

pub(in crate::ui) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
) {
    if entities.is_empty() {
        return;
    }
    let groups: &[(&str, &[ScalarField], &[FlagField])] = &[
        ("网格渲染", MODEL, &[]),
        ("刚体", BODY, &[]),
        ("角色控制器", CHARACTER, &[]),
        ("音频源", AUDIO, AUDIO_FLAGS),
        ("脚本", &[], SCRIPT_FLAGS),
        ("导航代理", NAVIGATION, NAVIGATION_FLAGS),
        ("动画", ANIMATOR, ANIMATOR_FLAGS),
    ];
    for &(name, fields, flags) in groups {
        let key = fields
            .first()
            .map(|field| field.key)
            .or_else(|| flags.first().map(|field| field.key))
            .unwrap();
        let component = schema::descriptor(key).unwrap();
        if !entities
            .iter()
            .all(|entity| component.read(entity).is_some())
        {
            continue;
        }
        ui.separator();
        ui.strong(name);
        if key == schema::NAVIGATION_ARRIVAL_DISTANCE
            && super::references::draw(editor, ui, assets, entities, &[schema::NAVIGATION_TARGET])
        {
            return;
        }
        let assets_fields: &[FieldKey] = match key {
            schema::ROUGHNESS => &[schema::MESH, schema::MATERIAL],
            schema::AUDIO_GAIN => &[schema::AUDIO_CLIP],
            schema::SCRIPT_ENABLED => &[schema::SCRIPT_ASSET],
            schema::ANIMATOR_SPEED => &[
                schema::ANIMATOR_SKELETON,
                schema::ANIMATOR_CONTROLLER,
                schema::ANIMATOR_CLIP,
            ],
            _ => &[],
        };
        if !assets_fields.is_empty() {
            let revision = editor.document.revision();
            super::assets::draw_common(editor, ui, assets, entities, assets_fields);
            if editor.document.revision() != revision {
                return;
            }
        }
        draw_fields(editor, ui, assets, entities, fields, editor.play.is_none());
        if key != schema::ANIMATOR_SPEED
            || entities
                .iter()
                .all(|e| e.animator.as_ref().is_none_or(|a| a.controller.is_none()))
        {
            draw_flags(editor, ui, assets, entities, flags);
        } else {
            ui.weak("状态的片段和循环由动画控制器配置。");
        }
        if key == schema::ANIMATOR_SPEED
            && ui
                .add_enabled(
                    editor.play.is_none() && !editor.is_busy() && !editor.documents.is_blocking(),
                    egui::Button::new("重建绑定表"),
                )
                .on_hover_text(
                    "逐个使用当前骨架，保留有效绑定，移除过期源节点，并为新增节点保留未绑定状态。",
                )
                .clicked()
            && !ui.input(|input| input.key_pressed(egui::Key::Escape))
        {
            let ids = entities.iter().map(|entity| entity.id).collect::<Vec<_>>();
            super::animation::rebuild_bindings(editor, assets, &ids);
            return;
        }
        if key == schema::AUDIO_GAIN {
            // Match the legacy Inspector: distances are disabled only when every source is 2D.
            let spatial = entities
                .iter()
                .any(|entity| entity.audio_source.as_ref().unwrap().spatial);
            draw_fields(
                editor,
                ui,
                assets,
                entities,
                AUDIO_DISTANCES,
                editor.play.is_none() && spatial,
            );
        }
    }
    if entities.iter().all(|entity| entity.skin.is_some()) {
        ui.separator();
        ui.strong("蒙皮");
        let revision = editor.document.revision();
        super::assets::draw_common(editor, ui, assets, entities, &[schema::SKIN_ASSET]);
        if editor.document.revision() != revision {
            return;
        }
        if super::references::draw(editor, ui, assets, entities, &[schema::SKIN_ANIMATOR]) {
            return;
        }
        ui.weak("替换蒙皮会保留各对象的动画实体绑定。");
    }
    if entities
        .iter()
        .all(|entity| entity.character_animation.is_some())
    {
        ui.separator();
        ui.strong("角色动画");
        if super::references::draw(
            editor,
            ui,
            assets,
            entities,
            &[
                schema::CHARACTER_ANIMATION_CHARACTER,
                schema::CHARACTER_ANIMATION_ANIMATOR,
            ],
        ) {
            return;
        }
    }
    super::overrides::draw_group(editor, ui, assets, entities);
    super::components::draw(editor, ui, assets, entities);
}

pub(super) fn draw_fields(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    fields: &[ScalarField],
    editable: bool,
) {
    let Some(primary) = entities.first() else {
        return;
    };
    for property in fields {
        let Some((FieldValue::Scalar(mut value), mixed)) = shared_value(entities, property.key)
        else {
            continue;
        };
        ui.push_id(
            (
                "scalar-property",
                editor.world(),
                primary.id,
                editor.selection.generation(),
                property.key.component,
                property.key.field,
            ),
            |ui| {
                let mut gesture = PropertyGesture::new(ui, editable, property.key);
                let label = if mixed {
                    format!("{}（多值）", property.label)
                } else {
                    property.label.into()
                };
                let range = property.min.min(value)..=property.max.max(value);
                let response = gesture.number(
                    ui,
                    egui::DragValue::new(&mut value)
                        .prefix(format!("{label} "))
                        .range(range)
                        .clamp_existing_to_range(false)
                        .speed(match property.key {
                            schema::ROUGHNESS | schema::AUDIO_GAIN | schema::ANIMATOR_SPEED => 0.01,
                            schema::AUDIO_MIN_DISTANCE | schema::AUDIO_MAX_DISTANCE => 0.1,
                            _ => 0.05,
                        })
                        .max_decimals(3),
                );
                let updates = if gesture.observe(&response, mixed) {
                    entities
                        .iter()
                        .map(|entity| FieldUpdate {
                            entity: entity.id,
                            field: property.key,
                            value: FieldValue::Scalar(value),
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                gesture.apply(editor, ui, assets, updates);
            },
        );
    }
}

fn draw_flags(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    fields: &[FlagField],
) {
    let Some(primary) = entities.first() else {
        return;
    };
    for property in fields {
        let Some((FieldValue::Bool(mut value), mixed)) = shared_value(entities, property.key)
        else {
            continue;
        };
        ui.push_id(
            (
                "common-flag",
                editor.world(),
                primary.id,
                editor.selection.generation(),
                property.key.component,
                property.key.field,
            ),
            |ui| {
                let editable = editor.play.is_none() && ui.is_enabled();
                let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
                let label = if mixed {
                    format!("{}（多值）", property.label)
                } else {
                    property.label.into()
                };
                let response = ui.add_enabled(
                    editable,
                    egui::Checkbox::new(&mut value, label).indeterminate(mixed),
                );
                if editable && response.changed() && !cancel {
                    let updates = entities
                        .iter()
                        .map(|entity| FieldUpdate {
                            entity: entity.id,
                            field: property.key,
                            value: FieldValue::Bool(value),
                        })
                        .collect::<Vec<_>>();
                    let result = editor
                        .commit()
                        .and_then(|_| editor.edit_fields(&updates, assets))
                        .and_then(|_| editor.commit());
                    editor.notice(result);
                }
            },
        );
    }
}
