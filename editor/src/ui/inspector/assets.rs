use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use comet_core::{AssetId, EntityId};
use comet_scene::{
    EntityData,
    schema::{self, FieldKey, FieldValue},
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default)]
pub(super) struct Catalog {
    revision: String,
    pub meshes: Vec<AssetId>,
    pub materials: Vec<AssetId>,
    pub audio: Vec<AssetId>,
    pub scripts: Vec<AssetId>,
    pub skeletons: Vec<AssetId>,
    pub clips: Vec<AssetId>,
    pub controllers: Vec<AssetId>,
    pub skins: Vec<AssetId>,
    pub skeleton: BTreeMap<AssetId, AssetId>,
    pub labels: BTreeMap<AssetId, String>,
}

pub(super) fn catalog(ui: &egui::Ui, assets: &AssetStore) -> Arc<Catalog> {
    let key = egui::Id::unique("inspector-asset-catalog");
    if let Some(value) = ui
        .data(|data| data.get_temp::<Arc<Catalog>>(key))
        .filter(|value| value.revision == assets.revision())
    {
        return value;
    }
    let mut value = Catalog {
        revision: assets.revision().into(),
        ..Default::default()
    };
    for (&id, entry) in &assets.index().products {
        let (list, kind) = match entry.kind {
            AssetKind::Mesh => (&mut value.meshes, "网格"),
            AssetKind::Material => (&mut value.materials, "材质"),
            AssetKind::Audio => (&mut value.audio, "音频"),
            AssetKind::Script => (&mut value.scripts, "脚本"),
            AssetKind::Skeleton => (&mut value.skeletons, "骨架"),
            AssetKind::Skin => (&mut value.skins, "蒙皮"),
            AssetKind::AnimationClip => (&mut value.clips, "动画片段"),
            AssetKind::AnimationController => (&mut value.controllers, "动画控制器"),
            _ => continue,
        };
        let label = entry.editor.as_ref().map_or_else(
            || match entry.kind {
                AssetKind::Script => "未命名脚本".into(),
                AssetKind::Skeleton | AssetKind::Skin | AssetKind::AnimationClip => {
                    format!("未命名{kind} {}", list.len() + 1)
                }
                _ => format!("未命名{kind} ({})", list.len() + 1),
            },
            |metadata| {
                if metadata.source == id {
                    metadata.label.clone()
                } else {
                    let source = assets
                        .index()
                        .products
                        .get(&metadata.source)
                        .and_then(|entry| entry.editor.as_ref())
                        .map_or("未命名来源", |metadata| metadata.label.as_str());
                    format!("{source} / {}", metadata.label)
                }
            },
        );
        list.push(id);
        value.labels.insert(id, label);
        if let Some(skeleton) = entry.dependencies.iter().find(|id| {
            assets
                .index()
                .products
                .get(id)
                .is_some_and(|entry| entry.kind == AssetKind::Skeleton)
        }) {
            value.skeleton.insert(id, *skeleton);
        }
    }
    for list in [
        &mut value.meshes,
        &mut value.materials,
        &mut value.audio,
        &mut value.scripts,
        &mut value.skeletons,
        &mut value.clips,
        &mut value.controllers,
        &mut value.skins,
    ] {
        list.sort_by(|a, b| value.labels[a].cmp(&value.labels[b]).then(a.cmp(b)));
    }
    let value = Arc::new(value);
    ui.data_mut(|data| data.insert_temp(key, value.clone()));
    value
}

pub(super) fn draw_common(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    fields: &[FieldKey],
) {
    let Some(primary) = entities.first() else {
        return;
    };
    let choices = catalog(ui, assets);
    let editable = editor.play.is_none()
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
    for &field in fields {
        let Some((value, mixed)) = super::properties::shared_value(entities, field) else {
            continue;
        };
        let current = match value {
            FieldValue::Asset(id) => Some(id),
            FieldValue::OptionalAsset(id) => id,
            _ => unreachable!("asset menu field"),
        };
        let (label, list, empty) = match field {
            schema::MESH => ("网格", &choices.meshes, None),
            schema::MATERIAL => ("材质", &choices.materials, None),
            schema::AUDIO_CLIP => ("音频片段", &choices.audio, None),
            schema::SCRIPT_ASSET => ("脚本", &choices.scripts, None),
            schema::ANIMATOR_SKELETON => ("骨架", &choices.skeletons, None),
            schema::ANIMATOR_CLIP => ("播放片段", &choices.clips, Some("不播放片段")),
            schema::ANIMATOR_CONTROLLER => ("动画控制器", &choices.controllers, Some("单片段播放")),
            schema::SKIN_ASSET => ("蒙皮资源", &choices.skins, None),
            _ => unreachable!("common asset menu field"),
        };
        let selected = ui
            .add_enabled_ui(
                editable
                    && (field != schema::ANIMATOR_CLIP
                        || entities
                            .iter()
                            .all(|e| e.animator.as_ref().is_none_or(|a| a.controller.is_none()))),
                |ui| {
                    draw(
                        ui,
                        editor,
                        primary.id,
                        Choice {
                            field,
                            label,
                            current,
                            mixed,
                            empty,
                        },
                        list.iter()
                            .copied()
                            .filter(|id| {
                                !matches!(
                                    field,
                                    schema::ANIMATOR_CLIP | schema::ANIMATOR_CONTROLLER
                                ) || choices.skeleton.get(id).is_some_and(|skeleton| {
                                    entities.iter().all(|entity| {
                                        entity
                                            .animator
                                            .as_ref()
                                            .is_some_and(|animator| animator.skeleton == *skeleton)
                                    })
                                })
                            })
                            .map(|id| (id, choices.labels[&id].as_str())),
                    )
                },
            )
            .inner;
        if field == schema::ANIMATOR_SKELETON {
            ui.weak("更换骨架会重建绑定并清除不匹配的播放资源；不兼容的蒙皮会使整组修改被拒绝。");
        }
        if editable
            && !cancel
            && let Some(selected) = selected
        {
            let ids = entities.iter().map(|entity| entity.id).collect::<Vec<_>>();
            let revision = editor.document.revision();
            let result = editor
                .commit()
                .and_then(|_| match field {
                    schema::MESH | schema::MATERIAL => {
                        editor.replace_model_asset(&ids, field, selected.unwrap(), assets)
                    }
                    schema::AUDIO_CLIP => {
                        editor.replace_audio_clips(&ids, selected.unwrap(), assets)
                    }
                    schema::SCRIPT_ASSET => {
                        editor.replace_script_assets(&ids, selected.unwrap(), assets)
                    }
                    schema::ANIMATOR_SKELETON => {
                        editor.replace_animation_skeletons(&ids, selected.unwrap(), assets)
                    }
                    schema::ANIMATOR_CLIP => editor.replace_animation_clips(&ids, selected, assets),
                    schema::ANIMATOR_CONTROLLER => {
                        editor.replace_animation_controllers(&ids, selected, assets)
                    }
                    schema::SKIN_ASSET => {
                        editor.replace_skin_assets(&ids, selected.unwrap(), assets)
                    }
                    _ => unreachable!("common asset menu field"),
                })
                .and_then(|_| editor.commit());
            editor.notice(result);
            if editor.document.revision() != revision {
                return;
            }
        }
    }
}

pub(super) struct Choice {
    pub field: FieldKey,
    pub label: &'static str,
    pub current: Option<AssetId>,
    pub mixed: bool,
    pub empty: Option<&'static str>,
}

/// Returns an explicit menu choice, including None for an optional asset.
pub(super) fn draw<'a>(
    ui: &mut egui::Ui,
    editor: &Editor,
    primary: EntityId,
    choice: Choice,
    items: impl Iterator<Item = (AssetId, &'a str)> + Clone,
) -> Option<Option<AssetId>> {
    let selected = choice.current.map_or_else(
        || {
            choice
                .empty
                .map_or_else(|| format!("选择{}", choice.label), str::to_owned)
        },
        |id| {
            items.clone().find(|(asset, _)| *asset == id).map_or_else(
                || format!("当前{}已缺失", choice.label),
                |(_, label)| label.to_owned(),
            )
        },
    );
    let mut result = None;
    ui.horizontal(|ui| {
        ui.label(if choice.mixed {
            format!("{}（多值）", choice.label)
        } else {
            choice.label.into()
        });
        egui::ComboBox::from_id_salt((
            "component-asset",
            editor.world(),
            primary,
            editor.selection.generation(),
            choice.field.component,
            choice.field.field,
        ))
        .width(ui.available_width().min(240.))
        .truncate()
        .selected_text(if choice.mixed { "多值" } else { &selected })
        .show_ui(ui, |ui| {
            if let Some(empty) = choice.empty {
                let active = !choice.mixed && choice.current.is_none();
                if ui.selectable_label(active, empty).clicked() && !active {
                    result = Some(None);
                }
            }
            let mut items = items.peekable();
            if items.peek().is_none() {
                ui.weak("没有匹配的资产");
            }
            for (id, label) in items {
                let active = !choice.mixed && choice.current == Some(id);
                if ui.selectable_label(active, label).clicked() && !active {
                    result = Some(Some(id));
                }
            }
        })
        .response
        .on_hover_text(selected);
    });
    result
}

pub(super) fn pick_draft(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    value: &mut Option<AssetId>,
    list: impl Iterator<Item = AssetId>,
    choices: &Catalog,
    empty: Option<&str>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(salt)
            .width(ui.available_width().min(230.))
            .truncate()
            .selected_text(
                value
                    .and_then(|id| choices.labels.get(&id).map(String::as_str))
                    .unwrap_or(empty.unwrap_or("当前资源已缺失")),
            )
            .show_ui(ui, |ui| {
                if let Some(empty) = empty {
                    ui.selectable_value(value, None, empty);
                }
                for id in list {
                    ui.selectable_value(value, Some(id), &choices.labels[&id]);
                }
            });
    });
}
