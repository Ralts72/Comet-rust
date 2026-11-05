use crate::session::Editor;
use comet_asset::AssetStore;
use comet_core::math::{EulerRot, Quat};
use comet_scene::EntityData;
use comet_scene::schema::{self, FieldValue};
mod animation;
mod audio;
mod joints;
mod model;
mod navigation;
mod overrides;
mod physics;
mod script;
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
                .hint_text("对象名称"),
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
    for (name, key) in [
        ("位置", schema::TRANSLATION),
        ("旋转", schema::ROTATION),
        ("缩放", schema::SCALE),
    ] {
        ui.label(name);
        let field = schema::descriptor(key).unwrap();
        let value = field.read(entity).unwrap();
        let mut vector = match value {
            FieldValue::Vec3(v) => v,
            FieldValue::Quat(q) => {
                let (x, y, z) = Quat::from_array(q).to_euler(EulerRot::XYZ);
                [x.to_degrees(), y.to_degrees(), z.to_degrees()]
            }
            _ => unreachable!(),
        };
        let mut changed = false;
        let mut finish = false;
        let animation_controlled = editor
            .play
            .as_ref()
            .is_some_and(|play| play.is_animation_controlled(entity.id, key));
        ui.add_enabled_ui(
            (editor.play.is_none() || entity.physics.is_none()) && !animation_controlled,
            |ui| {
                ui.horizontal(|ui| {
                    for (axis, v) in ["X", "Y", "Z"].into_iter().zip(&mut vector) {
                        let response = ui.add(
                            egui::DragValue::new(v)
                                .prefix(format!("{axis} "))
                                .speed(if key == schema::ROTATION { 0.5 } else { 0.01 })
                                .max_decimals(2),
                        );
                        changed |= response.changed();
                        finish |= response.drag_stopped() || response.lost_focus();
                    }
                });
            },
        );
        if animation_controlled {
            ui.weak("该字段由动画驱动，运行期间只读。");
        }
        if changed && ui.is_enabled() {
            let value = if key == schema::ROTATION {
                FieldValue::Quat(
                    Quat::from_euler(
                        EulerRot::XYZ,
                        vector[0].to_radians(),
                        vector[1].to_radians(),
                        vector[2].to_radians(),
                    )
                    .to_array(),
                )
            } else {
                FieldValue::Vec3(vector)
            };
            let result = editor.edit(entity.id, key, value, assets);
            editor.notice(result);
        }
        if finish && ui.is_enabled() {
            let result = editor.commit();
            editor.notice(result);
        }
        ui.add_space(6.);
    }
    model::draw(editor, ui, assets, entity);
    physics::draw(editor, ui, assets, entity);
    joints::draw(editor, ui, assets, entity);
    navigation::draw(editor, ui, assets, entity);
    audio::draw(editor, ui, assets, entity);
    script::draw(editor, ui, assets, entity);
    animation::draw(editor, ui, assets, entity);
    overrides::draw(editor, ui, assets, entity.id);
}
