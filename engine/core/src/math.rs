//! Shared numerical types and engine conventions. Backend-specific conversions stay in adapters.
pub use glam::{EulerRot, Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// Right-handed, Y-up, local forward -Z; clip depth is [0, 1].
pub fn perspective(fov_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective(fov_radians, aspect, near, far)
}
pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    glam::camera::rh::proj::directx::orthographic(left, right, bottom, top, near, far)
}
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    glam::camera::rh::view::look_at_mat4(eye, target, up)
}
