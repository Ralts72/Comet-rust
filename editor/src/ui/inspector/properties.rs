use crate::session::Editor;
use comet_asset::AssetStore;
use comet_scene::{
    EntityData, FieldUpdate,
    schema::{self, FieldKey, FieldValue},
};

/// Component panels collect input completion; each panel owns candidate validation and publication.
#[derive(Default)]
pub(super) struct ComponentEdit {
    pub(super) finish: bool,
}
impl ComponentEdit {
    pub(super) fn number(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        value: &mut f32,
        min: f32,
        max: f32,
    ) {
        let response = ui.add(
            egui::DragValue::new(value)
                .prefix(format!("{label} "))
                .range(min..=max)
                .speed(0.05)
                .max_decimals(3),
        );
        self.finish |= response.drag_stopped() || response.lost_focus();
    }
}

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
        self.numeric(ui, widget, false, false)
    }
    pub(super) fn number64(
        &self,
        ui: &mut egui::Ui,
        widget: egui::DragValue<'_>,
    ) -> egui::Response {
        self.numeric(ui, widget, false, true)
    }
    pub(super) fn mask(&self, ui: &mut egui::Ui, value: &mut u32, label: &str) -> egui::Response {
        self.numeric(
            ui,
            egui::DragValue::new(value)
                .prefix(format!("{label} "))
                .hexadecimal(8, false, true),
            true,
            false,
        )
    }
    fn numeric(
        &self,
        ui: &mut egui::Ui,
        widget: egui::DragValue<'_>,
        hexadecimal: bool,
        precise: bool,
    ) -> egui::Response {
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
                        let parsed = if hexadecimal {
                            u32::from_str_radix(
                                text.strip_prefix("0x")
                                    .or_else(|| text.strip_prefix("0X"))
                                    .unwrap_or(&text),
                                16,
                            )
                            .ok()
                            .map(f64::from)
                        } else {
                            text.parse::<f64>().ok()
                        };
                        parser_context
                            .data_mut(|data| data.insert_temp(invalid_text_id, parsed.is_none()));
                        parsed
                    })
                    .custom_formatter(move |value, precision| {
                        // DragValue parses its text on focus loss. Display rounding must not edit data.
                        if hexadecimal {
                            format!("{:08X}", value as u32)
                        } else if context.memory(|memory| memory.has_focus(id)) {
                            if precise {
                                value.to_string()
                            } else {
                                (value as f32).to_string()
                            }
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
            // Earlier pickers may consume Escape; raw intent must also cancel a stopped drag.
            cancel: ui.input(|input| {
                input.raw.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            pressed: true,
                            ..
                        }
                    )
                })
            }),
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
        self.active |= response.dragged()
            || response.drag_stopped()
            || response.has_focus()
            || response.lost_focus();
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
