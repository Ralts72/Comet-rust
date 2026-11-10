//! Owned, immutable collision geometry for callback queries; no borrowed Rapier world enters Rhai.
use super::Physics;
use anyhow::{Result, ensure};
use comet_core::EntityId;
use rapier3d::prelude::{Pose, Ray, SharedShape, Vector};

struct Shape {
    entity: EntityId,
    pose: Pose,
    shape: SharedShape,
    layers: u32,
    sensor: bool,
}
#[derive(Default)]
pub(crate) struct Queries(Vec<Shape>);
pub(crate) struct RayHit {
    pub entity: EntityId,
    pub fraction: f32,
    pub position: [f32; 3],
    pub normal: [f32; 3],
}
impl Queries {
    pub(super) fn capture(physics: &Physics) -> Self {
        Self(
            physics
                .bodies
                .iter()
                .map(|(entity, body)| {
                    let collider = &physics.world.colliders[body.collider];
                    Shape {
                        entity: *entity,
                        pose: *collider.position(),
                        shape: collider.shared_shape().clone(),
                        layers: collider.collision_groups().memberships.bits(),
                        sensor: collider.is_sensor(),
                    }
                })
                .collect(),
        )
    }
    pub fn raycast(
        &self,
        origin: [f32; 3],
        displacement: [f32; 3],
        layers: u32,
        ignore: EntityId,
    ) -> Result<Option<RayHit>> {
        ensure!(self.0.len() <= 8192, "physics query body budget exceeded");
        ensure!(
            origin.iter().chain(&displacement).all(|x| x.is_finite()),
            "ray must be finite"
        );
        let length = displacement
            .into_iter()
            .map(|v| f64::from(v).powi(2))
            .sum::<f64>()
            .sqrt();
        ensure!(
            length > 0. && length <= f64::from(f32::MAX),
            "ray displacement must have finite positive length"
        );
        ensure!(
            (0..3).all(
                |i| (f64::from(origin[i]) + f64::from(displacement[i])).abs()
                    <= f64::from(f32::MAX)
            ),
            "ray endpoint overflows"
        );
        let direction = displacement.map(|v| (f64::from(v) / length) as f32);
        let ray = Ray::new(Vector::from_array(origin), Vector::from_array(direction));
        let mut result = None;
        let mut nearest = length as f32;
        for shape in &self.0 {
            if shape.entity == ignore || shape.sensor || shape.layers & layers == 0 {
                continue;
            }
            if let Some(hit) = shape
                .shape
                .cast_ray_and_get_normal(&shape.pose, &ray, nearest, true)
            {
                // Equal-distance ties keep the first stable entity UUID.
                if result.is_some() && hit.time_of_impact >= nearest {
                    continue;
                }
                nearest = hit.time_of_impact;
                let position = ray.origin + ray.dir * nearest;
                ensure!(
                    nearest.is_finite()
                        && nearest >= 0.
                        && position.is_finite()
                        && hit.normal.is_finite(),
                    "physics ray result is not finite"
                );
                result = Some(RayHit {
                    entity: shape.entity,
                    fraction: (f64::from(nearest) / length) as f32,
                    position: position.to_array(),
                    normal: if nearest == 0. {
                        (-ray.dir).to_array()
                    } else {
                        hit.normal.to_array()
                    },
                });
            }
        }
        Ok(result)
    }
}
