use crate::DrawTransform;
use comet_asset_data::product::ProductFile;
use comet_core::math::{Mat4, Vec3};
use comet_core::{EntityId, WorldId};
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ViewToken {
    pub world: WorldId,
    pub view: u64,
    pub target_generation: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: Vec3::new(5., 3.5, 7.),
            target: Vec3::new(0., 0.7, 0.),
        }
    }
}
impl Camera {
    pub fn view_projection(self, aspect: f32) -> Mat4 {
        comet_core::math::perspective(45_f32.to_radians(), aspect.max(0.01), 0.05, 200.)
            * comet_core::math::look_at(self.eye, self.target, Vec3::Y)
    }
    pub fn light_projection() -> Mat4 {
        comet_core::math::orthographic(-7., 7., -7., 7., 0.1, 30.)
            * comet_core::math::look_at(Vec3::new(6., 10., 5.), Vec3::ZERO, Vec3::Y)
    }
}
#[derive(Clone)]
pub struct Draw {
    pub entity: EntityId,
    pub transform: DrawTransform,
    pub skin: Option<Arc<crate::SkinPalette>>,
    pub mesh: Arc<ProductFile>,
    pub material: Arc<ProductFile>,
    pub texture: Option<Arc<ProductFile>>,
    pub mesh_revision: String,
    pub texture_revision: Option<String>,
    pub roughness: f32,
    pub selected: bool,
}
#[derive(Default)]
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    pub pixels_per_point: f32,
}
impl Drop for UiFrame {
    fn drop(&mut self) {
        // The backend has applied these updates, or shutdown intentionally retires the queued frame.
        // epaint requires explicit acknowledgement when owned deltas are released.
        self.textures.clear();
    }
}
/// Coordinates are relative to the viewport, normalized to [0, 1] with a top-left origin.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum PickRequest {
    Point([f32; 2]),
    Rect { min: [f32; 2], max: [f32; 2] },
}
impl PickRequest {
    fn validate(self) -> anyhow::Result<()> {
        let valid = |point: [f32; 2]| {
            point
                .into_iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(&v))
        };
        anyhow::ensure!(
            match self {
                Self::Point(point) => valid(point),
                Self::Rect { min, max } => {
                    valid(min) && valid(max) && min[0] <= max[0] && min[1] <= max[1]
                }
            },
            "invalid viewport pick coordinates"
        );
        Ok(())
    }
}
pub struct FramePacket {
    pub number: u64,
    pub token: ViewToken,
    pub size: [u32; 2],
    pub viewport: [u32; 4],
    pub camera: Camera,
    pub draws: Arc<[Draw]>,
    pub ui: UiFrame,
    pub pick: Option<PickRequest>,
    pub capture: Option<PathBuf>,
    pub shadows: bool,
}
impl FramePacket {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.size.iter().all(|v| *v <= 16384),
            "frame exceeds supported target size limit"
        );
        let [x, y, w, h] = self.viewport;
        anyhow::ensure!(
            x.checked_add(w).is_some_and(|v| v <= self.size[0])
                && y.checked_add(h).is_some_and(|v| v <= self.size[1]),
            "viewport exceeds target"
        );
        anyhow::ensure!(
            self.camera.eye.is_finite()
                && self.camera.target.is_finite()
                && self.camera.eye.distance_squared(self.camera.target) > 0.0001,
            "invalid view camera"
        );
        anyhow::ensure!(
            (self.camera.eye - self.camera.target)
                .normalize()
                .dot(Vec3::Y)
                .abs()
                < 0.9999,
            "view camera is parallel to its up axis"
        );
        if let Some(request) = self.pick {
            request.validate()?;
        }
        for draw in self.draws.iter() {
            if let Some(skin) = &draw.skin {
                anyhow::ensure!(
                    skin.matches_mesh(&draw.mesh) && draw.transform.model() == Mat4::IDENTITY,
                    "skin pose does not belong to this mesh or uses a second model transform"
                );
            }
            anyhow::ensure!(
                draw.roughness.is_finite() && (0. ..=1.).contains(&draw.roughness),
                "invalid object roughness"
            );
        }
        for p in &self.ui.primitives {
            match &p.primitive {
                egui::epaint::Primitive::Mesh(mesh) => {
                    anyhow::ensure!(mesh.is_valid(), "invalid UI mesh")
                }
                egui::epaint::Primitive::Callback(_) => {
                    anyhow::bail!("native UI callbacks are not supported")
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct PickFeedback {
    pub token: ViewToken,
    pub entities: Vec<EntityId>,
}
#[derive(Clone, Debug, Serialize)]
pub struct FrameFeedback {
    pub number: u64,
    pub token: ViewToken,
    pub presented: bool,
    pub backend: String,
    pub worker_thread: String,
    pub draw_count: usize,
    pub gpu_ms: f64,
    pub pick: Option<PickFeedback>,
}
#[cfg(test)]
mod tests;
