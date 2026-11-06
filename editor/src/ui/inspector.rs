use crate::session::Editor;
use comet_asset::AssetStore;
use comet_scene::{EntityData, FieldUpdate};
mod animation;
mod audio;
pub(super) mod common;
mod joints;
mod model;
mod navigation;
mod overrides;
mod physics;
mod script;
pub(super) mod transform;

struct PropertyGesture {
    rejected_id: egui::Id,
    rejected: bool,
    enabled: bool,
    cancel: bool,
    confirm: bool,
    active: bool,
    finish: bool,
}
impl PropertyGesture {
    fn number(&self, ui: &mut egui::Ui, widget: egui::DragValue<'_>) -> egui::Response {
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
    fn new(ui: &egui::Ui, enabled: bool, field: comet_scene::schema::FieldKey) -> Self {
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
    fn observe(&mut self, response: &egui::Response, mixed: bool) -> bool {
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
    fn apply(
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
pub(super) fn clear_model_draft(ctx: &egui::Context) {
    model::clear_draft(ctx);
}

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    let authored = editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .any(|e| e.id == entity.id);
    if authored {
        let mut name = entity.name.clone();
        // TextEdit consumes Escape as focus loss; preserve its cancellation intent.
        let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
        let response = ui.add_enabled(
            editor.play.is_none(),
            egui::TextEdit::singleline(&mut name)
                .id_salt(("entity-name", editor.world(), entity.id))
                .hint_text("名称"),
        );
        if ui.is_enabled() && response.changed() && !cancel {
            let result = editor.rename_object(entity.id, name);
            editor.notice(result);
        }
        if ui.is_enabled() && response.lost_focus() {
            let result = if cancel {
                editor.cancel()
            } else {
                editor.commit()
            };
            editor.notice(result);
        }
    } else {
        ui.strong(&entity.name);
    }
    ui.label(egui::RichText::new(entity.id.to_string()).size(10.).weak());
    ui.separator();
    transform::draw(editor, ui, assets, &[entity]);
    model::draw(editor, ui, assets, entity);
    physics::draw(editor, ui, assets, entity);
    joints::draw(editor, ui, assets, entity);
    navigation::draw(editor, ui, assets, entity);
    audio::draw(editor, ui, assets, entity);
    script::draw(editor, ui, assets, entity);
    animation::draw(editor, ui, assets, entity);
    overrides::draw(editor, ui, assets, entity.id);
}
