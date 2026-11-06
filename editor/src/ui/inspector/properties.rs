use crate::session::Editor;
use comet_asset::AssetStore;
use comet_scene::{
    EntityData, FieldUpdate,
    schema::{self, FieldKey, FieldValue},
};

pub(super) fn shared_value(
    entities: &[&EntityData],
    field: FieldKey,
) -> Option<(FieldValue, bool)> {
    let descriptor = schema::descriptor(field).ok()?;
    let first = descriptor.read(entities.first()?)?;
    let mut mixed = false;
    for entity in entities.iter().skip(1) {
        mixed |= descriptor.read(entity)? != first;
    }
    Some((first, mixed))
}

pub(super) struct PropertyGesture {
    rejected_id: egui::Id,
    rejected: bool,
    enabled: bool,
    cancel: bool,
    confirm: bool,
    active: bool,
    finish: bool,
}
impl PropertyGesture {
    pub(super) fn number(&self, ui: &mut egui::Ui, widget: egui::DragValue<'_>) -> egui::Response {
        ui.add_enabled_ui(self.enabled, |ui| {
            let id = ui.next_auto_id();
            let context = ui.ctx().clone();
            let parser_context = context.clone();
            let invalid_text_id = self.rejected_id.with("invalid-text");
            let formatter = ui.style().number_formatter.clone();
            ui.add(
                widget
                    .custom_parser(move |text| {
                        let text: String = text
                            .chars()
                            .filter(|character| !character.is_whitespace())
                            .map(|character| if character == '−' { '-' } else { character })
                            .collect();
                        let parsed = text.parse::<f64>().ok();
                        parser_context
                            .data_mut(|data| data.insert_temp(invalid_text_id, parsed.is_none()));
                        parsed
                    })
                    .custom_formatter(move |value, precision| {
                        // DragValue parses its text on focus loss. Display rounding must not edit data.
                        if context.memory(|memory| memory.has_focus(id)) {
                            (value as f32).to_string()
                        } else {
                            formatter.format(value, precision)
                        }
                    }),
            )
        })
        .inner
    }
    pub(super) fn new(ui: &egui::Ui, enabled: bool, field: comet_scene::schema::FieldKey) -> Self {
        let rejected_id =
            ui.make_persistent_id(("property-preview-rejected", field.component, field.field));
        Self {
            rejected_id,
            rejected: ui
                .data(|data| data.get_temp::<bool>(rejected_id))
                .unwrap_or(false),
            enabled: enabled && ui.is_enabled(),
            // Numeric widgets consume these keys while ending text focus.
            cancel: ui.input(|input| input.key_pressed(egui::Key::Escape)),
            confirm: ui.input(|input| input.key_pressed(egui::Key::Enter)),
            active: false,
            finish: false,
        }
    }
    pub(super) fn observe(&mut self, response: &egui::Response, mixed: bool) -> bool {
        let invalid_text = response
            .ctx
            .data(|data| data.get_temp::<bool>(self.rejected_id.with("invalid-text")))
            .unwrap_or(false);
        self.active |= response.dragged() || response.has_focus() || response.lost_focus();
        self.finish |= response.drag_stopped() || response.lost_focus();
        if self.cancel && self.enabled {
            response.surrender_focus();
        }
        self.enabled
            && !self.cancel
            && (response.changed()
                || (mixed
                    && self.confirm
                    && response.lost_focus()
                    && !self.rejected
                    && !invalid_text))
    }
    pub(super) fn apply(
        self,
        editor: &mut Editor,
        ui: &egui::Ui,
        assets: &mut AssetStore,
        updates: Vec<FieldUpdate>,
    ) {
        if !self.enabled {
            return;
        }
        if self.cancel {
            ui.data_mut(|data| data.remove::<bool>(self.rejected_id));
            ui.data_mut(|data| data.remove::<bool>(self.rejected_id.with("invalid-text")));
            if self.active {
                let result = editor.cancel();
                editor.notice(result);
                ui.ctx().stop_dragging();
            }
            return;
        }
        if !updates.is_empty() {
            let result = editor.edit_fields(&updates, assets);
            // A rejected text draft must not confirm the snapshot's fallback value on Enter.
            ui.data_mut(|data| {
                if result.is_err() {
                    data.insert_temp(self.rejected_id, true);
                } else {
                    data.remove::<bool>(self.rejected_id);
                }
            });
            editor.notice(result);
        }
        if self.finish {
            ui.data_mut(|data| data.remove::<bool>(self.rejected_id));
            ui.data_mut(|data| data.remove::<bool>(self.rejected_id.with("invalid-text")));
            let result = editor.commit();
            editor.notice(result);
        }
    }
}
