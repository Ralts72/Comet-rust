use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use comet_core::{AssetId, EntityId, WorldId};
use comet_editor_core::FieldTarget;
use comet_scene::{
    AudioSource, EntityData,
    schema::{self, FieldValue},
};
use std::sync::Arc;

struct Choices {
    revision: String,
    clips: Vec<(AssetId, String)>,
}

fn choices(ui: &egui::Ui, assets: &AssetStore) -> Arc<Choices> {
    let key = egui::Id::unique("inspector-audio-choices");
    ui.data(|data| data.get_temp::<Arc<Choices>>(key))
        .filter(|cache| cache.revision == assets.revision())
        .unwrap_or_else(|| {
            let mut clips = Vec::new();
            for (&id, entry) in &assets.index().products {
                if entry.kind != AssetKind::Audio {
                    continue;
                }
                let label = entry.editor.as_ref().map_or_else(
                    || format!("未命名音频 ({})", clips.len() + 1),
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
                clips.push((id, label));
            }
            clips.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
            let cache = Arc::new(Choices {
                revision: assets.revision().to_owned(),
                clips,
            });
            ui.data_mut(|data| data.insert_temp(key, cache.clone()));
            cache
        })
}

#[derive(Clone)]
struct Draft {
    world: WorldId,
    entity: EntityId,
    revision: String,
    clip: Option<AssetId>,
}
fn draft_id() -> egui::Id {
    egui::Id::unique("inspector-audio-draft")
}
fn clear_draft(ui: &egui::Ui) {
    ui.data_mut(|data| data.remove::<Draft>(draft_id()));
}

fn picker(
    ui: &mut egui::Ui,
    editor: &Editor,
    entity: EntityId,
    value: &mut Option<AssetId>,
    choices: &Choices,
) {
    ui.horizontal(|ui| {
        ui.label("音频片段");
        let selected = value.map_or("选择音频", |id| {
            choices
                .clips
                .iter()
                .find(|(asset, _)| *asset == id)
                .map_or("当前音频片段已缺失", |(_, label)| label.as_str())
        });
        egui::ComboBox::from_id_salt(("audio-clip", editor.world(), entity))
            .width(ui.available_width().min(240.))
            .truncate()
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (id, label) in &choices.clips {
                    ui.selectable_value(value, Some(*id), label);
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
    ui.strong("音频源");
    // A Play world's entity IDs do not identify fields in the author document.
    let target = if editor.play.is_none() {
        Some(super::overrides::target(editor, ui, assets, entity.id))
    } else {
        None
    };
    let authored = matches!(target, Some(Ok(FieldTarget::Authored(_))));
    let nested = matches!(&target, Some(Err(_)));
    let editable = target.as_ref().is_some_and(|value| value.is_ok())
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let Some(source) = &entity.audio_source else {
        if authored {
            let choices = choices(ui, assets);
            add(editor, ui, assets, entity.id, &choices, editable, cancel);
        } else {
            clear_draft(ui);
            ui.weak(if editor.play.is_some() {
                "运行期间音频源配置只读。"
            } else {
                "Prefab 的音频源组件结构请在源 Prefab 中编辑。"
            });
        }
        return;
    };
    clear_draft(ui);
    let choices = choices(ui, assets);
    let mut clip = Some(source.clip);
    let mut next = source.clone();
    let mut remove = false;
    let mut gain_response = None;
    let mut distances = Vec::new();
    ui.add_enabled_ui(editable, |ui| {
        picker(ui, editor, entity.id, &mut clip, &choices);
        gain_response = Some(ui.add(egui::Slider::new(&mut next.gain, 0. ..=1.).text("音量")));
        ui.checkbox(&mut next.looping, "循环");
        ui.checkbox(&mut next.autoplay, "启动时播放");
        ui.checkbox(&mut next.spatial, "空间音效");
        ui.add_enabled_ui(next.spatial, |ui| {
            for (label, field, value) in [
                (
                    "开始衰减距离（米）",
                    schema::AUDIO_MIN_DISTANCE,
                    &mut next.min_distance,
                ),
                (
                    "最大距离（米）",
                    schema::AUDIO_MAX_DISTANCE,
                    &mut next.max_distance,
                ),
            ] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    let response = ui.add(
                        egui::DragValue::new(&mut *value)
                            .speed(0.1)
                            .range(0. ..=f32::MAX),
                    );
                    distances.push((field, *value, response));
                });
            }
        });
        if authored {
            remove = ui.small_button("移除音频源").clicked();
        }
    });
    if !editable {
        ui.weak(if editor.play.is_some() {
            "运行期间音频源配置只读。"
        } else if nested {
            "嵌套实例的音频源请在源 Prefab 中编辑。"
        } else {
            "当前音频源配置暂不可编辑。"
        });
        play_controls(editor, ui, entity.id);
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
    if clip != Some(source.clip)
        || next.looping != source.looping
        || next.autoplay != source.autoplay
        || next.spatial != source.spatial
    {
        next.clip = clip.unwrap();
        replace(editor, assets, entity.id, Some(next));
        return;
    }
    let response = gain_response.unwrap();
    if response.changed() {
        let result = editor.edit(
            entity.id,
            schema::AUDIO_GAIN,
            FieldValue::Scalar(next.gain),
            assets,
        );
        editor.notice(result);
    }
    let mut finish = response.drag_stopped() || response.lost_focus();
    for (field, value, response) in distances {
        if response.changed() {
            let result = editor.edit(entity.id, field, FieldValue::Scalar(value), assets);
            editor.notice(result);
        }
        finish |= response.drag_stopped() || response.lost_focus();
    }
    if finish {
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
                && draft.revision == assets.revision()
        })
        .unwrap_or_else(|| Draft {
            world: editor.document.world_id(),
            entity,
            revision: assets.revision().to_owned(),
            clip: None,
        });
    let mut add = false;
    ui.add_enabled_ui(editable, |ui| {
        picker(ui, editor, entity, &mut draft.clip, choices);
        add = ui
            .add_enabled(draft.clip.is_some(), egui::Button::new("添加音频源"))
            .clicked();
    });
    if choices.clips.is_empty() {
        ui.weak("将 .wav 音频放入 assets 并刷新资源后，可添加音频源。");
    }
    if cancel {
        draft.clip = None;
    }
    if add && !cancel {
        replace(
            editor,
            assets,
            entity,
            Some(AudioSource::new(draft.clip.unwrap())),
        );
    } else {
        ui.data_mut(|data| data.insert_temp(draft_id(), draft));
    }
}

fn replace(
    editor: &mut Editor,
    assets: &mut AssetStore,
    entity: EntityId,
    source: Option<AudioSource>,
) {
    let result = editor
        .commit()
        .and_then(|_| editor.edit_audio_source(entity, source, assets))
        .and_then(|_| editor.commit());
    editor.notice(result);
}

fn play_controls(editor: &mut Editor, ui: &mut egui::Ui, entity: EntityId) {
    if editor.play.is_none() {
        return;
    }
    let enabled = ui.is_enabled() && !editor.is_busy() && !editor.documents.is_blocking();
    let mut play = false;
    let mut stop = false;
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            play = ui.button("播放声音").clicked();
            stop = ui.button("停止声音").clicked();
        });
    });
    let session = editor.play.as_mut().unwrap();
    if session.is_paused() || session.is_suspended() {
        ui.weak("恢复运行后才会出声。");
    }
    let result = if play {
        session.play_sound(entity)
    } else if stop {
        session.stop_sound(entity)
    } else {
        return;
    };
    editor.notice(result);
}

#[cfg(test)]
mod tests;
