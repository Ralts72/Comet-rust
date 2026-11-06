use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use comet_core::{AssetId, EntityId, WorldId};
use comet_editor_core::FieldTarget;
use comet_scene::{
    EntityData, Model,
    schema::{self, FieldValue},
};
use std::{collections::BTreeMap, sync::Arc};

struct Choices {
    revision: String,
    meshes: Vec<AssetId>,
    materials: Vec<AssetId>,
    labels: BTreeMap<AssetId, String>,
}

fn choices(ui: &egui::Ui, assets: &AssetStore) -> Arc<Choices> {
    let id = egui::Id::unique("inspector-model-choices");
    ui.data(|data| data.get_temp::<Arc<Choices>>(id))
        .filter(|choices| choices.revision == assets.revision())
        .unwrap_or_else(|| {
            let mut choices = Choices {
                revision: assets.revision().to_owned(),
                meshes: vec![],
                materials: vec![],
                labels: BTreeMap::new(),
            };
            for (&id, entry) in &assets.index().products {
                let (list, kind) = match entry.kind {
                    AssetKind::Mesh => (&mut choices.meshes, "网格"),
                    AssetKind::Material => (&mut choices.materials, "材质"),
                    _ => continue,
                };
                let label = match &entry.editor {
                    Some(editor) if editor.source == id => editor.label.clone(),
                    Some(editor) => {
                        let source = assets
                            .index()
                            .products
                            .get(&editor.source)
                            .and_then(|source| source.editor.as_ref())
                            .map_or("未命名来源", |source| source.label.as_str());
                        format!("{source} / {}", editor.label)
                    }
                    None => format!("未命名{kind} ({})", list.len() + 1),
                };
                list.push(id);
                choices.labels.insert(id, label);
            }
            for list in [&mut choices.meshes, &mut choices.materials] {
                list.sort_by(|a, b| choices.labels[a].cmp(&choices.labels[b]).then(a.cmp(b)));
            }
            let choices = Arc::new(choices);
            ui.data_mut(|data| data.insert_temp(id, choices.clone()));
            choices
        })
}

#[derive(Clone)]
struct Draft {
    world: WorldId,
    entity: EntityId,
    products: String,
    mesh: Option<AssetId>,
    material: Option<AssetId>,
}

fn draft_id() -> egui::Id {
    egui::Id::unique("inspector-model-draft")
}

pub(super) fn clear_draft(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Draft>(draft_id()));
}

fn picker(
    ui: &mut egui::Ui,
    editor: &Editor,
    entity: EntityId,
    label: &str,
    value: &mut Option<AssetId>,
    list: &[AssetId],
    choices: &Choices,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let selected = value.map_or_else(
            || format!("选择{label}"),
            |id| {
                choices
                    .labels
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("当前{label}已缺失"))
            },
        );
        egui::ComboBox::from_id_salt(("model-asset", editor.world(), entity, label))
            .width(ui.available_width().min(240.))
            .truncate()
            .selected_text(&selected)
            .show_ui(ui, |ui| {
                for &id in list {
                    ui.selectable_value(value, Some(id), &choices.labels[&id]);
                }
            })
            .response
            .on_hover_text(selected);
    });
}

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.separator();
    ui.strong("网格渲染");
    let target = if editor.play.is_none() {
        Some(super::overrides::target(editor, ui, assets, entity.id))
    } else {
        None
    };
    let authored = matches!(target, Some(Ok(FieldTarget::Authored(_))));
    let nested = matches!(&target, Some(Err(_)));
    let editable = target.as_ref().is_some_and(|target| target.is_ok())
        && editor.play.is_none()
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let Some(model) = &entity.model else {
        if !authored {
            clear_draft(ui.ctx());
            ui.weak(if editor.play.is_some() {
                "运行期间网格渲染配置只读。"
            } else {
                "Prefab 的网格渲染组件结构请在源 Prefab 中编辑。"
            });
            return;
        }
        let choices = choices(ui, assets);
        add(editor, ui, assets, entity.id, &choices, editable, cancel);
        return;
    };
    let choices = choices(ui, assets);
    clear_draft(ui.ctx());
    let mut mesh = Some(model.mesh);
    let mut material = Some(model.surface.material);
    let mut roughness = model.surface.roughness;
    let mut remove = false;
    let mut roughness_response = None;
    ui.add_enabled_ui(editable, |ui| {
        picker(
            ui,
            editor,
            entity.id,
            "网格",
            &mut mesh,
            &choices.meshes,
            &choices,
        );
        picker(
            ui,
            editor,
            entity.id,
            "材质",
            &mut material,
            &choices.materials,
            &choices,
        );
        roughness_response =
            Some(ui.add(egui::Slider::new(&mut roughness, 0. ..=1.).text("粗糙度")));
        if authored {
            remove = ui.small_button("移除网格渲染").clicked();
        }
    });
    if !editable {
        ui.weak(if editor.play.is_some() {
            "运行期间网格渲染配置只读。"
        } else if nested {
            "嵌套实例的网格渲染请在源 Prefab 中编辑。"
        } else {
            "当前网格渲染配置暂不可编辑。"
        });
        return;
    }
    if cancel {
        let result = editor.cancel();
        editor.notice(result);
        return;
    }
    if remove {
        replace(editor, assets, entity.id, None);
        return;
    }
    if mesh != Some(model.mesh) || material != Some(model.surface.material) {
        let mut next = model.clone();
        next.mesh = mesh.unwrap();
        next.surface.material = material.unwrap();
        replace(editor, assets, entity.id, Some(next));
        return;
    }
    let response = roughness_response.unwrap();
    if response.changed() {
        let result = editor.edit(
            entity.id,
            schema::ROUGHNESS,
            FieldValue::Scalar(roughness),
            assets,
        );
        editor.notice(result);
    }
    if response.drag_stopped() || response.lost_focus() {
        let result = editor.commit();
        editor.notice(result);
    }
}

fn add(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: EntityId,
    choices: &Choices,
    editable: bool,
    cancel: bool,
) {
    let mut draft = ui
        .data(|data| data.get_temp::<Draft>(draft_id()))
        .filter(|draft| {
            draft.world == editor.document.world_id()
                && draft.entity == entity
                && draft.products == assets.revision()
        })
        .unwrap_or_else(|| Draft {
            world: editor.document.world_id(),
            entity,
            products: assets.revision().to_owned(),
            mesh: None,
            material: None,
        });
    let mut add = false;
    ui.add_enabled_ui(editable, |ui| {
        picker(
            ui,
            editor,
            entity,
            "网格",
            &mut draft.mesh,
            &choices.meshes,
            choices,
        );
        picker(
            ui,
            editor,
            entity,
            "材质",
            &mut draft.material,
            &choices.materials,
            choices,
        );
        add = ui
            .add_enabled(
                draft.mesh.is_some() && draft.material.is_some(),
                egui::Button::new("添加网格渲染"),
            )
            .clicked();
    });
    if choices.meshes.is_empty() || choices.materials.is_empty() {
        ui.weak("导入含网格与材质的模型，或添加 .mesh/.material 文件并刷新资源。");
    }
    if cancel {
        draft.mesh = None;
        draft.material = None;
    }
    if add && !cancel {
        let result = editor
            .add_model(entity, draft.mesh.unwrap(), draft.material.unwrap(), assets)
            .and_then(|_| editor.commit());
        if result.is_ok() {
            ui.data_mut(|data| data.remove::<Draft>(draft_id()));
        } else {
            ui.data_mut(|data| data.insert_temp(draft_id(), draft));
        }
        editor.notice(result);
    } else {
        ui.data_mut(|data| data.insert_temp(draft_id(), draft));
    }
}

fn replace(editor: &mut Editor, assets: &mut AssetStore, entity: EntityId, model: Option<Model>) {
    let result = editor
        .edit_model(entity, model, assets)
        .and_then(|_| editor.commit());
    editor.notice(result);
}

#[cfg(test)]
mod tests;
