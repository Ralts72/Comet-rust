use crate::resources::ResourceVersion;
use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_asset_data::product::ProductIndex;
use comet_core::{AssetId, EntityId, WorldId};
use comet_editor_core::FieldTarget;
use comet_scene::{Animator, EntityData, EntityRef, SkinBinding, schema};
use std::{collections::BTreeMap, sync::Arc};

use super::assets::{catalog as choices, pick_draft as asset_picker};

#[derive(Clone)]
struct Targets {
    version: ResourceVersion,
    owner: EntityId,
    nodes: Arc<Vec<(EntityId, String)>>,
    animators: Arc<Vec<(EntityId, String, AssetId)>>,
}
fn targets(
    editor: &mut Editor,
    ui: &egui::Ui,
    assets: &mut AssetStore,
    owner: &EntityData,
) -> Targets {
    let key = egui::Id::unique("animation-entity-targets");
    if let Some(value) = ui
        .data(|data| data.get_temp::<Targets>(key))
        .filter(|value| value.version.matches(&editor.document, assets) && value.owner == owner.id)
    {
        return value;
    }
    let nodes = editor
        .cached_field_references(ui.ctx(), owner.id, assets)
        .map(|references| references.targets.clone())
        .unwrap_or_default();
    let by_id: BTreeMap<_, _> = editor
        .last_snapshot
        .entities
        .iter()
        .map(|entity| (entity.id, entity))
        .collect();
    let animators = nodes
        .iter()
        .filter_map(|(id, name)| {
            by_id
                .get(id)
                .and_then(|entity| entity.animator.as_ref())
                .map(|animator| (*id, name.clone(), animator.skeleton))
        })
        .collect();
    let value = Targets {
        version: ResourceVersion::capture(&editor.document, assets),
        owner: owner.id,
        nodes: Arc::new(nodes),
        animators: Arc::new(animators),
    };
    ui.data_mut(|data| data.insert_temp(key, value.clone()));
    value
}

#[derive(Clone)]
struct Nodes {
    index: Arc<ProductIndex>,
    skeleton: AssetId,
    values: Arc<Vec<(EntityId, String)>>,
}
fn skeleton_nodes(
    ui: &egui::Ui,
    assets: &mut AssetStore,
    skeleton: AssetId,
) -> anyhow::Result<Arc<Vec<(EntityId, String)>>> {
    let key = egui::Id::unique("animation-skeleton-nodes");
    if let Some(value) = ui.data(|data| data.get_temp::<Nodes>(key)).filter(|value| {
        Arc::ptr_eq(&value.index, &assets.shared_index()) && value.skeleton == skeleton
    }) {
        return Ok(value.values);
    }
    let product = assets.load(skeleton)?;
    let Product::Skeleton(skeleton_data) = &product.payload else {
        anyhow::bail!("当前资源不是骨架")
    };
    let values: Arc<Vec<(EntityId, String)>> = Arc::new(
        skeleton_data
            .nodes
            .iter()
            .map(|node| (node.id, node.name.clone()))
            .collect(),
    );
    ui.data_mut(|data| {
        data.insert_temp(
            key,
            Nodes {
                index: assets.shared_index(),
                skeleton,
                values: values.clone(),
            },
        )
    });
    Ok(values)
}

#[derive(Clone, Default)]
struct Draft {
    world: Option<WorldId>,
    owner: Option<EntityId>,
    skeleton: Option<AssetId>,
    skin: Option<AssetId>,
}

fn entity_picker<'a>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut Option<EntityRef>,
    list: impl Iterator<Item = (EntityId, &'a str)> + Clone,
) {
    let current = match value {
        Some(EntityRef::Local { entity }) => Some(*entity),
        _ => None,
    };
    let mut selected = current;
    let name = list
        .clone()
        .find(|(id, _)| Some(*id) == current)
        .map(|(_, name)| name)
        .unwrap_or(if value.is_some() {
            "当前绑定（只读或不兼容）"
        } else {
            "未绑定"
        });
    egui::ComboBox::from_id_salt(salt)
        .width(ui.available_width().min(230.))
        .truncate()
        .selected_text(name)
        .show_ui(ui, |ui| {
            if ui.selectable_label(value.is_none(), "未绑定").clicked() {
                *value = None;
                selected = None;
            }
            for (id, name) in list {
                ui.selectable_value(&mut selected, Some(id), name);
            }
        });
    if selected != current {
        *value = selected.map(EntityRef::local);
    }
}

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.separator();
    ui.strong("动画");
    let target = if editor.play.is_none() {
        Some(super::overrides::target(editor, ui, assets, entity.id))
    } else {
        None
    };
    let authored = matches!(target, Some(Ok(FieldTarget::Authored(_))));
    let editable = target.as_ref().is_some_and(|target| target.is_ok())
        && editor.play.is_none()
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let choices = choices(ui, assets);
    let targets = if editable {
        Some(targets(editor, ui, assets, entity))
    } else {
        None
    };
    let key = egui::Id::unique("animation-component-draft");
    let mut draft = ui
        .data(|data| data.get_temp::<Draft>(key))
        .filter(|draft| draft.world == Some(editor.world()) && draft.owner == Some(entity.id))
        .unwrap_or(Draft {
            world: Some(editor.world()),
            owner: Some(entity.id),
            ..Default::default()
        });
    if let Some(animator) = &entity.animator {
        if let Some(play) = &editor.play {
            if let Some(state) = play.animation_state(entity.id) {
                if let Some(name) = &state.state {
                    ui.label(format!("当前状态：{name}，过渡 {:.0}%", state.blend * 100.));
                }
                ui.label(format!(
                    "片段时间：{:.3} / {:.3} 秒",
                    state.time, state.duration
                ));
                let status = if animator.clip.is_none() {
                    "未选择片段"
                } else if state.playing {
                    "播放中"
                } else if !animator.looping && state.time >= state.duration {
                    "已结束"
                } else {
                    "已暂停"
                };
                ui.label(format!("片段状态：{status}"));
                ui.label(format!("实际速度：{:.2}×", state.speed));
            } else if animator.clip.is_none() {
                ui.label("片段状态：未选择片段");
            }
            if play.is_paused() {
                ui.weak("运行世界：已暂停");
            }
            if play.is_suspended() {
                ui.weak("运行世界：宿主挂起");
            }
        }
        let revision = editor.document.revision();
        ui.add_enabled_ui(editable, |ui| {
            super::assets::draw_common(
                editor,
                ui,
                assets,
                &[entity],
                &[
                    schema::ANIMATOR_SKELETON,
                    schema::ANIMATOR_CONTROLLER,
                    schema::ANIMATOR_CLIP,
                ],
            );
        });
        if editor.document.revision() != revision {
            return;
        }
        let (mut looping, mut speed) = (animator.looping, animator.speed);
        let mut discrete = false;
        let mut finish = false;
        let mut remove = false;
        let mut binding_edits = vec![];
        let mut rebuild = false;
        ui.add_enabled_ui(editable, |ui| {
            ui.add_enabled_ui(animator.controller.is_none(), |ui| {
                discrete |= ui.checkbox(&mut looping, "循环").changed();
            });
            let response = ui.add(
                egui::DragValue::new(&mut speed)
                    .range(0. ..=f32::MAX)
                    .speed(0.01)
                    .prefix("播放速度 "),
            );
            finish |= response.drag_stopped() || response.lost_focus();
            egui::CollapsingHeader::new("骨骼绑定")
                .id_salt(("animator-bindings", editor.world(), entity.id))
                .show(ui, |ui| {
                    if let Some(targets) = &targets {
                        match skeleton_nodes(ui, assets, animator.skeleton) {
                        Ok(nodes) => {
                            let incomplete = animator.bindings.len() != nodes.len() || nodes.iter().any(|(id, _)| !animator.bindings.contains_key(id));
                            if incomplete {
                                ui.weak("绑定表与当前骨架不一致；重建会保留已有节点的绑定，并移除过期声明。");
                                if ui.small_button("重建绑定表").clicked() { rebuild = true; }
                            }
                                for (source, name) in nodes.iter() {
                                    ui.label(name);
                                    let before = animator.bindings.get(source).cloned().flatten();
                                    let mut next = before.clone();
                                    entity_picker(
                                        ui,
                                        ("animation-bone", editor.world(), entity.id, source),
                                        &mut next,
                                        targets.nodes.iter().map(|(id, name)| (*id, name.as_str())),
                                    );
                                    if before != next {
                                        binding_edits.push((*source, next));
                                    }
                                }
                            }
                            Err(error) => {
                                ui.colored_label(
                                    egui::Color32::LIGHT_RED,
                                    format!("骨架读取失败：{error:#}"),
                                );
                            }
                        }
                    } else {
                        ui.weak("骨骼绑定配置当前只读。");
                    }
                });
            if authored {
                remove = ui.small_button("移除动画").clicked();
            }
        });
        if editable && !cancel {
            if remove {
                replace(editor, assets, entity, None, entity.skin.clone());
                return;
            }
            if rebuild {
                rebuild_bindings(editor, assets, &[entity.id]);
                return;
            }
            let changed =
                looping != animator.looping || speed != animator.speed || !binding_edits.is_empty();
            if changed {
                discrete |= !binding_edits.is_empty();
                let mut next = animator.clone();
                next.looping = looping;
                next.speed = speed;
                for (source, target) in binding_edits {
                    next.bindings.insert(source, target);
                }
                let result =
                    editor.edit_animation(entity.id, Some(next), entity.skin.clone(), assets);
                editor.notice(result);
            }
            if discrete || finish {
                let result = editor.commit();
                editor.notice(result);
            }
        }
    } else if authored {
        ui.add_enabled_ui(editable, |ui| {
            asset_picker(
                ui,
                ("new-animator-skeleton", editor.world(), entity.id),
                "骨架",
                &mut draft.skeleton,
                choices.skeletons.iter().copied(),
                &choices,
                Some("请选择骨架"),
            );
            if ui
                .add_enabled(draft.skeleton.is_some(), egui::Button::new("添加动画"))
                .clicked()
                && !cancel
            {
                replace(
                    editor,
                    assets,
                    entity,
                    draft.skeleton.map(Animator::new),
                    entity.skin.clone(),
                );
            }
        });
    }
    ui.strong("蒙皮");
    if entity.skin.is_some() {
        ui.weak("整体位置由骨骼层级决定；整体移动请选择实例根节点。");
    }
    if let Some(skin) = &entity.skin {
        let revision = editor.document.revision();
        ui.add_enabled_ui(editable, |ui| {
            super::assets::draw_common(editor, ui, assets, &[entity], &[schema::SKIN_ASSET]);
        });
        if editor.document.revision() != revision {
            return;
        }
        let mut animator = skin.animator.clone();
        let mut remove = false;
        ui.add_enabled_ui(editable, |ui| {
            ui.label("动画实体");
            entity_picker(
                ui,
                ("skin-animator", editor.world(), entity.id),
                &mut animator,
                targets
                    .as_ref()
                    .into_iter()
                    .flat_map(|targets| targets.animators.iter())
                    .filter(|(_, _, skeleton)| choices.skeleton.get(&skin.skin) == Some(skeleton))
                    .map(|(id, name, _)| (*id, name.as_str())),
            );
            if authored {
                remove = ui.small_button("移除蒙皮绑定").clicked();
            }
        });
        if editable && !cancel {
            if remove {
                replace(editor, assets, entity, entity.animator.clone(), None);
                return;
            }
            if animator != skin.animator {
                replace(
                    editor,
                    assets,
                    entity,
                    entity.animator.clone(),
                    Some(SkinBinding {
                        skin: skin.skin,
                        animator,
                    }),
                );
            }
        }
    } else if authored && entity.model.is_some() {
        ui.add_enabled_ui(editable, |ui| {
            asset_picker(
                ui,
                ("new-skin", editor.world(), entity.id),
                "蒙皮资源",
                &mut draft.skin,
                choices.skins.iter().copied(),
                &choices,
                Some("请选择蒙皮"),
            );
            if ui
                .add_enabled(draft.skin.is_some(), egui::Button::new("添加蒙皮绑定"))
                .clicked()
                && !cancel
            {
                replace(
                    editor,
                    assets,
                    entity,
                    entity.animator.clone(),
                    draft.skin.map(|skin| SkinBinding {
                        skin,
                        animator: None,
                    }),
                );
            }
        });
    } else if authored {
        ui.weak("先为实体添加网格渲染，再绑定蒙皮。");
    }
    draw_character_driver(
        editor,
        ui,
        assets,
        entity,
        targets.as_ref(),
        authored,
        editable,
        cancel,
    );
    if !editable {
        ui.weak(if editor.play.is_some() {
            "运行期间动画配置只读。"
        } else {
            "Prefab 组件结构或嵌套配置请在源 Prefab 中编辑。"
        });
    }
    if editable && cancel {
        let result = editor.cancel();
        editor.notice(result);
        draft.skeleton = None;
        draft.skin = None;
    }
    ui.data_mut(|data| data.insert_temp(key, draft));
}

fn replace(
    editor: &mut Editor,
    assets: &mut AssetStore,
    entity: &EntityData,
    animator: Option<Animator>,
    skin: Option<SkinBinding>,
) {
    let result = editor
        .commit()
        .and_then(|_| editor.edit_animation(entity.id, animator, skin, assets))
        .and_then(|_| editor.commit());
    editor.notice(result);
}

#[allow(clippy::too_many_arguments)]
fn draw_character_driver(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
    targets: Option<&Targets>,
    authored: bool,
    editable: bool,
    cancel: bool,
) {
    if entity.character_animation.is_none() && (!authored || entity.parent.is_none()) {
        return;
    }
    ui.strong("角色动画");
    let mut next = entity.character_animation.clone();
    let mut changed = false;
    ui.add_enabled_ui(editable, |ui| {
        if let Some(driver) = &mut next {
            ui.weak("从父角色的实际移动、着地和蹲伏状态驱动动画；本实体负责外观的脚底位置与朝向。");
            ui.label("角色实体");
            entity_picker(
                ui,
                ("animation-character", editor.world(), entity.id),
                &mut driver.character,
                targets
                    .into_iter()
                    .flat_map(|t| t.nodes.iter())
                    .filter(|(id, _)| Some(*id) == entity.parent)
                    .map(|(id, name)| (*id, name.as_str())),
            );
            ui.label("动画实体");
            entity_picker(
                ui,
                ("character-animator", editor.world(), entity.id),
                &mut driver.animator,
                targets
                    .into_iter()
                    .flat_map(|t| t.animators.iter())
                    .map(|(id, name, _)| (*id, name.as_str())),
            );
            if authored && ui.small_button("移除角色动画").clicked() {
                next = None;
            }
        } else if ui.small_button("添加角色动画").clicked() {
            next = Some(comet_scene::CharacterAnimation {
                character: entity.parent.map(EntityRef::local),
                animator: None,
            });
        }
        changed = next != entity.character_animation;
    });
    if editable && !cancel && changed {
        let result = editor
            .commit()
            .and_then(|_| editor.edit_character_animation(entity.id, next, assets))
            .and_then(|_| editor.commit());
        editor.notice(result);
    }
}

pub(super) fn rebuild_bindings(editor: &mut Editor, assets: &mut AssetStore, ids: &[EntityId]) {
    let result = editor
        .commit()
        .and_then(|_| editor.rebuild_animation_bindings(ids, assets))
        .and_then(|_| editor.commit());
    editor.notice(result);
}

#[cfg(test)]
mod tests;
