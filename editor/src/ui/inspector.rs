use super::*;
use comet_scene::schema::{self, FieldValue};
use glam::{EulerRot, Quat};
pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entity: &EntityData,
) {
    ui.strong(&entity.name);
    ui.label(egui::RichText::new(entity.id.to_string()).size(10.).weak());
    ui.separator();
    for (name, key) in [
        ("Position", schema::TRANSLATION),
        ("Rotation", schema::ROTATION),
        ("Scale", schema::SCALE),
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
        if changed {
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
        if finish {
            let result = editor.commit();
            editor.notice(result);
        }
        ui.add_space(6.);
    }
    if let Some(model) = &entity.model {
        ui.separator();
        ui.strong("Surface");
        ui.label(
            egui::RichText::new(format!("Material {}", model.surface.material))
                .size(10.)
                .weak(),
        );
        let mut value = model.surface.roughness;
        let response = ui.add(egui::Slider::new(&mut value, 0.05..=1.).text("Roughness"));
        if response.changed() {
            let result = editor.edit(
                entity.id,
                schema::ROUGHNESS,
                FieldValue::Scalar(value),
                assets,
            );
            editor.notice(result);
        }
        if response.drag_stopped() || response.lost_focus() {
            let result = editor.commit();
            editor.notice(result);
        }
    }
}
