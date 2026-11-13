use crate::session::Editor;
use comet_asset::AssetStore;
use comet_scene::EntityData;
mod animation;
mod assets;
mod audio;
pub(super) mod common;
mod components;
mod joints;
mod model;
mod navigation;
mod overrides;
mod physics;
mod properties;
mod references;
mod script;
pub(super) mod transform;

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
    components::draw(editor, ui, assets, &[entity]);
}
