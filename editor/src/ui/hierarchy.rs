use crate::{
    selection::{SelectMode, Selection},
    session::Editor,
};
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::SourceCatalog;
use comet_core::{EntityId, WorldId};
use comet_scene::{ContentSource, EntityData};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
enum Action {
    CreateChild(EntityId),
    Duplicate(EntityId),
    PlaceCopies(EntityId),
    SavePrefab(EntityId),
    Delete(EntityId),
    Detach(EntityId),
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

pub(super) fn draw(editor: &mut Editor, ui: &mut egui::Ui, assets: &mut AssetStore) {
    ui.horizontal(|ui| {
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
    let source = editor.document.shared_source();
    let tree = Tree {
        source: &source,
        sources: &editor.sources,
        children: &children,
        authored: &authored,
        world,
        can_edit,
        visible: &visible,
    };
    egui::ScrollArea::vertical().show(ui, |ui| {
        tree.show(ui, None, &mut editor.selection, &mut action, assets);
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
            Action::PlaceCopies(entity) => {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        placement_id(),
                        Placement {
                            entity,
                            world: editor.document.world_id(),
                            revision: editor.document.revision(),
                            selection: editor.selection.generation(),
                            products: assets.revision().into(),
                            copies: 3,
                            step: [2., 0., 0.],
                            focus: true,
                            error: None,
                        },
                    )
                });
                Ok(())
            }
            Action::Delete(id) => editor.delete_object(id, assets),
            Action::Detach(id) => editor.detach_resource(id, assets),
            Action::SavePrefab(id) => editor.begin_save_prefab(id, assets),
            Action::Reparent(id, parent) => editor.reparent_object(id, parent, assets),
        };
        editor.notice(result);
    }
}

struct Tree<'a> {
    source: &'a ContentSource,
    sources: &'a SourceCatalog,
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
        assets: &mut AssetStore,
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
                        if ui
                            .add_enabled(
                                self.authored.contains(&entity.id),
                                egui::Button::new("批量放置..."),
                            )
                            .on_hover_text("按世界坐标间距复制作者子树及挂接实例。")
                            .on_disabled_hover_text(
                                "请选择作者实体或资源的作者容器；直接 Prefab 根请先解除关联。",
                            )
                            .clicked()
                        {
                            *action = Some(Action::PlaceCopies(entity.id));
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.authored.contains(&entity.id),
                                egui::Button::new("保存为 Prefab..."),
                            )
                            .on_hover_text("保存作者子树为独立 Prefab，保留子实例关联。")
                            .on_disabled_hover_text(
                                "请选择作者实体或资源的作者容器；直接实例根请先解除关联。",
                            )
                            .clicked()
                        {
                            *action = Some(Action::SavePrefab(entity.id));
                            ui.close();
                        }
                        if !self.authored.contains(&entity.id) {
                            match crate::objects::resource_at_root(self.source, entity.id, assets) {
                                Ok(Some(prefab)) => {
                                    let model = self
                                        .sources
                                        .of_kind(AssetKind::Prefab)
                                        .find(|source| source.id == prefab)
                                        .and_then(|source| source.source.extension())
                                        .and_then(|extension| extension.to_str())
                                        .is_some_and(|extension| {
                                            extension.eq_ignore_ascii_case("gltf")
                                                || extension.eq_ignore_ascii_case("glb")
                                        });
                                    let label = if model {
                                        "解除模型关联"
                                    } else {
                                        "解除 Prefab 关联"
                                    };
                                    if ui
                                        .button(label)
                                        .on_hover_text("解除当前这一层关联，嵌套实例继续保留关联。")
                                        .clicked()
                                    {
                                        *action = Some(Action::Detach(entity.id));
                                        ui.close();
                                    }
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    ui.weak("无法读取资源关联").on_hover_text(error.to_string());
                                }
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
                    self.show(ui, Some(entity.id), selected, action, assets);
                });
            });
        }
    }
}

fn placement_id() -> egui::Id {
    egui::Id::unique("hierarchy-placement")
}

pub(crate) fn cancel_placement(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Placement>(placement_id()));
}

#[derive(Clone)]
struct Placement {
    entity: EntityId,
    world: WorldId,
    revision: u64,
    selection: u64,
    products: String,
    copies: u32,
    step: [f32; 3],
    focus: bool,
    error: Option<String>,
}

/// The draft belongs to this UI context, never to the author document or its history.
pub(super) fn placement_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    assets: &mut AssetStore,
    active: bool,
) -> bool {
    let Some(mut draft) = ctx.data_mut(|data| {
        let draft = data.get_temp::<Placement>(placement_id());
        data.remove::<Placement>(placement_id());
        draft
    }) else {
        return false;
    };
    if !active
        || editor.play.is_some()
        || editor.is_busy()
        || editor.documents.is_blocking()
        || draft.world != editor.document.world_id()
        || draft.revision != editor.document.revision()
        || draft.selection != editor.selection.generation()
        || draft.products != assets.revision()
    {
        return true;
    }
    let mut open = !ctx.input(|input| input.key_pressed(egui::Key::Escape));
    let mut close = false;
    if open {
        egui::Window::new("批量放置")
            .id(placement_id())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("按世界坐标间距复制选中组，保留组内相对位置与引用。");
                ui.weak("切换选择或编辑内容会取消本次放置。");
                ui.horizontal(|ui| {
                    ui.label("新增份数");
                    let response = ui.add(
                        egui::DragValue::new(&mut draft.copies)
                            .range(1..=comet_editor_core::MAX_PLACEMENT_COPIES),
                    );
                    if draft.focus {
                        response.request_focus();
                        draft.focus = false;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("每份间距 XYZ");
                    for (axis, value) in ["X", "Y", "Z"].into_iter().zip(&mut draft.step) {
                        ui.add(
                            egui::DragValue::new(value)
                                .speed(0.1)
                                .prefix(format!("{axis} ")),
                        );
                    }
                });
                let valid = (1..=comet_editor_core::MAX_PLACEMENT_COPIES).contains(&draft.copies)
                    && draft.step.iter().all(|value| value.is_finite());
                if !valid {
                    ui.weak(format!(
                        "份数须在 1 到 {} 之间，间距须是有限数值。",
                        comet_editor_core::MAX_PLACEMENT_COPIES
                    ));
                }
                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::from_rgb(244, 141, 131), error);
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(valid, egui::Button::new("放置")).clicked() {
                        match editor.place_copies(draft.entity, draft.copies, draft.step, assets) {
                            Ok(()) => close = true,
                            Err(error) => draft.error = Some(format!("{error:#}")),
                        }
                    }
                    if ui.button("取消").clicked() {
                        close = true;
                    }
                });
            });
    }
    if open && !close {
        ctx.data_mut(|data| data.insert_temp(placement_id(), draft));
    }
    true
}
