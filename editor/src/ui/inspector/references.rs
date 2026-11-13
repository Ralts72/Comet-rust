use crate::{fields::accepts_binding, resources::ResourceVersion, session::Editor};
use anyhow::Result;
use comet_asset::AssetStore;
use comet_core::EntityId;
use comet_scene::{
    EntityData, EntityRef,
    schema::{self, FieldKey, FieldValue},
};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone)]
struct Choices {
    version: ResourceVersion,
    owners: Vec<EntityId>,
    values: Result<Arc<Vec<(EntityId, String)>>, String>,
}

pub(super) fn choices(
    editor: &Editor,
    ui: &egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    field: FieldKey,
) -> Result<Arc<Vec<(EntityId, String)>>, String> {
    let key = egui::Id::unique("inspector-binding-choices").with((field.component, field.field));
    let owners: Vec<_> = entities.iter().map(|entity| entity.id).collect();
    if let Some(cache) = ui
        .data(|data| data.get_temp::<Choices>(key))
        .filter(|cache| cache.owners == owners && cache.version.matches(&editor.document, assets))
    {
        return cache.values;
    }
    let values = (|| -> Result<_> {
        let mut values = None;
        for &id in &owners {
            let target = editor.resolve_field_target(id, assets)?;
            let references = editor.field_references(target, assets)?;
            if let Some(values) = &mut values {
                let ids: BTreeSet<_> = references.targets.iter().map(|(id, _)| *id).collect();
                Vec::retain(values, |(id, _)| ids.contains(id));
            } else {
                values = Some(references.targets);
            }
        }
        let mut values = values.unwrap_or_default();
        values.retain(|(id, _)| {
            editor
                .last_snapshot
                .entities
                .binary_search_by_key(id, |entity| entity.id)
                .ok()
                .is_some_and(|index| {
                    entities.iter().all(|owner| {
                        accepts_binding(
                            field,
                            owner,
                            &editor.last_snapshot.entities[index],
                            &editor.last_snapshot,
                            assets.index(),
                        )
                    })
                })
        });
        Ok(Arc::new(values))
    })()
    .map_err(|error| format!("{error:#}"));
    ui.data_mut(|data| {
        data.insert_temp(
            key,
            Choices {
                version: ResourceVersion::capture(&editor.document, assets),
                owners,
                values: values.clone(),
            },
        )
    });
    values
}

/// Return after a published edit so callers never draw later controls from stale entities.
pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    fields: &[FieldKey],
) -> bool {
    let Some(primary) = entities.first() else {
        return false;
    };
    for &field in fields {
        let Some((FieldValue::Entity(current), mixed)) =
            super::properties::shared_value(entities, field)
        else {
            continue;
        };
        let (label, empty) = match field {
            schema::NAVIGATION_TARGET => ("导航目标", "无目标"),
            schema::SKIN_ANIMATOR | schema::CHARACTER_ANIMATION_ANIMATOR => ("动画实体", "未绑定"),
            schema::CHARACTER_ANIMATION_CHARACTER => ("角色实体", "未绑定"),
            _ => continue,
        };
        let editable = editor.play.is_none()
            && !editor.is_busy()
            && !editor.documents.is_blocking()
            && ui.is_enabled();
        let choices = if editable {
            choices(editor, ui, assets, entities, field)
        } else {
            Ok(Arc::default())
        };
        let mut value = current.clone();
        let label = if mixed {
            format!("{label}（多值）")
        } else {
            label.into()
        };
        let mut selected = false;
        let response = ui.add_enabled_ui(editable && choices.is_ok(), |ui| {
            ui.horizontal(|ui| {
                ui.label(label);
                selected = pick(
                    ui,
                    (
                        "entity-binding",
                        editor.world(),
                        primary.id,
                        editor.selection.generation(),
                        field.component,
                        field.field,
                    ),
                    &mut value,
                    choices
                        .as_ref()
                        .into_iter()
                        .flat_map(|list| list.iter())
                        .map(|(id, name)| (*id, name.as_str())),
                    empty,
                    mixed,
                    current.as_ref().and_then(|reference| match reference {
                        EntityRef::Local { entity } => editor
                            .last_snapshot
                            .entities
                            .iter()
                            .find(|e| e.id == *entity)
                            .map(|e| e.name.as_str()),
                        _ => None,
                    }),
                );
            });
        });
        if let Err(error) = choices {
            response.response.on_disabled_hover_text(error);
        }
        if editable && selected && !ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            let target = value.and_then(|reference| match reference {
                EntityRef::Local { entity } => Some(entity),
                _ => None,
            });
            let ids: Vec<_> = entities.iter().map(|entity| entity.id).collect();
            let revision = editor.document.revision();
            let result = editor
                .commit()
                .and_then(|_| editor.replace_entity_bindings(&ids, field, target, assets))
                .and_then(|_| editor.commit());
            editor.notice(result);
            if editor.document.revision() != revision {
                return true;
            }
        }
    }
    false
}

pub(super) fn pick<'a>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut Option<EntityRef>,
    list: impl Iterator<Item = (EntityId, &'a str)> + Clone,
    empty: &str,
    mixed: bool,
    fallback: Option<&str>,
) -> bool {
    let current = value.as_ref().and_then(|reference| match reference {
        EntityRef::Local { entity } => Some(*entity),
        _ => None,
    });
    let name = if mixed {
        "多值"
    } else if value.is_none() {
        empty
    } else {
        list.clone()
            .find(|(id, _)| Some(*id) == current)
            .map(|(_, name)| name)
            .or(fallback)
            .unwrap_or("当前绑定（只读或不兼容）")
    };
    let mut selected = None;
    egui::ComboBox::from_id_salt(salt)
        .width(ui.available_width().min(230.))
        .truncate()
        .selected_text(name)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(!mixed && value.is_none(), empty)
                .clicked()
            {
                selected = Some(None);
            }
            for (id, name) in list {
                if ui
                    .selectable_label(!mixed && current == Some(id), name)
                    .clicked()
                {
                    selected = Some(Some(EntityRef::local(id)));
                }
            }
        });
    if let Some(selected) = selected {
        *value = selected;
        true
    } else {
        false
    }
}
