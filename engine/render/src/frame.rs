use comet_asset_data::product::ProductFile;
use comet_core::{EntityId, WorldId};
use glam::{Mat4, Vec3};
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
        glam::camera::rh::proj::directx::perspective(
            45_f32.to_radians(),
            aspect.max(0.01),
            0.05,
            200.,
        ) * glam::camera::rh::view::look_at_mat4(self.eye, self.target, Vec3::Y)
    }
    pub fn light_projection() -> Mat4 {
        glam::camera::rh::proj::directx::orthographic(-7., 7., -7., 7., 0.1, 30.)
            * glam::camera::rh::view::look_at_mat4(Vec3::new(6., 10., 5.), Vec3::ZERO, Vec3::Y)
    }
}
#[derive(Clone)]
pub struct Draw {
    pub entity: EntityId,
    pub transform: Mat4,
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
pub struct FramePacket {
    pub number: u64,
    pub token: ViewToken,
    pub size: [u32; 2],
    pub viewport: [u32; 4],
    pub camera: Camera,
    pub draws: Vec<Draw>,
    pub ui: UiFrame,
    pub pick: Option<[f32; 2]>,
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
        for draw in &self.draws {
            anyhow::ensure!(
                draw.transform.is_finite() && draw.transform.determinant().abs() > 1e-8,
                "invalid object transform"
            );
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
    pub entity: Option<EntityId>,
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
pub(crate) fn pick(frame: &FramePacket) -> Option<PickFeedback> {
    let uv = frame.pick?;
    let aspect = frame.viewport[2] as f32 / frame.viewport[3].max(1) as f32;
    let inverse = frame.camera.view_projection(aspect).inverse();
    let far = inverse.project_point3(Vec3::new(uv[0] * 2. - 1., 1. - uv[1] * 2., 1.));
    let origin = frame.camera.eye;
    let direction = (far - origin).normalize();
    let mut closest = f32::INFINITY;
    let mut entity = None;
    for draw in &frame.draws {
        let comet_asset_data::product::Product::Mesh(mesh) = &draw.mesh.payload else {
            continue;
        };
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = triangle.map(|i| {
                draw.transform
                    .transform_point3(Vec3::from_array(mesh.positions[i as usize]))
            });
            let ab = b - a;
            let ac = c - a;
            let p = direction.cross(ac);
            let determinant = ab.dot(p);
            if determinant.abs() < 1e-7 {
                continue;
            }
            let inv = 1. / determinant;
            let from = origin - a;
            let u = from.dot(p) * inv;
            let q = from.cross(ab);
            let v = direction.dot(q) * inv;
            let t = ac.dot(q) * inv;
            if u >= 0. && v >= 0. && u + v <= 1. && t > 0. && t < closest {
                closest = t;
                entity = Some(draw.entity);
            }
        }
    }
    Some(PickFeedback {
        token: frame.token,
        entity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use comet_asset_data::product::{MaterialData, MeshData, Product, ProductFile};
    use comet_core::AssetId;
    #[test]
    fn triangle_pick_uses_world_transforms_and_returns_the_nearest_entity() {
        let mesh = Arc::new(ProductFile {
            version: 1,
            id: AssetId::new(),
            source_digest: "0".repeat(64),
            payload: Product::Mesh(MeshData {
                version: 1,
                positions: vec![[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]],
                normals: vec![[0., 0., 1.]; 3],
                uvs: vec![[0.; 2]; 3],
                indices: vec![0, 1, 2],
            }),
        });
        let material = Arc::new(ProductFile {
            version: 1,
            id: AssetId::new(),
            source_digest: "0".repeat(64),
            payload: Product::Material(MaterialData {
                version: 1,
                base_color: [1.; 4],
                roughness: 0.5,
                metallic: 0.,
                base_color_texture: None,
            }),
        });
        let far = Draw {
            entity: EntityId::new(),
            transform: Mat4::IDENTITY,
            mesh,
            material,
            texture: None,
            mesh_revision: "revision".into(),
            texture_revision: None,
            roughness: 0.5,
            selected: false,
        };
        let mut near = far.clone();
        near.entity = EntityId::new();
        near.transform = Mat4::from_translation(Vec3::Z);
        let expected = near.entity;
        let mut frame = FramePacket {
            number: 0,
            token: ViewToken {
                world: WorldId::new(),
                view: 1,
                target_generation: 1,
            },
            size: [64, 64],
            viewport: [0, 0, 64, 64],
            camera: Camera {
                eye: Vec3::new(0., 0., 4.),
                target: Vec3::ZERO,
            },
            draws: vec![far, near],
            ui: UiFrame::default(),
            pick: Some([0.5, 0.5]),
            capture: None,
            shadows: true,
        };
        frame.validate().unwrap();
        assert_eq!(pick(&frame).unwrap().entity, Some(expected));
        frame.pick = Some([0., 0.]);
        assert_eq!(pick(&frame).unwrap().entity, None);
        frame.viewport[2] = 65;
        assert!(frame.validate().is_err());
    }
}
