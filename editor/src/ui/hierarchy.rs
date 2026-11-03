use crate::session::Editor;
use comet_asset::AssetStore;
use comet_core::EntityId;
use comet_scene::EntityData;

#[derive(Clone, Copy)]
enum Action {
    CreateChild(EntityId),
    Duplicate(EntityId),
    Delete(EntityId),
}

pub(super) fn show(editor: &mut Editor, root: &mut egui::Ui, assets: &mut AssetStore) {
    egui::Panel::left("scene")
        .default_size(220.)
        .min_size(180.)
        .show(root, |ui| {
            ui.horizontal(|ui| {
                ui.heading("层级");
                if ui
                    .add_enabled(
                        editor.play.is_none() && !editor.export.is_running(),
                        egui::Button::new("新建对象"),
                    )
                    .clicked()
                {
                    let result = editor.create_object(None, assets);
                    editor.notice(result);
                }
            });
            ui.label(
                editor
                    .source
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            )
            .on_hover_text(editor.source.display().to_string());
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
                action: &mut Option<Action>,
                can_edit: bool,
            ) {
                for entity in entities.iter().filter(|e| e.parent == parent) {
                    let response = ui.selectable_label(*selected == Some(entity.id), &entity.name);
                    if response.clicked() {
                        *selected = Some(entity.id);
                    }
                    response.context_menu(|ui| {
                        ui.add_enabled_ui(can_edit, |ui| {
                            for (label, operation) in [
                                ("创建子对象", Action::CreateChild(entity.id)),
                                ("复制", Action::Duplicate(entity.id)),
                                ("删除", Action::Delete(entity.id)),
                            ] {
                                if ui.button(label).clicked() {
                                    *action = Some(operation);
                                    ui.close();
                                }
                            }
                        });
                    });
                    ui.indent(entity.id.to_string(), |ui| {
                        tree(ui, entities, Some(entity.id), selected, action, can_edit)
                    });
                }
            }
            let before = editor.selected;
            let mut action = None;
            let can_edit = editor.play.is_none() && !editor.export.is_running();
            egui::ScrollArea::vertical().show(ui, |ui| {
                tree(
                    ui,
                    &snapshot.entities,
                    None,
                    &mut editor.selected,
                    &mut action,
                    can_edit,
                )
            });
            if before != editor.selected {
                let result = editor.commit();
                editor.notice(result);
            }
            if let Some(action) = action {
                let result = match action {
                    Action::CreateChild(id) => editor.create_object(Some(id), assets),
                    Action::Duplicate(id) => editor.duplicate_object(id, assets),
                    Action::Delete(id) => editor.delete_object(id, assets),
                };
                editor.notice(result);
            }
        });
}
