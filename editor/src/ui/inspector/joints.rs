use crate::session::Editor;
use comet_asset::AssetStore;
use comet_editor_core::JointEdit;
use comet_runtime::{JointKind, JointStatus};
use comet_scene::{BodyMotion, EntityData};
mod fields;

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
                    replace(
                        editor,
                        assets,
                        entity.id,
                        crate::joints::initial_joint(kind, entity),
                    );
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
                    ui.add_enabled_ui(editor.play.is_none(), |ui| {
                        if authored
                            && ui.button(format!("移除{}", name(kind))).clicked()
                            && !ui.input(|input| input.key_pressed(egui::Key::Escape))
                        {
                            let value = match kind {
                                JointKind::Point => JointEdit::Point(None),
                                JointKind::Hinge => JointEdit::Hinge(None),
                                JointKind::Slider => JointEdit::Slider(None),
                            };
                            replace(editor, assets, entity.id, value);
                            return;
                        }
                        fields::draw(editor, ui, assets, &[entity], kind);
                    });
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

pub(super) fn draw_common(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
) -> bool {
    for kind in [JointKind::Point, JointKind::Hinge, JointKind::Slider] {
        let field = fields::connection(kind);
        if !entities.iter().all(|entity| {
            comet_scene::schema::descriptor(field)
                .unwrap()
                .read(entity)
                .is_some()
        }) {
            continue;
        }
        ui.separator();
        ui.strong(name(kind));
        let revision = editor.document.revision();
        fields::draw(editor, ui, assets, entities, kind);
        if revision != editor.document.revision() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests;
