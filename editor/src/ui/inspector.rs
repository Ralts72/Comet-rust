use super::*;
use comet_core::math::{EulerRot, Quat};
use comet_scene::schema::{self, FieldValue};
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
        ui.add_enabled_ui(editor.play.is_none() || entity.physics.is_none(), |ui| {
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
        ui.strong("表面");
        ui.label(
            egui::RichText::new(format!("材质 {}", model.surface.material))
                .size(10.)
                .weak(),
        );
        let mut value = model.surface.roughness;
        let response = ui.add(egui::Slider::new(&mut value, 0.05..=1.).text("粗糙度"));
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
    if let Some(body) = &entity.physics {
        ui.separator();
        ui.strong("物理");
        let motion = match body.motion {
            comet_scene::BodyMotion::Fixed => "静态刚体",
            comet_scene::BodyMotion::Dynamic => "动态刚体",
            comet_scene::BodyMotion::Kinematic => "运动学刚体",
        };
        ui.label(motion);
        ui.label(match body.shape {
            comet_scene::ColliderShape::Box { half_extents } => format!(
                "长方体 · 半尺寸 {:.2} / {:.2} / {:.2}",
                half_extents[0], half_extents[1], half_extents[2]
            ),
            comet_scene::ColliderShape::Sphere { radius } => format!("球体 · 半径 {radius:.2}"),
            comet_scene::ColliderShape::Capsule {
                half_height,
                radius,
            } => format!("胶囊体 · 半高 {half_height:.2} · 半径 {radius:.2}"),
        });
        ui.label(format!(
            "碰撞层 {:08x} / 掩码 {:08x}",
            body.layer, body.mask
        ));
    }
    for (label, key) in [
        ("质量", schema::MASS),
        ("摩擦系数", schema::FRICTION),
        ("移动速度", schema::MOVE_SPEED),
        ("跳跃速度", schema::JUMP_SPEED),
    ] {
        let Some(FieldValue::Scalar(mut value)) = schema::descriptor(key).unwrap().read(entity)
        else {
            continue;
        };
        let response = ui.add_enabled(
            editor.play.is_none(),
            egui::DragValue::new(&mut value)
                .prefix(format!("{label} "))
                .speed(0.05),
        );
        if response.changed() {
            let result = editor.edit(entity.id, key, FieldValue::Scalar(value), assets);
            editor.notice(result);
        }
        if response.drag_stopped() || response.lost_focus() {
            let result = editor.commit();
            editor.notice(result);
        }
    }
}
