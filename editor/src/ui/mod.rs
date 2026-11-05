mod gizmo;
mod hierarchy;
mod inspector;
mod project;
mod resources;
mod viewport;
pub(crate) use viewport::cancel_gesture as cancel_viewport_gesture;
use viewport::click_modifiers;
#[cfg(test)]
mod tests;
use crate::documents::BrowserMode;
use crate::selection::SelectionPick;
use crate::session::Editor;
use crate::transform::GizmoTool;
use anyhow::Result;
use comet_asset::AssetStore;
use egui::Color32;
pub(crate) fn show(
    editor: &mut Editor,
    root: &mut egui::Ui,
    assets: &mut AssetStore,
    active: bool,
) -> Result<viewport::ViewportFrame> {
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
                if ui.button("新建场景").clicked() {
                    editor.documents.open_browser(BrowserMode::NewScene);
                }
                if ui.button("打开文档").clicked() {
                    editor.documents.open_browser(BrowserMode::Open);
                }
                project::menu(editor, ui);
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
                    if ui
                        .button("重开")
                        .on_hover_text("恢复本次运行的起始状态（R）")
                        .clicked()
                    {
                        let result = editor.restart_play();
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
    hierarchy::show(editor, root, assets);
    egui::Panel::right("inspector")
        .default_size(280.)
        .show(root, |ui| {
            ui.heading("属性");
            ui.add_space(12.);
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
                        ui.weak("请在场景树或视口中选择对象。");
                    } else if editor.selection.len() > 1 {
                        inspector::clear_model_draft(ui.ctx());
                        ui.strong(format!("已选择 {} 个对象", editor.selection.len()));
                        if let Some(entity) = entity {
                            ui.label(format!("主对象：{}", entity.name));
                        }
                        ui.weak(
                            "拖动视口坐标轴可成组移动。右键层级对象可批量复制、删除或调整父级。",
                        );
                        if editor.play.is_some() {
                            ui.weak("运行期间暂不支持成组修改。");
                        }
                    } else if let Some(entity) = entity {
                        inspector::draw(editor, ui, assets, entity);
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
    resources::show(editor, root, assets);
    let view = viewport::show(editor, root, assets, active);
    let ctx = root.ctx();
    if !root.is_enabled() || editor.is_busy() {
        return view.finish(editor, assets, ctx, active);
    }
    if editor.play.is_none() && !ctx.text_edit_focused() {
        if active
            && editor.drag_origin.is_none()
            && !view.owns_pointer
            && !ctx.egui_is_using_pointer()
            && !editor.documents.is_blocking()
        {
            // consume_key(NONE) also accepts Shift/Alt; tool keys require exact event modifiers.
            let tool = ctx.input_mut(|input| {
                let mut tool = None;
                input.events.retain(|event| {
                    let egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } = event
                    else {
                        return true;
                    };
                    if !modifiers.is_none() {
                        return true;
                    }
                    tool = Some(match key {
                        egui::Key::W => GizmoTool::Translate,
                        egui::Key::E => GizmoTool::Rotate,
                        egui::Key::R => GizmoTool::Scale,
                        _ => return true,
                    });
                    false
                });
                tool
            });
            if let Some(tool) = tool {
                editor.gizmo_tool = tool;
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            let ids = editor
                .last_snapshot
                .entities
                .iter()
                .map(|entity| entity.id)
                .collect::<Vec<_>>();
            editor.commit()?;
            editor.picking.clear();
            editor.selection.replace(ids);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D))
            && let Some(id) = editor.selection.active()
        {
            let result = editor.duplicate_object(id, assets);
            editor.notice(result);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
            && let Some(id) = editor.selection.active()
        {
            let result = editor.delete_object(id, assets);
            editor.notice(result);
        }
    }
    let new_project = ctx.input_mut(|i| {
        i.consume_key(
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
            egui::Key::N,
        )
    });
    let open_project = ctx.input_mut(|i| {
        i.consume_key(
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
            egui::Key::O,
        )
    });
    if new_project || open_project {
        let current = editor.project_root().to_owned();
        editor.documents.open_project_browser(new_project, &current);
    }
    if !new_project && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
        editor.documents.open_browser(BrowserMode::NewScene);
    }
    if !open_project && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
        editor.documents.open_browser(BrowserMode::Open);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        let result = editor.cancel();
        editor.notice(result);
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
        let result = editor.save(assets);
        editor.notice(result);
    }
    let redo = editor.play.is_none()
        && ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::Z,
            )
        });
    let undo = editor.play.is_none()
        && !redo
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
    if redo || undo {
        let result = editor.commit().and_then(|_| {
            if redo {
                editor.document.redo()
            } else {
                editor.document.undo()
            }
        });
        editor.notice(result);
    }
    view.finish(editor, assets, ctx, active)
}
