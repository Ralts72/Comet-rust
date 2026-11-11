use super::inspector;
use crate::session::Editor;
use comet_asset::AssetStore;
use egui::Color32;

pub(super) fn draw(editor: &mut Editor, ui: &mut egui::Ui, assets: &mut AssetStore) {
    ui.add_space(10.);
    egui::ScrollArea::vertical()
        .id_salt("inspector-content")
        .show(ui, |ui| {
            // Keep large rig binding maps shared while widgets publish later candidates.
            let snapshot = editor.last_snapshot.clone();
            let entity = editor.selection.active().and_then(|id| {
                snapshot
                    .entities
                    .binary_search_by_key(&id, |entity| entity.id)
                    .ok()
                    .map(|index| &snapshot.entities[index])
            });
            if editor.selection.is_empty() {
                inspector::clear_model_draft(ui.ctx());
                ui.weak("请在层级或视口中选择实体。");
            } else if editor.selection.len() > 1 {
                inspector::clear_model_draft(ui.ctx());
                ui.strong(format!("共同属性 · {} 个实体", editor.selection.len()));
                if let Some(entity) = entity {
                    ui.label(format!("主实体：{}", entity.name));
                    let mut selected = vec![entity];
                    selected.extend(
                        editor
                            .selection
                            .ids()
                            .into_iter()
                            .filter(|id| *id != entity.id)
                            .filter_map(|id| {
                                snapshot
                                    .entities
                                    .binary_search_by_key(&id, |entity| entity.id)
                                    .ok()
                                    .map(|index| &snapshot.entities[index])
                            }),
                    );
                    if selected.len() == editor.selection.len() {
                        ui.weak("多值以主实体显示；修改一轴保留其他轴，数值使用各自的局部坐标。");
                        inspector::transform::draw(editor, ui, assets, &selected);
                        inspector::common::draw(editor, ui, assets, &selected);
                    } else {
                        ui.weak("所选实体已变化，请重新选择。");
                    }
                }
                ui.weak("拖动视口坐标轴可成组移动。右键层级实体可批量复制、删除或调整父级。");
                if editor.play.is_some() {
                    ui.weak("运行期间暂不支持成组修改。");
                }
            } else if let Some(entity) = entity {
                inspector::draw(editor, ui, assets, entity);
            }
            ui.separator();
            ui.checkbox(&mut editor.shadows, "方向光阴影");
            if let Some(message) = editor.error.clone() {
                ui.add_space(10.);
                ui.colored_label(Color32::from_rgb(244, 141, 131), "操作未完成");
                ui.collapsing("诊断详情", |ui| {
                    ui.label(message);
                });
                if ui.small_button("关闭提示").clicked() {
                    editor.error = None;
                }
            }
        });
}
