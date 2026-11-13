use super::assets::{Catalog, catalog, pick_draft};
use crate::{components::ComponentResources, resources::ResourceVersion, session::Editor};
use comet_asset::AssetStore;
use comet_editor_core::ComponentKind;
use comet_scene::EntityData;

#[derive(Clone)]
struct Draft {
    version: ResourceVersion,
    selection: u64,
    kind: ComponentKind,
    resources: ComponentResources,
}
fn name(kind: ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Model => "网格渲染",
        ComponentKind::Physics => "刚体",
        ComponentKind::Character => "角色控制器",
        ComponentKind::Script => "脚本",
        ComponentKind::PointJoint => "点关节",
        ComponentKind::HingeJoint => "铰链关节",
        ComponentKind::SliderJoint => "滑动关节",
        ComponentKind::NavigationRegion => "导航区域",
        ComponentKind::NavigationAgent => "导航代理",
        ComponentKind::Audio => "音频源",
        ComponentKind::Animation => "动画",
        ComponentKind::Skin => "蒙皮绑定",
        ComponentKind::CharacterAnimation => "角色动画驱动",
    }
}
fn resource(
    kind: ComponentKind,
    choices: &Catalog,
) -> Option<(&'static str, &[comet_core::AssetId])> {
    Some(match kind {
        ComponentKind::Model => ("网格", &choices.meshes),
        ComponentKind::Script => ("脚本", &choices.scripts),
        ComponentKind::Audio => ("音频片段", &choices.audio),
        ComponentKind::Animation => ("骨架", &choices.skeletons),
        ComponentKind::Skin => ("蒙皮资源", &choices.skins),
        _ => return None,
    })
}

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
) {
    if entities.is_empty() {
        return;
    }
    let source = editor.document.shared_source();
    let authored = entities.iter().all(|entity| {
        source
            .base
            .entities
            .binary_search_by_key(&entity.id, |e| e.id)
            .is_ok()
    });
    let editable = authored
        && editor.play.is_none()
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    let ids: Vec<_> = entities.iter().map(|e| e.id).collect();
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let key = egui::Id::unique("component-add-draft");
    let mut draft = ui.data(|data| data.get_temp::<Draft>(key)).filter(|draft| {
        editable
            && !cancel
            && draft.selection == editor.selection.generation()
            && draft.version.matches(&editor.document, assets)
    });
    let mut request = None;
    ui.separator();
    ui.push_id(
        (
            "component-structure",
            editor.world(),
            editor.selection.generation(),
        ),
        |ui| {
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal(|ui| {
                    ui.menu_button("添加组件", |ui| {
                        ui.weak("只添加缺失组件，保留已有配置。");
                        let choices = catalog(ui, assets);
                        for &kind in ComponentKind::ALL {
                            if entities.iter().all(|e| kind.is_present(e)) {
                                continue;
                            }
                            if ui.button(name(kind)).clicked() && !cancel {
                                if resource(kind, &choices).is_some() {
                                    draft = Some(Draft {
                                        version: ResourceVersion::capture(&editor.document, assets),
                                        selection: editor.selection.generation(),
                                        kind,
                                        resources: ComponentResources::default(),
                                    });
                                } else {
                                    request = Some((kind, Some(ComponentResources::default())));
                                }
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("移除组件", |ui| {
                        ui.weak("同时清理依赖组件与专用引用，可撤销。");
                        for &kind in ComponentKind::ALL {
                            if entities.iter().any(|e| kind.is_present(e))
                                && ui.button(name(kind)).on_hover_text(match kind {
                                    ComponentKind::Physics => "同时移除角色、导航代理和自身关节，其他关节的连接改为未绑定。",
                                    ComponentKind::Character => "保留刚体，移除导航代理并解除角色动画驱动的角色绑定。",
                                    ComponentKind::Animation => "蒙皮和角色动画驱动的动画实体绑定会解除。",
                                    ComponentKind::Model => "同时移除依赖该模型的蒙皮绑定。",
                                    _ => "只移除所选组件，保留实体及其通用引用。",
                                }).clicked()
                                && !cancel
                            {
                                request = Some((kind, None));
                                ui.close();
                            }
                        }
                    });
                });
            })
            .response
            .on_disabled_hover_text(
                "组件结构只允许在停止运行且没有模态操作时修改作者实体；Prefab 请编辑源文件。",
            );
        },
    );
    if let Some(value) = &mut draft {
        let choices = catalog(ui, assets);
        let mut open = true;
        let mut dismiss = false;
        egui::Window::new(format!("添加{}", name(value.kind)))
            .id(egui::Id::unique((
                "component-resources",
                editor.world(),
                editor.selection.generation(),
                value.kind,
            )))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label(format!("目标：{} 个作者实体；保留已有组件。", ids.len()));
                if let Some((label, items)) = resource(value.kind, &choices) {
                    pick_draft(
                        ui,
                        (value.kind, "asset"),
                        label,
                        &mut value.resources.asset,
                        items.iter().copied(),
                        &choices,
                        Some("请选择资源"),
                    );
                }
                if value.kind == ComponentKind::Model {
                    pick_draft(
                        ui,
                        (value.kind, "material"),
                        "材质",
                        &mut value.resources.material,
                        choices.materials.iter().copied(),
                        &choices,
                        Some("请选择材质"),
                    );
                }
                if matches!(value.kind, ComponentKind::Animation | ComponentKind::Skin) {
                    ui.weak("新增后可在单选面板中配置实体绑定。");
                }
                let ready = value.resources.asset.is_some()
                    && (value.kind != ComponentKind::Model || value.resources.material.is_some());
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(ready, egui::Button::new("添加到所选实体"))
                        .clicked()
                    {
                        request = Some((value.kind, Some(value.resources)));
                    }
                    dismiss = ui.button("取消").clicked();
                });
            });
        if !open || dismiss {
            draft = None;
        }
    }
    if editable && let Some((kind, resources)) = request {
        let result = editor
            .commit()
            .and_then(|_| match resources {
                Some(resources) => editor.add_components(&ids, kind, resources, assets),
                None => editor.remove_components(&ids, kind, assets),
            })
            .and_then(|_| editor.commit());
        if result.is_ok() {
            draft = None;
        }
        editor.notice(result);
    }
    ui.data_mut(|data| {
        if let Some(draft) = draft {
            data.insert_temp(key, draft);
        } else {
            data.remove::<Draft>(key);
        }
    });
}
