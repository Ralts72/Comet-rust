mod gizmo;
mod inspector;
#[cfg(test)]
mod tests;
use crate::session::Editor;
use anyhow::Result;
use comet_asset::AssetStore;
use comet_core::EntityId;
use comet_scene::EntityData;
use egui::{Color32, Rect, Vec2};
pub fn show(
    editor: &mut Editor,
    root: &mut egui::Ui,
    assets: &mut AssetStore,
) -> Result<(Rect, Option<[f32; 2]>)> {
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
                    .add_enabled(editor.play.is_none(), egui::Button::new("Save"))
                    .clicked()
                {
                    let result = editor.save(assets);
                    editor.notice(result);
                }
                if ui
                    .add_enabled(editor.play.is_none(), egui::Button::new("Undo"))
                    .clicked()
                {
                    let result = editor.commit().and_then(|_| editor.document.undo());
                    editor.notice(result);
                }
                if ui
                    .add_enabled(editor.play.is_none(), egui::Button::new("Redo"))
                    .clicked()
                {
                    let result = editor.commit().and_then(|_| editor.document.redo());
                    editor.notice(result);
                }
                ui.separator();
                if editor.play.is_none() {
                    if ui.button("Play").clicked() {
                        let result = editor.start(assets);
                        editor.notice(result);
                    }
                } else {
                    let paused = editor.play.as_ref().unwrap().is_paused();
                    if ui.button(if paused { "Resume" } else { "Pause" }).clicked() {
                        editor.play.as_mut().unwrap().set_paused(!paused);
                    }
                    if ui.add_enabled(paused, egui::Button::new("Step")).clicked() {
                        let result = editor.play.as_mut().unwrap().single_step();
                        editor.notice(result);
                    }
                    if ui.button("Stop").clicked() {
                        editor.stop();
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(if editor.document.is_dirty() {
                        "Unsaved *"
                    } else {
                        "Saved"
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
                    "{} entities · GPU {:.2} ms",
                    editor.last_snapshot.entities.len(),
                    editor.gpu_ms
                ));
                if let Some(play) = &editor.play {
                    ui.small(format!("Play {:.2}s", play.elapsed()));
                }
            });
        });
    egui::Panel::left("scene")
        .default_size(220.)
        .show(root, |ui| {
            ui.heading("Scene");
            ui.label(
                egui::RichText::new(if editor.play.is_some() {
                    "PLAY WORLD"
                } else {
                    "AUTHORING"
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
            ui.heading("Inspector");
            ui.add_space(12.);
            if let Some(entity) = editor.selected_entity() {
                inspector::draw(editor, ui, assets, &entity);
            } else {
                ui.weak("Select an object in the scene or viewport.");
            }
            ui.separator();
            ui.checkbox(&mut editor.shadows, "Directional shadows");
            if let Some(message) = editor.error.clone() {
                ui.add_space(12.);
                ui.colored_label(Color32::from_rgb(244, 141, 131), message);
                if ui.small_button("Dismiss").clicked() {
                    editor.error = None;
                }
            }
        });
    let mut viewport = Rect::NOTHING;
    let mut pick = None;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(root, |ui| {
            viewport = ui.max_rect();
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            comet_application::orbit(&mut editor.camera, &response);
            ui.painter().text(
                viewport.left_top() + Vec2::new(18., 16.),
                egui::Align2::LEFT_TOP,
                "PERSPECTIVE  /  LIT",
                egui::FontId::proportional(12.),
                Color32::from_gray(190),
            );
            ui.painter().text(
                viewport.left_bottom() + Vec2::new(18., -18.),
                egui::Align2::LEFT_BOTTOM,
                "Right drag: orbit   ·   Scroll: zoom   ·   Drag axes: translate",
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
    Ok((viewport, pick))
}
