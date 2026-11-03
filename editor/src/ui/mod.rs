mod gizmo;
mod inspector;
#[cfg(test)]
mod tests;
use crate::session::Editor;
use anyhow::Result;
use comet_asset::AssetStore;
use comet_core::{ActionInput, EntityId};
use comet_scene::EntityData;
use egui::{Color32, Rect, Vec2};
pub fn show(
    editor: &mut Editor,
    root: &mut egui::Ui,
    assets: &mut AssetStore,
    active: bool,
) -> Result<(Rect, Option<[f32; 2]>, ActionInput)> {
    egui::Panel::top("toolbar")
        .exact_size(44.)
        .show(root, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(
                    egui::RichText::new("C O M E T")
                        .strong()
                        .color(Color32::from_rgb(236, 185, 99)),
                );
                ui.separator();
                if ui
                    .add_enabled(editor.play.is_none(), egui::Button::new("保存"))
                    .clicked()
                {
                    let result = editor.save(assets);
                    editor.notice(result);
                }
                if ui
                    .add_enabled(editor.play.is_none(), egui::Button::new("撤销"))
                    .clicked()
                {
                    let result = editor.commit().and_then(|_| editor.document.undo());
                    editor.notice(result);
                }
                if ui
                    .add_enabled(editor.play.is_none(), egui::Button::new("重做"))
                    .clicked()
                {
                    let result = editor.commit().and_then(|_| editor.document.redo());
                    editor.notice(result);
                }
                if ui.button("导出游戏").clicked() {
                    editor.export.open = true;
                }
                ui.separator();
                if editor.play.is_none() {
                    if ui.button("运行").clicked() {
                        let result = editor.start(assets);
                        editor.notice(result);
                    }
                } else {
                    let paused = editor.play.as_ref().unwrap().is_paused();
                    if ui.button(if paused { "继续" } else { "暂停" }).clicked() {
                        editor.play.as_mut().unwrap().set_paused(!paused);
                    }
                    if ui.add_enabled(paused, egui::Button::new("单步")).clicked() {
                        let result = editor.play.as_mut().unwrap().request_single_step();
                        editor.notice(result);
                    }
                    if ui.button("停止").clicked() {
                        editor.stop();
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(if editor.document.is_dirty() {
                        "未保存 *"
                    } else {
                        "已保存"
                    });
                });
            });
        });
    egui::Panel::bottom("status")
        .exact_size(27.)
        .show(root, |ui| {
            ui.horizontal(|ui| {
                ui.small(&editor.status);
                ui.separator();
                ui.small(format!(
                    "{} 个实体 · GPU {:.2} 毫秒",
                    editor.last_snapshot.entities.len(),
                    editor.gpu_ms
                ));
                if let Some(play) = &editor.play {
                    ui.small(format!(
                        "运行 {:.2} 秒 · {} 步 · 掉时 {:.3} 秒",
                        play.elapsed(),
                        play.steps(),
                        play.dropped_time()
                    ));
                }
            });
        });
    egui::Panel::left("scene")
        .default_size(220.)
        .show(root, |ui| {
            ui.heading("场景");
            ui.label(
                egui::RichText::new(if editor.play.is_some() {
                    "运行场景"
                } else {
                    "编辑场景"
                })
                .small()
                .weak(),
            );
            ui.add_space(10.);
            let snapshot = editor.last_snapshot.clone();
            fn tree(
                ui: &mut egui::Ui,
                entities: &[EntityData],
                parent: Option<EntityId>,
                selected: &mut Option<EntityId>,
            ) {
                for entity in entities.iter().filter(|e| e.parent == parent) {
                    if ui
                        .selectable_label(*selected == Some(entity.id), &entity.name)
                        .clicked()
                    {
                        *selected = Some(entity.id);
                    }
                    ui.indent(entity.id.to_string(), |ui| {
                        tree(ui, entities, Some(entity.id), selected)
                    });
                }
            }
            let before = editor.selected;
            egui::ScrollArea::vertical().show(ui, |ui| {
                tree(ui, &snapshot.entities, None, &mut editor.selected)
            });
            if before != editor.selected {
                let result = editor.commit();
                editor.notice(result);
            }
        });
    egui::Panel::right("inspector")
        .default_size(280.)
        .show(root, |ui| {
            ui.heading("属性");
            ui.add_space(12.);
            egui::ScrollArea::vertical()
                .id_salt("inspector-content")
                .show(ui, |ui| {
                    if let Some(entity) = editor.selected_entity() {
                        inspector::draw(editor, ui, assets, &entity);
                    } else {
                        ui.weak("请在场景树或视口中选择对象。");
                    }
                    ui.separator();
                    ui.checkbox(&mut editor.shadows, "方向光阴影");
                    if let Some(message) = editor.error.clone() {
                        ui.add_space(12.);
                        ui.colored_label(Color32::from_rgb(244, 141, 131), "操作未完成");
                        ui.collapsing("诊断详情", |ui| {
                            ui.label(message);
                        });
                        if ui.small_button("关闭提示").clicked() {
                            editor.error = None;
                        }
                    }
                });
        });
    let mut viewport = Rect::NOTHING;
    let mut pick = None;
    let mut actions = ActionInput::default();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(root, |ui| {
            viewport = ui.max_rect();
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            if response.clicked() {
                response.request_focus();
            }
            comet_application::orbit(&mut editor.camera, &response);
            if editor.play.is_some() {
                actions = comet_application::sample_actions(&response, active, editor.camera);
            }
            ui.painter().text(
                viewport.left_top() + Vec2::new(18., 16.),
                egui::Align2::LEFT_TOP,
                "透视 / 光照",
                egui::FontId::proportional(12.),
                Color32::from_gray(190),
            );
            ui.painter().text(
                viewport.left_bottom() + Vec2::new(18., -18.),
                egui::Align2::LEFT_BOTTOM,
                if editor.play.is_some() {
                    "WASD：移动 · 空格：跳跃 · Shift：跑步 · 右键拖动：环绕"
                } else {
                    "右键拖动：环绕 · 滚轮：缩放 · 拖动坐标轴：移动"
                },
                egui::FontId::proportional(11.),
                Color32::from_gray(160),
            );
            let gizmo = gizmo::draw(editor, ui, assets, viewport);
            if response.clicked()
                && !gizmo
                && let Some(pos) = response.interact_pointer_pos()
            {
                pick = Some([
                    (pos.x - viewport.min.x) / viewport.width(),
                    (pos.y - viewport.min.y) / viewport.height(),
                ]);
            }
        });
    let ctx = root.ctx();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        let result = editor.cancel();
        editor.notice(result);
    }
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
        let result = editor.save(assets);
        editor.notice(result);
    }
    if editor.play.is_none() && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
        let redo = ctx.input(|i| i.modifiers.shift);
        let result = editor.commit().and_then(|_| {
            if redo {
                editor.document.redo()
            } else {
                editor.document.undo()
            }
        });
        editor.notice(result);
    }
    Ok((viewport, pick, actions))
}
