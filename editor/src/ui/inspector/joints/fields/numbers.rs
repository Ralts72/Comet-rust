use super::editable;
use crate::{session::Editor, ui::inspector::properties::PropertyGesture};
use comet_asset::AssetStore;
use comet_core::math::{EulerRot, Quat};
use comet_scene::{
    EntityData, FieldUpdate, JointMotor,
    schema::{self, FieldKey, FieldValue},
};

#[derive(Clone, Copy)]
pub(super) enum Part {
    Axis(usize),
    Minimum,
    Maximum,
    Speed,
    Target,
    Frequency,
    Damping,
    Effort,
}
impl Part {
    fn read(self, value: &FieldValue) -> f32 {
        match (self, value) {
            (Self::Axis(axis), FieldValue::Vec3(value)) => value[axis],
            (Self::Axis(axis), FieldValue::Quat(value)) => {
                let (x, y, z) = Quat::from_array(*value).to_euler(EulerRot::XYZ);
                [x, y, z][axis].to_degrees()
            }
            (Self::Minimum, FieldValue::JointLimits(Some(value))) => value.min,
            (Self::Maximum, FieldValue::JointLimits(Some(value))) => value.max,
            (Self::Speed, FieldValue::JointMotor(Some(JointMotor::Velocity { speed, .. }))) => {
                *speed
            }
            (Self::Target, FieldValue::JointMotor(Some(JointMotor::Position { target, .. }))) => {
                *target
            }
            (
                Self::Frequency,
                FieldValue::JointMotor(Some(JointMotor::Position { frequency_hz, .. })),
            ) => *frequency_hz,
            (
                Self::Damping,
                FieldValue::JointMotor(Some(JointMotor::Position { damping_ratio, .. })),
            ) => *damping_ratio,
            (
                Self::Effort,
                FieldValue::JointMotor(Some(
                    JointMotor::Velocity { max_effort, .. }
                    | JointMotor::Position { max_effort, .. },
                )),
            ) => *max_effort,
            _ => unreachable!("joint numeric control requires a common value variant"),
        }
    }
    fn write(self, value: &mut FieldValue, next: f32) {
        match (self, value) {
            (Self::Axis(axis), FieldValue::Vec3(value)) => value[axis] = next,
            (Self::Axis(axis), FieldValue::Quat(value)) => {
                let (x, y, z) = Quat::from_array(*value).to_euler(EulerRot::XYZ);
                let mut degrees = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
                degrees[axis] = next;
                *value = Quat::from_euler(
                    EulerRot::XYZ,
                    degrees[0].to_radians(),
                    degrees[1].to_radians(),
                    degrees[2].to_radians(),
                )
                .to_array();
            }
            (Self::Minimum, FieldValue::JointLimits(Some(value))) => value.min = next,
            (Self::Maximum, FieldValue::JointLimits(Some(value))) => value.max = next,
            (Self::Speed, FieldValue::JointMotor(Some(JointMotor::Velocity { speed, .. }))) => {
                *speed = next
            }
            (Self::Target, FieldValue::JointMotor(Some(JointMotor::Position { target, .. }))) => {
                *target = next
            }
            (
                Self::Frequency,
                FieldValue::JointMotor(Some(JointMotor::Position { frequency_hz, .. })),
            ) => *frequency_hz = next,
            (
                Self::Damping,
                FieldValue::JointMotor(Some(JointMotor::Position { damping_ratio, .. })),
            ) => *damping_ratio = next,
            (
                Self::Effort,
                FieldValue::JointMotor(Some(
                    JointMotor::Velocity { max_effort, .. }
                    | JointMotor::Position { max_effort, .. },
                )),
            ) => *max_effort = next,
            _ => unreachable!("joint numeric control requires a common value variant"),
        }
    }
}
pub(super) struct Number {
    pub(super) field: FieldKey,
    pub(super) part: Part,
    pub(super) label: &'static str,
    pub(super) angular: bool,
    pub(super) min: f32,
}
pub(super) fn number(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    property: Number,
) {
    let descriptor = schema::descriptor(property.field).unwrap();
    let first = descriptor.read(entities[0]).unwrap();
    let mut value = f64::from(property.part.read(&first));
    let mixed = entities
        .iter()
        .skip(1)
        .any(|entity| f64::from(property.part.read(&descriptor.read(entity).unwrap())) != value);
    if property.angular {
        value = value.to_degrees();
    }
    ui.push_id(
        (
            property.field.component,
            property.field.field,
            property.label,
        ),
        |ui| {
            let mut gesture = PropertyGesture::new(ui, editable(editor, ui), property.field);
            let label = if mixed {
                format!("{}（多值）", property.label)
            } else {
                property.label.into()
            };
            let maximum = if property.angular {
                f64::from(f32::MAX).to_degrees()
            } else {
                f64::from(f32::MAX)
            };
            let minimum = if property.angular {
                f64::from(property.min).to_degrees()
            } else {
                f64::from(property.min)
            };
            let minimum = minimum.min(value);
            let response = gesture.number64(
                ui,
                egui::DragValue::new(&mut value)
                    .prefix(format!("{label} "))
                    .range(minimum..=maximum)
                    .clamp_existing_to_range(false)
                    .speed(if property.angular { 0.5 } else { 0.05 })
                    .max_decimals(3),
            );
            let updates = if gesture.observe(&response, mixed) {
                let next = if property.angular {
                    value.to_radians() as f32
                } else {
                    value as f32
                };
                entities
                    .iter()
                    .map(|entity| {
                        let mut value = descriptor.read(entity).unwrap();
                        property.part.write(&mut value, next);
                        FieldUpdate {
                            entity: entity.id,
                            field: property.field,
                            value,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            gesture.apply(editor, ui, assets, updates);
        },
    );
}
pub(super) fn vector(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    field: FieldKey,
    label: &str,
) {
    ui.push_id((field.component, field.field), |ui| {
        ui.label(label);
        ui.horizontal(|ui| {
            for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                number(
                    editor,
                    ui,
                    assets,
                    entities,
                    Number {
                        field,
                        part: Part::Axis(axis),
                        label,
                        angular: false,
                        min: f32::MIN,
                    },
                );
            }
        });
    });
}
