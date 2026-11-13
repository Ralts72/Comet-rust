use crate::{fields::ColliderKind, session::Editor};
use comet_asset::AssetStore;
use comet_core::EntityId;
use comet_editor_core::FieldTarget;
use comet_scene::{
    BodyMotion, Character, ColliderShape, EntityData, FieldUpdate, PhysicsBody,
    schema::{self, FieldValue},
};
mod fields;
pub(super) use fields::{body_fields, character_fields};

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.separator();
    ui.strong("物理");
    let target = if editor.play.is_none() {
        Some(super::overrides::target(editor, ui, assets, entity.id))
    } else {
        None
    };
    let authored = matches!(target, Some(Ok(FieldTarget::Authored(_))));
    let editable = target.as_ref().is_some_and(|target| target.is_ok())
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    if let Some(Err(error)) = &target {
        ui.weak("嵌套实例的物理配置请在源 Prefab 中编辑。")
            .on_hover_text(error);
    }
    if editor.play.is_some() {
        ui.weak("停止运行后可修改物理组件。");
    }
    if entity.physics.is_some() {
        if ui
            .add_enabled_ui(editable, |ui| body_fields(editor, ui, assets, &[entity]))
            .inner
        {
            return;
        }
        if entity.character.is_some() {
            ui.separator();
            ui.strong("角色控制器");
            ui.add_enabled_ui(editable, |ui| {
                character_fields(editor, ui, assets, &[entity])
            });
        }
        if authored {
            ui.add_enabled_ui(editable, |ui| {
                if ui
                    .button(if entity.character.is_some() {
                        "移除刚体及角色控制器"
                    } else {
                        "移除刚体"
                    })
                    .clicked()
                {
                    replace(editor, assets, entity.id, None, None);
                } else if entity.character.is_some() {
                    if ui.button("移除角色控制器").clicked() {
                        replace(editor, assets, entity.id, entity.physics.clone(), None);
                    }
                } else {
                    let body = entity.physics.as_ref().unwrap();
                    let compatible = body.motion == BodyMotion::Kinematic
                        && !body.sensor
                        && matches!(body.shape, ColliderShape::Capsule { .. });
                    if ui
                        .add_enabled(compatible, egui::Button::new("添加角色控制器"))
                        .on_hover_text("需要非触发器的运动学胶囊刚体、统一正缩放及直立朝向。")
                        .clicked()
                    {
                        replace(
                            editor,
                            assets,
                            entity.id,
                            entity.physics.clone(),
                            Some(new_character(editor)),
                        );
                    }
                }
            });
        } else {
            ui.weak("Prefab 实例的组件结构请在源 Prefab 中编辑。");
        }
    } else if authored && entity.parent.is_none() {
        ui.add_enabled_ui(editable, |ui| {
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
        });
    } else if authored {
        ui.weak("物理组件仅支持场景根实体。");
    }
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
