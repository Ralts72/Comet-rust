use super::properties::PropertyGesture;
use crate::session::Editor;
use comet_asset::AssetStore;
use comet_core::math::{EulerRot, Quat};
use comet_scene::{
    EntityData, FieldUpdate,
    schema::{self, FieldKey, FieldValue},
};

pub(in crate::ui) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
) {
    let Some(primary) = entities.first() else {
        return;
    };
    let grouped = entities.len() > 1;
    // A selection change must retire the previous selection's focused/dragged widget.
    ui.push_id(
        (
            "transform-properties",
            editor.world(),
            primary.id,
            editor.selection.generation(),
        ),
        |ui| {
            for (name, key) in [
                ("位置", schema::TRANSLATION),
                ("旋转", schema::ROTATION),
                ("缩放", schema::SCALE),
            ] {
                let mut vector = axes(primary, key);
                let mut mixed = [false; 3];
                for entity in entities.iter().skip(1) {
                    let other = axes(entity, key);
                    for axis in 0..3 {
                        mixed[axis] |= other[axis] != vector[axis];
                    }
                    if mixed.iter().all(|value| *value) {
                        break;
                    }
                }
                ui.label(if mixed.iter().any(|value| *value) {
                    format!("{name}（多值）")
                } else {
                    name.into()
                });
                let mut changed = [false; 3];
                let animation_controlled = editor
                    .play
                    .as_ref()
                    .is_some_and(|play| play.is_animation_controlled(primary.id, key));
                let editable = if grouped {
                    editor.play.is_none()
                } else {
                    (editor.play.is_none() || primary.physics.is_none()) && !animation_controlled
                };
                let mut gesture = PropertyGesture::new(ui, editable, key);
                ui.add_enabled_ui(editable, |ui| {
                    ui.horizontal(|ui| {
                        for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                            let response = gesture.number(
                                ui,
                                egui::DragValue::new(&mut vector[index])
                                    .prefix(format!("{axis} "))
                                    .speed(if key == schema::ROTATION { 0.5 } else { 0.01 })
                                    .max_decimals(2),
                            );
                            changed[index] = gesture.observe(&response, mixed[index]);
                        }
                    });
                });
                if animation_controlled {
                    ui.weak("该字段由动画驱动，运行期间只读。");
                }
                let updates = if changed.iter().any(|changed| *changed) {
                    entities
                        .iter()
                        .map(|entity| {
                            let mut current = axes(entity, key);
                            // Inspector axes are absolute local values, including parent/child selections.
                            for index in 0..3 {
                                if changed[index] {
                                    current[index] = vector[index];
                                }
                            }
                            FieldUpdate {
                                entity: entity.id,
                                field: key,
                                value: field_value(key, current),
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                gesture.apply(editor, ui, assets, updates);
                ui.add_space(6.);
            }
        },
    );
}

fn axes(entity: &EntityData, key: FieldKey) -> [f32; 3] {
    match key {
        schema::TRANSLATION => entity.transform.translation,
        schema::SCALE => entity.transform.scale,
        schema::ROTATION => {
            let (x, y, z) = Quat::from_array(entity.transform.rotation).to_euler(EulerRot::XYZ);
            [x.to_degrees(), y.to_degrees(), z.to_degrees()]
        }
        _ => unreachable!("transform property"),
    }
}

fn field_value(key: FieldKey, vector: [f32; 3]) -> FieldValue {
    if key == schema::ROTATION {
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
    }
}
