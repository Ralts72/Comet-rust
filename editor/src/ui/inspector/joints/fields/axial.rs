use super::{
    assign, editable, keys,
    numbers::{Number, Part, number},
};
use crate::session::Editor;
use comet_asset::AssetStore;
use comet_runtime::JointKind;
use comet_scene::{
    EntityData, JointLimits, JointMotor,
    schema::{self, FieldValue},
};

fn mode(value: &Option<JointMotor>) -> usize {
    match value {
        None => 0,
        Some(JointMotor::Velocity { .. }) => 1,
        Some(JointMotor::Position { .. }) => 2,
    }
}
pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&EntityData],
    kind: JointKind,
) {
    let fields = keys(kind);
    let angular = kind == JointKind::Hinge;
    let enabled = editable(editor, ui);
    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let limits = schema::descriptor(fields[7]).unwrap();
    let FieldValue::JointLimits(first) = limits.read(entities[0]).unwrap() else {
        unreachable!()
    };
    let mut active = first.is_some();
    let mixed = entities.iter().any(|entity| matches!(limits.read(entity), Some(FieldValue::JointLimits(value)) if value.is_some() != active));
    let label = if angular {
        "角度限位"
    } else {
        "行程限位"
    };
    if ui
        .add_enabled(
            enabled,
            egui::Checkbox::new(
                &mut active,
                if mixed {
                    format!("{label}（多值）")
                } else {
                    label.into()
                },
            )
            .indeterminate(mixed),
        )
        .changed()
        && !cancel
    {
        assign(editor, assets, entities, fields[7], |value| {
            let FieldValue::JointLimits(old) = value else {
                unreachable!()
            };
            FieldValue::JointLimits(if active {
                Some(old.unwrap_or(JointLimits {
                    min: if angular {
                        -std::f32::consts::FRAC_PI_2
                    } else {
                        -1.
                    },
                    max: if angular {
                        std::f32::consts::FRAC_PI_2
                    } else {
                        1.
                    },
                }))
            } else {
                None
            })
        });
        return;
    }
    if !mixed && active {
        if angular {
            ui.weak("铰链限位跨度不超过 360°；上下限相同会锁定该轴。");
        }
        for (part, label) in [
            (
                Part::Minimum,
                if angular {
                    "最小角度 (°)"
                } else {
                    "最小行程 (m)"
                },
            ),
            (
                Part::Maximum,
                if angular {
                    "最大角度 (°)"
                } else {
                    "最大行程 (m)"
                },
            ),
        ] {
            number(
                editor,
                ui,
                assets,
                entities,
                Number {
                    field: fields[7],
                    part,
                    label,
                    angular,
                    min: f32::MIN,
                },
            );
        }
    }
    let motor = schema::descriptor(fields[8]).unwrap();
    let FieldValue::JointMotor(first) = motor.read(entities[0]).unwrap() else {
        unreachable!()
    };
    let current = mode(&first);
    let mixed = entities.iter().any(|entity| matches!(motor.read(entity), Some(FieldValue::JointMotor(value)) if mode(&value) != current));
    let mut selected = None;
    let names = ["关闭驱动", "速度驱动", "位置驱动"];
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label(if mixed {
                "驱动模式（多值）"
            } else {
                "驱动模式"
            });
            egui::ComboBox::from_id_salt("motor")
                .selected_text(if mixed { "多值" } else { names[current] })
                .show_ui(ui, |ui| {
                    for (index, name) in names.into_iter().enumerate() {
                        if ui
                            .selectable_label(!mixed && index == current, name)
                            .clicked()
                        {
                            selected = Some(index);
                        }
                    }
                });
        })
    });
    if let Some(next) = selected.filter(|_| enabled && !cancel) {
        assign(editor, assets, entities, fields[8], |value| {
            let FieldValue::JointMotor(old) = value else {
                unreachable!()
            };
            FieldValue::JointMotor(if mode(&old) == next {
                old
            } else {
                match next {
                    1 => Some(JointMotor::Velocity {
                        speed: 0.,
                        max_effort: 10.,
                    }),
                    2 => Some(JointMotor::Position {
                        target: 0.,
                        frequency_hz: 2.,
                        damping_ratio: 1.,
                        max_effort: 10.,
                    }),
                    _ => None,
                }
            })
        });
        return;
    }
    if mixed || current == 0 {
        return;
    }
    let mut draw = |part, label, angular, min| {
        number(
            editor,
            ui,
            assets,
            entities,
            Number {
                field: fields[8],
                part,
                label,
                angular,
                min,
            },
        )
    };
    draw(
        if current == 1 {
            Part::Speed
        } else {
            Part::Target
        },
        match (angular, current) {
            (true, 1) => "目标角速度 (°/s)",
            (true, _) => "目标角度 (°)",
            (false, 1) => "目标速度 (m/s)",
            _ => "目标行程 (m)",
        },
        angular,
        f32::MIN,
    );
    if current == 2 {
        draw(Part::Frequency, "驱动频率 (Hz)", false, 0.001);
        draw(Part::Damping, "驱动阻尼比", false, 0.);
    }
    draw(
        Part::Effort,
        if angular {
            "最大力矩 (N·m)"
        } else {
            "最大驱动力 (N)"
        },
        false,
        0.,
    );
}
