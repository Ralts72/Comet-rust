use crate::{Camera, Projection};
use comet_core::math::{Quat, Vec3};

/// The caller authorizes the viewport; pan is an editor gesture, separate from gameplay input.
pub fn navigate_camera(
    camera: &mut Camera,
    response: &egui::Response,
    active: bool,
    allow_pan: bool,
    look_delta: [f32; 2],
) {
    if !active || !response.enabled() || !response.ctx.input(|input| input.focused) {
        return;
    }
    let mouse_rotating = response.dragged_by(egui::PointerButton::Secondary);
    let rotating =
        (mouse_rotating || look_delta != [0.; 2]) && camera.projection == Projection::Perspective;
    let panning = allow_pan && response.dragged_by(egui::PointerButton::Middle);
    let hovered = response.hovered();
    let (delta, scroll) = response.ctx.input(|input| {
        (
            input.pointer.delta(),
            if hovered {
                input.smooth_scroll_delta.y
            } else {
                0.
            },
        )
    });
    let rotation_delta = if mouse_rotating {
        delta
    } else {
        egui::Vec2::ZERO
    } + egui::Vec2::from(look_delta);
    if !rotation_delta.is_finite()
        || !delta.is_finite()
        || !scroll.is_finite()
        || (!rotating && !panning && scroll == 0.)
        || camera.validate().is_err()
    {
        return;
    }
    let mut next = *camera;
    let mut offset = next.eye - next.target;
    if rotating {
        offset = Quat::from_rotation_y(-rotation_delta.x * 0.006) * offset;
        let right = offset.cross(Vec3::Y).normalize_or_zero();
        let pitched = Quat::from_axis_angle(right, rotation_delta.y * 0.006) * offset;
        if pitched.normalize().dot(Vec3::Y).abs() < 0.96 {
            offset = pitched;
        }
        next.eye = next.target + offset;
    }
    if panning && response.rect.height() > 0. {
        let forward = -offset.normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward);
        let height = next.visible_height();
        let translation = (-right * delta.x + up * delta.y) * (height / response.rect.height());
        next.eye += translation;
        next.target += translation;
    }
    if scroll != 0. {
        let scale = (-f64::from(scroll) * 0.002).exp().clamp(0.8, 1.2);
        match next.projection {
            Projection::Perspective => {
                let margin =
                    (f64::from(next.far) - f64::from(next.near)).min(f64::from(next.near)) * 0.001;
                let distance = (offset.as_dvec3().length() * scale)
                    .clamp(f64::from(next.near) + margin, f64::from(next.far) - margin);
                next.eye =
                    (next.target.as_dvec3() + offset.as_dvec3().normalize() * distance).as_vec3();
            }
            Projection::Orthographic { height } => {
                next.projection = Projection::Orthographic {
                    height: (f64::from(height) * scale)
                        .clamp(f64::from(f32::MIN_POSITIVE), f64::from(f32::MAX))
                        as f32,
                };
            }
        }
    }
    if next.validate().is_ok() {
        *camera = next;
    }
}

#[cfg(test)]
mod tests;
