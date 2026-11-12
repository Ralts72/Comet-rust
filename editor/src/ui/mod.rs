mod gizmo;
mod hierarchy;
mod inspector;
mod project;
mod properties;
mod resources;
mod theme;
mod viewport;
pub(crate) mod workspace;
pub(crate) use hierarchy::cancel_placement;
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
    theme::apply(root);
    let mut workspace = std::mem::take(&mut editor.workspace);
    egui::Panel::top("toolbar")
        .exact_size(44.)
        .show(root, |ui| {
            let width = ui.available_width();
            egui::ScrollArea::horizontal()
                .id_salt("editor-toolbar")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(width);
                    ui.horizontal_centered(|ui| {
                        ui.label(
                            egui::RichText::new("Comet")
                                .strong()
                                .color(Color32::from_gray(230)),
                        );
                        ui.separator();
                        project::files(editor, ui, assets);
                        project::menu(editor, ui);
                        project::edit(editor, ui, assets);
                        workspace.menu(ui, editor);
                        ui.separator();
                        if ui.button("新建场景").clicked() {
                            editor.documents.open_browser(BrowserMode::NewScene);
                        }
                        if ui.button("打开场景 / Prefab").clicked() {
                            editor.documents.open_browser(BrowserMode::Open);
                        }
                        if ui
                            .add_enabled(
                                editor.play.is_none(),
                                egui::Button::new(if editor.document.prefab_asset().is_some() {
                                    "保存 Prefab"
                                } else {
                                    "保存场景"
                                }),
                            )
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
                        if ui.button("导出 App...").clicked() {
                            editor.export.open = true;
                        }
                        ui.separator();
                        if editor.play.is_none() {
                            if ui.button("运行").clicked() {
                                let result = editor.start(assets);
                                if result.is_ok() {
                                    workspace.focus(workspace::Pane::Scene);
                                }
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
                        let used = ui.cursor().left() - ui.min_rect().left();
                        ui.add_space((width - used - 64.).max(8.));
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
    let view = workspace.show(editor, root, assets, active);
    editor.workspace = workspace;
    resources::preview(editor, root.ctx(), assets);
    let ctx = root.ctx();
    if hierarchy::placement_dialog(editor, ctx, assets, active) {
        return view.finish(editor, assets, ctx, active);
    }
    if !root.is_enabled() || editor.is_busy() {
        return view.finish(editor, assets, ctx, active);
    }
    if editor.play.is_none() && !ctx.text_edit_focused() {
        if active
            && editor.drag_origin.is_none()
            && !editor.documents.is_blocking()
            && !ctx.egui_is_using_pointer()
        {
            let (mut copy, mut paste) = (false, false);
            ctx.input_mut(|input| {
                input.events.retain(|event| {
                    let command = match event {
                        egui::Event::Copy => Some(false),
                        egui::Event::Paste(_) => Some(true),
                        egui::Event::Key {
                            key: egui::Key::Paste,
                            pressed: true,
                            repeat: false,
                            ..
                        } => Some(true),
                        egui::Event::Key {
                            key,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if modifiers.command && !modifiers.alt && !modifiers.shift => match key {
                            egui::Key::C => Some(false),
                            egui::Key::V => Some(true),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(is_paste) = command {
                        if is_paste {
                            paste = true;
                        } else {
                            copy = true;
                        }
                        false
                    } else {
                        true
                    }
                })
            });
            if copy && let Some(id) = editor.selection.active() {
                let result = editor.copy_objects(id, assets);
                editor.notice(result);
            }
            if paste && editor.clipboard.is_some() {
                let result = editor.paste_objects(None, assets);
                editor.notice(result);
            }
        }
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
