use crate::session::Editor;
use anyhow::{Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use comet_scene::{EntityData, EntityRef, Script, ScriptValue};

const TYPES: [&str; 6] = ["布尔", "整数", "数值", "文本", "三维向量", "实体引用"];

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.separator();
    ui.strong("脚本");
    let authored = editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .any(|e| e.id == entity.id);
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let editable = editor.play.is_none()
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && ui.is_enabled();
    let Some(script) = &entity.script else {
        if !authored {
            ui.weak("Prefab 的脚本组件结构请在源 Prefab 中编辑。");
            return;
        }
        let asset = editor
            .sources
            .of_kind(AssetKind::Script)
            .next()
            .map(|s| s.id);
        if let Some(asset) = asset {
            if ui
                .add_enabled(editable, egui::Button::new("添加脚本"))
                .clicked()
                && !cancel
            {
                replace(editor, assets, entity.id, Some(Script::new(asset)));
            }
        } else {
            ui.weak("将 .rhai 脚本放入 assets 并刷新资源后，可添加脚本组件。");
        }
        return;
    };
    let mut next = script.clone();
    let mut finish = false;
    let mut remove = false;
    let mut error = None;
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal(|ui| {
            finish |= ui.checkbox(&mut next.enabled, "启用脚本").changed();
            if authored {
                remove = ui.small_button("移除脚本").clicked();
            }
        });
        let before = next.asset;
        egui::ComboBox::from_id_salt(("script-asset", editor.world(), entity.id))
            .selected_text(
                editor
                    .sources
                    .of_kind(AssetKind::Script)
                    .find(|source| source.id == next.asset)
                    .map_or_else(
                        || next.asset.to_string(),
                        |source| source.source.display().to_string(),
                    ),
            )
            .show_ui(ui, |ui| {
                for source in editor.sources.of_kind(AssetKind::Script) {
                    ui.selectable_value(
                        &mut next.asset,
                        source.id,
                        source.source.display().to_string(),
                    );
                }
            });
        finish |= next.asset != before;
        if authored {
            if !remove
                && let Err(value) =
                    parameters(ui, editor, assets, entity, &mut next, &mut finish, cancel)
            {
                error = Some(value);
            }
        } else {
            ui.weak("Prefab 实例可覆盖脚本资源和启用状态；参数及组件结构请在源 Prefab 中编辑。");
            for (name, value) in &script.parameters {
                ui.label(format!("{name}: {value:?}"));
            }
        }
    });
    if !editable {
        if editor.play.is_some() {
            ui.weak("运行期间脚本配置只读，停止后可修改。");
        }
        return;
    }
    if cancel {
        let result = editor.cancel();
        editor.notice(result);
        return;
    }
    if let Some(error) = error {
        editor.notice(Err(error));
        return;
    }
    if remove {
        replace(editor, assets, entity.id, None);
        return;
    }
    if next != *script {
        let result = editor.edit_script(entity.id, Some(next), assets);
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
    id: comet_core::EntityId,
    value: Option<Script>,
) {
    let result = editor
        .commit()
        .and_then(|_| editor.edit_script(id, value, assets))
        .and_then(|_| editor.commit());
    editor.notice(result);
}

fn parameters(
    ui: &mut egui::Ui,
    editor: &Editor,
    assets: &mut AssetStore,
    entity: &EntityData,
    script: &mut Script,
    finish: &mut bool,
    cancel: bool,
) -> Result<()> {
    let keys: Vec<_> = script.parameters.keys().cloned().collect();
    let mut rename = None;
    let mut remove = None;
    for key in keys {
        ui.push_id(
            ("script-parameter", editor.world(), entity.id, &key),
            |ui| {
                let draft_id = ui.make_persistent_id("name");
                let draft = ui.ctx().data_mut(|data| data.get_temp::<String>(draft_id));
                ui.horizontal(|ui| {
                    ui.label(format!("参数：{key}"));
                    if draft.is_none() && ui.small_button("重命名").clicked() {
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(draft_id, key.clone()));
                    }
                    if ui.small_button("删除参数").clicked() {
                        remove = Some(key.clone());
                        ui.ctx().data_mut(|data| data.remove::<String>(draft_id));
                        *finish = true;
                    }
                });
                if let Some(mut draft) = draft {
                    ui.weak("名称草稿仅在确认后生效。");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut draft)
                            .id(draft_id)
                            .hint_text("参数名")
                            .desired_width(160.),
                    );
                    if ui.is_enabled() && response.changed() {
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(draft_id, draft.clone()));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("确认重命名").clicked() {
                            if draft != key {
                                rename = Some((key.clone(), draft, draft_id));
                            } else {
                                ui.ctx().data_mut(|data| data.remove::<String>(draft_id));
                            }
                        }
                        if ui.button("取消重命名").clicked() || (cancel && ui.is_enabled()) {
                            ui.ctx().data_mut(|data| data.remove::<String>(draft_id));
                            rename = None;
                        }
                    });
                }
                let value = script.parameters.get_mut(&key).unwrap();
                let mut kind = kind(value);
                let before = kind;
                egui::ComboBox::from_id_salt("type")
                    .selected_text(TYPES[kind])
                    .show_ui(ui, |ui| {
                        for (index, label) in TYPES.iter().enumerate() {
                            ui.selectable_value(&mut kind, index, *label);
                        }
                    });
                if kind != before {
                    *value = default_value(kind);
                    *finish = true;
                }
                draw_value(ui, editor, assets, entity.id, value, finish);
                ui.add_space(4.);
            },
        );
    }
    if let Some((before, after, draft_id)) = rename
        && remove.as_ref() != Some(&before)
    {
        ensure!(!after.is_empty(), "参数名不能为空");
        ensure!(
            !script.parameters.contains_key(&after),
            "参数名已存在：{after}"
        );
        let value = script.parameters.remove(&before).unwrap();
        script.parameters.insert(after, value);
        ui.ctx().data_mut(|data| data.remove::<String>(draft_id));
        *finish = true;
    }
    if let Some(key) = remove {
        script.parameters.remove(&key);
    }
    ui.separator();
    let id = ui.make_persistent_id(("new-script-parameter", editor.world(), entity.id));
    let mut name = ui
        .ctx()
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_default();
    ui.add(
        egui::TextEdit::singleline(&mut name)
            .id(id)
            .hint_text("新参数名"),
    );
    ui.ctx().data_mut(|data| data.insert_temp(id, name.clone()));
    if ui.button("添加参数").clicked() && !cancel {
        ensure!(!name.is_empty(), "参数名不能为空");
        ensure!(
            !script.parameters.contains_key(&name),
            "参数名已存在：{name}"
        );
        script.parameters.insert(name, ScriptValue::Number(0.));
        ui.ctx().data_mut(|data| data.remove::<String>(id));
        *finish = true;
    }
    Ok(())
}

fn kind(value: &ScriptValue) -> usize {
    match value {
        ScriptValue::Bool(_) => 0,
        ScriptValue::Integer(_) => 1,
        ScriptValue::Number(_) => 2,
        ScriptValue::Text(_) => 3,
        ScriptValue::Vec3(_) => 4,
        ScriptValue::Entity(_) => 5,
    }
}

fn default_value(kind: usize) -> ScriptValue {
    match kind {
        0 => ScriptValue::Bool(false),
        1 => ScriptValue::Integer(0),
        2 => ScriptValue::Number(0.),
        3 => ScriptValue::Text(String::new()),
        4 => ScriptValue::Vec3([0.; 3]),
        _ => ScriptValue::Entity(None),
    }
}

fn draw_value(
    ui: &mut egui::Ui,
    editor: &Editor,
    assets: &mut AssetStore,
    owner: comet_core::EntityId,
    value: &mut ScriptValue,
    finish: &mut bool,
) {
    let response = match value {
        ScriptValue::Bool(value) => {
            *finish |= ui.checkbox(value, "值").changed();
            return;
        }
        ScriptValue::Integer(value) => ui.add(egui::DragValue::new(value).speed(1.)),
        ScriptValue::Number(value) => ui.add(egui::DragValue::new(value).speed(0.05)),
        ScriptValue::Text(value) => ui.add(egui::TextEdit::singleline(value).hint_text("文本值")),
        ScriptValue::Vec3(values) => {
            ui.horizontal(|ui| {
                for (axis, value) in ["X", "Y", "Z"].into_iter().zip(values) {
                    let response = ui.add(
                        egui::DragValue::new(value)
                            .prefix(format!("{axis} "))
                            .speed(0.05),
                    );
                    *finish |= response.drag_stopped() || response.lost_focus();
                }
            });
            return;
        }
        ScriptValue::Entity(value) => {
            let before = value.clone();
            let targets = if editor.play.is_none() && ui.is_enabled() {
                editor.cached_field_references(ui.ctx(), owner, assets).ok()
            } else {
                None
            };
            egui::ComboBox::from_id_salt("entity")
                .selected_text(match &*value {
                    None => "无引用".into(),
                    Some(EntityRef::Local { entity }) => targets
                        .as_ref()
                        .and_then(|targets| targets.targets.iter().find(|(id, _)| id == entity))
                        .map(|(_, name)| name.clone())
                        .or_else(|| {
                            editor
                                .last_snapshot
                                .entities
                                .iter()
                                .find(|node| node.id == *entity)
                                .map(|node| node.name.clone())
                        })
                        .unwrap_or_else(|| "目标已缺失".into()),
                    Some(EntityRef::InInstance { .. }) => "实例引用".into(),
                    Some(EntityRef::External { .. }) => "外部场景引用".into(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(value, None, "无引用");
                    for (id, name) in targets.iter().flat_map(|targets| &targets.targets) {
                        ui.selectable_value(value, Some(EntityRef::local(*id)), name);
                    }
                });
            *finish |= before != *value;
            return;
        }
    };
    *finish |= response.drag_stopped() || response.lost_focus();
}

#[cfg(test)]
mod tests;
