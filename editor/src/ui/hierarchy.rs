use crate::{
    selection::{SelectMode, Selection},
    session::Editor,
};
use comet_asset::AssetStore;
use comet_core::{EntityId, WorldId};
use comet_scene::EntityData;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
enum Action {
    CreateChild(EntityId),
    Duplicate(EntityId),
    Delete(EntityId),
    Reparent(EntityId, Option<EntityId>),
}

#[derive(Clone, Copy)]
struct HierarchyDrag {
    world: WorldId,
    entity: EntityId,
    selection_generation: u64,
}

fn drop_target(
    ui: &egui::Ui,
    response: &egui::Response,
    world: WorldId,
    selection_generation: u64,
    parent: Option<EntityId>,
) -> Option<Action> {
    if let Some(drag) = response.dnd_hover_payload::<HierarchyDrag>()
        && drag.world == world
        && drag.selection_generation == selection_generation
        && Some(drag.entity) != parent
    {
        ui.painter().rect_stroke(
            response.rect,
            2.,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    response
        .dnd_release_payload::<HierarchyDrag>()
        .filter(|drag| {
            drag.world == world
                && drag.selection_generation == selection_generation
                && Some(drag.entity) != parent
        })
        .map(|drag| Action::Reparent(drag.entity, parent))
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
                        editor.play.is_none() && !editor.is_busy(),
                        egui::Button::new("创建实体"),
                    )
                    .clicked()
                {
                    let result = editor.create_object(None, assets);
                    editor.notice(result);
                }
            });
            ui.label(
                editor
                    .project_root()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            )
            .on_hover_text(editor.project_root().display().to_string());
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
                    format!("运行预览 · 作者{}", editor.document_kind_label())
                } else {
                    format!("编辑{}", editor.document_kind_label())
                })
                .small()
                .weak(),
            );
            ui.add_space(10.);
            let snapshot = editor.last_snapshot.clone();
            // Build the child lookup once, rather than scanning the scene at every node.
            let mut children: BTreeMap<Option<EntityId>, Vec<&EntityData>> = BTreeMap::new();
            for entity in &snapshot.entities {
                children.entry(entity.parent).or_default().push(entity);
            }
            let authored: BTreeSet<_> = editor
                .document
                .shared_source()
                .base
                .entities
                .iter()
                .map(|entity| entity.id)
                .collect();
            let world = editor.document.world_id();
            let before = editor.selection.generation();
            let mut action = None;
            let can_edit = editor.play.is_none() && !editor.is_busy();
            if can_edit {
                let response = ui
                    .button(format!("{}根节点", editor.document_kind_label()))
                    .on_hover_text("将作者实体拖到这里，取消父级并保持世界变换。");
                action = drop_target(ui, &response, world, editor.selection.generation(), None);
                if response.clicked() {
                    editor.selection.clear();
                }
            }
            let mut visible = Vec::new();
            fn flatten(
                children: &BTreeMap<Option<EntityId>, Vec<&EntityData>>,
                parent: Option<EntityId>,
                ids: &mut Vec<EntityId>,
            ) {
                for entity in children.get(&parent).into_iter().flatten() {
                    ids.push(entity.id);
                    flatten(children, Some(entity.id), ids);
                }
            }
            flatten(&children, None, &mut visible);
            ui.small("Ctrl/Cmd 多选 · Shift 范围选择");
            let tree = Tree {
                children: &children,
                authored: &authored,
                world,
                can_edit,
                visible: &visible,
            };
            egui::ScrollArea::vertical().show(ui, |ui| {
                tree.show(ui, None, &mut editor.selection, &mut action);
            });
            if before != editor.selection.generation() {
                editor.picking.clear();
                editor.drag_origin = None;
                let result = editor.commit();
                editor.notice(result);
            }
            if let Some(action) = action {
                let result = match action {
                    Action::CreateChild(id) => editor.create_object(Some(id), assets),
                    Action::Duplicate(id) => editor.duplicate_object(id, assets),
                    Action::Delete(id) => editor.delete_object(id, assets),
                    Action::Reparent(id, parent) => editor.reparent_object(id, parent, assets),
                };
                editor.notice(result);
            }
        });
}

struct Tree<'a> {
    children: &'a BTreeMap<Option<EntityId>, Vec<&'a EntityData>>,
    authored: &'a BTreeSet<EntityId>,
    world: WorldId,
    can_edit: bool,
    visible: &'a [EntityId],
}
impl Tree<'_> {
    fn show(
        &self,
        ui: &mut egui::Ui,
        parent: Option<EntityId>,
        selected: &mut Selection,
        action: &mut Option<Action>,
    ) {
        for entity in self.children.get(&parent).into_iter().flatten() {
            ui.push_id(entity.id, |ui| {
                let authored = self.can_edit && self.authored.contains(&entity.id);
                let response = ui.add(
                    egui::Button::selectable(selected.contains(entity.id), &entity.name).sense(
                        if authored {
                            egui::Sense::click_and_drag()
                        } else {
                            egui::Sense::click()
                        },
                    ),
                );
                if response.clicked() {
                    let modifiers = super::click_modifiers(ui, egui::PointerButton::Primary);
                    if modifiers.shift {
                        selected.range(
                            self.visible,
                            entity.id,
                            modifiers.command || modifiers.ctrl,
                        );
                    } else {
                        selected.choose(
                            Some(entity.id),
                            if modifiers.command || modifiers.ctrl {
                                SelectMode::Toggle
                            } else {
                                SelectMode::Replace
                            },
                        );
                    }
                }
                if response.secondary_clicked() {
                    selected.activate(entity.id);
                }
                if authored {
                    if response.drag_started() {
                        selected.activate(entity.id);
                    }
                    response.dnd_set_drag_payload(HierarchyDrag {
                        world: self.world,
                        entity: entity.id,
                        selection_generation: selected.generation(),
                    });
                    if let Some(drop) = drop_target(
                        ui,
                        &response,
                        self.world,
                        selected.generation(),
                        Some(entity.id),
                    ) {
                        *action = Some(drop);
                    }
                }
                response.context_menu(|ui| {
                    ui.add_enabled_ui(self.can_edit, |ui| {
                        for (label, operation) in [
                            ("创建子实体", Action::CreateChild(entity.id)),
                            ("复制", Action::Duplicate(entity.id)),
                            ("删除", Action::Delete(entity.id)),
                        ] {
                            if ui.button(label).clicked() {
                                *action = Some(operation);
                                ui.close();
                            }
                        }
                        if authored
                            && entity.parent.is_some()
                            && ui.button("移到场景根节点").clicked()
                        {
                            *action = Some(Action::Reparent(entity.id, None));
                            ui.close();
                        }
                    });
                });
                ui.indent("children", |ui| {
                    self.show(ui, Some(entity.id), selected, action);
                });
            });
        }
    }
}
