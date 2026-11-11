use super::properties::ComponentEdit;
use crate::{
    navigation::{AGENT_FIELDS, REGION_FIELDS},
    session::Editor,
};
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use comet_core::{EntityId, WorldId};
use comet_editor_core::FieldTarget;
use comet_scene::{BodyMotion, EntityData, EntityRef, NavigationAgent, NavigationRegion};

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.separator();
    ui.strong("导航");
    let target = if editor.play.is_none() {
        Some(super::overrides::target(editor, ui, assets, entity.id))
    } else {
        None
    };
    let authored = matches!(target, Some(Ok(FieldTarget::Authored(_))));
    let editable = target.as_ref().is_some_and(|target| target.is_ok())
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    if let Some(Err(error)) = &target {
        ui.weak("嵌套实例的导航配置请在源 Prefab 中编辑。")
            .on_hover_text(error);
    }
    if editor.play.is_some() {
        ui.weak("运行期间导航配置只读。");
    }
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));

    if let Some(region) = &entity.navigation_region {
        egui::CollapsingHeader::new("导航区域")
            .default_open(true)
            .show(ui, |ui| {
                let mut next = entity.clone();
                let value = next.navigation_region.as_mut().unwrap();
                let mut edit = ComponentEdit::default();
                let mut remove = false;
                ui.add_enabled_ui(editable, |ui| {
                    if authored {
                        remove = ui.small_button("移除导航区域").clicked();
                    }
                    let before = value.data;
                    egui::ComboBox::from_id_salt(("navigation-data", entity.id))
                        .width(ui.available_width().min(240.))
                        .truncate()
                        .selected_text(
                            value
                                .data
                                .and_then(|asset| {
                                    editor
                                        .sources
                                        .of_kind(AssetKind::Navigation)
                                        .find(|source| source.id == asset)
                                })
                                .map_or_else(
                                    || {
                                        if value.data.is_some() {
                                            "当前导航数据已缺失".into()
                                        } else {
                                            "未绑定导航数据".into()
                                        }
                                    },
                                    |source| source.source.display().to_string(),
                                ),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut value.data, None, "未绑定导航数据");
                            for source in editor.sources.of_kind(AssetKind::Navigation) {
                                ui.selectable_value(
                                    &mut value.data,
                                    Some(source.id),
                                    source.source.display().to_string(),
                                );
                            }
                        });
                    edit.finish |= value.data != before;
                    ui.label("区域半尺寸（米）");
                    ui.horizontal(|ui| {
                        for (label, value) in
                            ["X", "Y", "Z"].into_iter().zip(&mut value.half_extents)
                        {
                            edit.number(ui, label, value, 0.001, f32::MAX);
                        }
                    });
                    edit.number(ui, "网格尺寸", &mut value.cell_size, 0.001, f32::MAX);
                    edit.number(ui, "代理半径", &mut value.agent_radius, 0.001, f32::MAX);
                    edit.number(ui, "代理高度", &mut value.agent_height, 0.001, f32::MAX);
                    edit.number(ui, "最大攀爬", &mut value.max_climb, 0., f32::MAX);
                    edit.number(ui, "最大坡度 (°)", &mut value.max_slope_degrees, 0., 89.9);
                });
                if editable {
                    if cancel {
                        let result = editor.cancel();
                        editor.notice(result);
                    } else if remove {
                        replace(
                            editor,
                            assets,
                            entity,
                            None,
                            entity.navigation_agent.clone(),
                        );
                    } else {
                        update(editor, assets, entity, &next, authored, edit.finish);
                    }
                }
                bake(editor, ui, assets, entity.id, region, editable);
            });
    } else {
        let existing = if editor.play.is_none() {
            editor
                .navigation_snapshot(assets)
                .ok()
                .and_then(|snapshot| {
                    editor
                        .navigation
                        .region(&editor.document, &snapshot, assets)
                })
        } else {
            None
        };
        let eligible = entity.parent.is_none()
            && entity.character.is_none()
            && entity
                .physics
                .as_ref()
                .is_none_or(|body| body.motion == BodyMotion::Fixed)
            && entity.transform.scale == [1.; 3]
            && entity.transform.rotation[0..3] == [0.; 3]
            && entity.transform.rotation[3].abs() == 1.;
        if ui
            .add_enabled(
                authored && editable && existing.is_none() && eligible,
                egui::Button::new("添加导航区域"),
            )
            .on_disabled_hover_text("场景只能有一个导航区域；需要单位旋转、单位缩放的静止根实体。")
            .clicked()
        {
            replace(
                editor,
                assets,
                entity,
                Some(NavigationRegion::default()),
                entity.navigation_agent.clone(),
            );
        }
    }

    if entity.navigation_agent.is_some() {
        egui::CollapsingHeader::new("导航代理")
            .default_open(true)
            .show(ui, |ui| {
                let mut next = entity.clone();
                let value = next.navigation_agent.as_mut().unwrap();
                let choices = if editable {
                    editor
                        .cached_field_references(ui.ctx(), entity.id, assets)
                        .ok()
                } else {
                    None
                };
                let mut edit = ComponentEdit::default();
                let mut remove = false;
                ui.add_enabled_ui(editable, |ui| {
                    if authored {
                        remove = ui.small_button("移除导航代理").clicked();
                    }
                    edit.finish |= ui.checkbox(&mut value.enabled, "启用导航代理").changed();
                    let before = value.target.clone();
                    let name = match &value.target {
                        Some(EntityRef::Local { entity }) => choices
                            .as_ref()
                            .and_then(|choices| choices.targets.iter().find(|(id, _)| id == entity))
                            .map(|(_, name)| name.as_str())
                            .or_else(|| {
                                editor
                                    .last_snapshot
                                    .entities
                                    .iter()
                                    .find(|node| node.id == *entity)
                                    .map(|node| node.name.as_str())
                            })
                            .unwrap_or("目标已缺失"),
                        Some(EntityRef::External { .. }) => "其他场景目标",
                        Some(EntityRef::InInstance { .. }) => "实例目标",
                        None => "无目标",
                    };
                    egui::ComboBox::from_id_salt(("navigation-target", entity.id))
                        .width(ui.available_width().min(240.))
                        .truncate()
                        .selected_text(name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut value.target, None, "无目标");
                            for (id, name) in choices
                                .iter()
                                .flat_map(|choices| &choices.targets)
                                .filter(|(id, _)| *id != entity.id)
                            {
                                ui.selectable_value(
                                    &mut value.target,
                                    Some(EntityRef::local(*id)),
                                    name,
                                );
                            }
                        });
                    edit.finish |= value.target != before;
                    edit.number(ui, "到达距离", &mut value.arrival_distance, 0., f32::MAX);
                });
                if editable {
                    if cancel {
                        let result = editor.cancel();
                        editor.notice(result);
                    } else if remove {
                        replace(
                            editor,
                            assets,
                            entity,
                            entity.navigation_region.clone(),
                            None,
                        );
                    } else {
                        update(editor, assets, entity, &next, authored, edit.finish);
                    }
                }
            });
    } else if ui
        .add_enabled(
            authored
                && editable
                && entity
                    .character
                    .as_ref()
                    .is_some_and(|character| !character.player_controlled),
            egui::Button::new("添加导航代理"),
        )
        .on_disabled_hover_text("导航代理需要已关闭玩家控制的胶囊角色。")
        .clicked()
    {
        replace(
            editor,
            assets,
            entity,
            entity.navigation_region.clone(),
            Some(NavigationAgent::default()),
        );
    }
}

fn update(
    editor: &mut Editor,
    assets: &mut AssetStore,
    before: &EntityData,
    after: &EntityData,
    authored: bool,
    finish: bool,
) {
    let changed = REGION_FIELDS.iter().chain(AGENT_FIELDS).any(|field| {
        let field = comet_scene::schema::descriptor(*field).unwrap();
        field.read(before) != field.read(after)
    });
    if changed {
        let result = if authored {
            editor.edit_navigation(
                before.id,
                after.navigation_region.clone(),
                after.navigation_agent.clone(),
                assets,
            )
        } else {
            editor.edit_navigation_fields(before, after, assets)
        };
        editor.notice(result);
    }
    if finish {
        let result = editor.commit();
        editor.notice(result);
    }
}
fn replace(
    editor: &mut Editor,
    assets: &mut AssetStore,
    entity: &EntityData,
    region: Option<NavigationRegion>,
    agent: Option<NavigationAgent>,
) {
    let result = editor
        .commit()
        .and_then(|_| editor.edit_navigation(entity.id, region, agent, assets))
        .and_then(|_| editor.commit());
    editor.notice(result);
}

#[derive(Clone)]
struct OutputPath {
    world: WorldId,
    region: EntityId,
    path: String,
}
fn bake(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    region: EntityId,
    value: &NavigationRegion,
    editable: bool,
) {
    if editor.play.is_some() {
        return;
    }
    ui.separator();
    let snapshot = match editor.navigation_snapshot(assets) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            ui.weak("导航输入暂不可用。")
                .on_hover_text(format!("{error:#}"));
            return;
        }
    };
    let input = editor.navigation.input(&editor.document, &snapshot, assets);
    match editor
        .navigation
        .status(&editor.document, &snapshot, assets)
    {
        Ok(()) => {
            ui.weak("导航数据与当前几何一致。");
        }
        Err(error) => {
            let message = if input.is_err() {
                "导航输入不可用，请检查区域和静态长方体地面。"
            } else if error.contains("陈旧") {
                "导航数据已陈旧，请重新烘焙。"
            } else if value.data.is_none() {
                "尚未烘焙导航数据，请先烘焙。"
            } else {
                "导航数据不可用，请重新烘焙。"
            };
            ui.weak(message).on_hover_text(error);
        }
    }
    let id = egui::Id::unique("navigation-output-path");
    let mut draft = ui
        .data(|data| data.get_temp::<OutputPath>(id))
        .filter(|draft| draft.world == editor.document.world_id() && draft.region == region)
        .unwrap_or_else(|| OutputPath {
            world: editor.document.world_id(),
            region,
            path: value
                .data
                .and_then(|id| {
                    editor
                        .sources
                        .of_kind(AssetKind::Navigation)
                        .find(|source| source.id == id)
                })
                .map_or_else(
                    || format!("navigation-{region}.nav"),
                    |source| source.source.display().to_string(),
                ),
        });
    ui.label("导航文件（相对 assets，使用 .nav 后缀）");
    ui.add_enabled(
        editable,
        egui::TextEdit::singleline(&mut draft.path).desired_width(f32::INFINITY),
    );
    if ui
        .add_enabled(
            editable && input.is_ok() && !draft.path.trim().is_empty(),
            egui::Button::new("烘焙导航"),
        )
        .on_disabled_hover_text("需要可用的导航区域和静态长方体地面；运行或后台任务期间不能烘焙。")
        .clicked()
    {
        let result = editor.begin_navigation_bake(std::path::Path::new(draft.path.trim()), assets);
        editor.notice(result);
    }
    ui.data_mut(|data| data.insert_temp(id, draft));
}

#[cfg(test)]
mod tests;
