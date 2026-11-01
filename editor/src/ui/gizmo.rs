use super::*;
use comet_scene::schema::{self, FieldValue};
use egui::{Pos2, Stroke};
use glam::Vec3;
pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    viewport: Rect,
) -> bool {
    let Some(entity) = editor.selected_entity() else {
        return false;
    };
    let Ok(transforms) = comet_application::world_transforms(&editor.last_snapshot) else {
        return false;
    };
    let world = transforms[&entity.id];
    let origin = world.transform_point3(Vec3::ZERO);
    let vp = editor.camera.view_projection(viewport.aspect_ratio());
    let project = |position: Vec3| -> Option<Pos2> {
        let clip = vp * position.extend(1.);
        if clip.w <= 0. {
            return None;
        }
        let p = clip.truncate() / clip.w;
        Some(Pos2::new(
            viewport.center().x + p.x * viewport.width() * 0.5,
            viewport.center().y - p.y * viewport.height() * 0.5,
        ))
    };
    let Some(start) = project(origin) else {
        return false;
    };
    let mut active = false;
    let parent = entity
        .parent
        .and_then(|id| transforms.get(&id))
        .copied()
        .unwrap_or(glam::Mat4::IDENTITY);
    if parent.determinant().abs() < 1e-8 {
        return false;
    }
    for (axis, color, label) in [
        (Vec3::X, Color32::from_rgb(240, 101, 108), "X"),
        (Vec3::Y, Color32::from_rgb(122, 211, 144), "Y"),
        (Vec3::Z, Color32::from_rgb(109, 165, 244), "Z"),
    ] {
        let Some(end) = project(origin + axis * 0.9) else {
            continue;
        };
        let direction = end - start;
        let length = direction.length();
        if length < 5. {
            continue;
        }
        let response = ui.interact(
            Rect::from_two_pos(start + direction * 0.25, end).expand(7.),
            ui.scope_id().with(("gizmo", label)),
            egui::Sense::drag(),
        );
        let hovered = response.hovered() || response.dragged();
        active |= hovered;
        ui.painter().arrow(
            start,
            direction,
            Stroke::new(if hovered { 4. } else { 2.5 }, color),
        );
        ui.painter().text(
            end + direction.normalized() * 10.,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.),
            color,
        );
        if response.drag_started() {
            editor.drag_origin = Some(entity.transform.translation);
        }
        if response.dragged()
            && let Some(initial) = editor.drag_origin
        {
            let initial_world = parent.transform_point3(Vec3::from_array(initial));
            let Some(initial_start) = project(initial_world) else {
                continue;
            };
            let Some(initial_end) = project(initial_world + axis * 0.9) else {
                continue;
            };
            let initial_axis = initial_end - initial_start;
            let amount = response
                .total_drag_delta()
                .unwrap_or_default()
                .dot(initial_axis)
                / initial_axis.length_sq()
                * 0.9;
            let local =
                Vec3::from_array(initial) + parent.inverse().transform_vector3(axis) * amount;
            let result = editor.edit(
                entity.id,
                schema::TRANSLATION,
                FieldValue::Vec3(local.to_array()),
                assets,
            );
            editor.notice(result);
        }
        if response.drag_stopped() {
            editor.drag_origin = None;
            let result = editor.commit();
            editor.notice(result);
        }
    }
    active
}
