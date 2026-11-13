//! Persistent physics configuration. Runtime owns all backend objects and simulated state.
use crate::EntityData;
use anyhow::{Result, ensure};
use bevy_ecs::prelude::Component;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyMotion {
    Fixed,
    Dynamic,
    Kinematic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ColliderShape {
    Box {
        half_extents: [f32; 3],
    },
    Sphere {
        radius: f32,
    },
    /// Y-axis segment half length, excluding the two hemispheres.
    Capsule {
        half_height: f32,
        radius: f32,
    },
}

impl ColliderShape {
    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::Box { half_extents } => ensure!(
                half_extents.iter().copied().all(positive),
                "invalid box dimensions"
            ),
            Self::Sphere { radius } => ensure!(positive(*radius), "invalid radius"),
            Self::Capsule {
                half_height,
                radius,
            } => {
                ensure!(positive(*radius), "invalid radius");
                ensure!(nonnegative(*half_height), "invalid capsule height");
            }
        }
        Ok(())
    }
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsBody {
    pub motion: BodyMotion,
    pub shape: ColliderShape,
    pub layer: u32,
    pub mask: u32,
    pub sensor: bool,
    pub mass: f32,
    pub friction: f32,
    pub restitution: f32,
}
impl Default for PhysicsBody {
    fn default() -> Self {
        Self {
            motion: BodyMotion::Fixed,
            shape: ColliderShape::Box {
                half_extents: [0.5; 3],
            },
            layer: 1,
            mask: u32::MAX,
            sensor: false,
            mass: 1.,
            friction: 0.5,
            restitution: 0.,
        }
    }
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Character {
    pub player_controlled: bool,
    pub move_speed: f32,
    pub run_multiplier: f32,
    pub jump_speed: f32,
    pub gravity: f32,
    pub max_slope_degrees: f32,
    pub step_height: f32,
    pub snap_distance: f32,
    pub crouch_speed: f32,
    /// Total capsule height ratio; the radius sets the minimum possible height.
    pub crouch_height_ratio: f32,
}
impl Default for Character {
    fn default() -> Self {
        Self {
            player_controlled: true,
            move_speed: 4.,
            run_multiplier: 1.75,
            jump_speed: 5.,
            gravity: 9.81,
            max_slope_degrees: 45.,
            step_height: 0.3,
            snap_distance: 0.15,
            crouch_speed: 2.,
            crouch_height_ratio: 0.6,
        }
    }
}
fn positive(v: f32) -> bool {
    v.is_finite() && v > 0.
}
fn nonnegative(v: f32) -> bool {
    v.is_finite() && v >= 0.
}

pub(crate) fn validate(entity: &EntityData) -> Result<()> {
    let Some(body) = &entity.physics else {
        ensure!(
            entity.character.is_none(),
            "character {} requires a physics body",
            entity.id
        );
        return Ok(());
    };
    ensure!(
        entity.parent.is_none(),
        "physics body {} must be a scene root",
        entity.id
    );
    let scale = entity.transform.scale;
    ensure!(
        scale.into_iter().all(positive),
        "physics body {} requires positive scale",
        entity.id
    );
    match body.shape {
        ColliderShape::Box { half_extents } => {
            ensure!(
                half_extents
                    .into_iter()
                    .zip(scale)
                    .all(|(h, s)| positive(h * s)),
                "box scale overflow"
            );
        }
        ColliderShape::Sphere { radius } | ColliderShape::Capsule { radius, .. } => {
            ensure!(positive(radius * scale[0]), "invalid radius");
            ensure!(
                scale.into_iter().all(|s| (s / scale[0] - 1.).abs() < 1e-6),
                "sphere and capsule require uniform scale"
            );
        }
    }
    if let ColliderShape::Capsule { half_height, .. } = body.shape {
        ensure!(
            nonnegative(half_height * scale[0]),
            "invalid capsule height"
        );
    }
    // Field descriptors own value ranges; this layer checks entity and world constraints.
    if entity.character.is_some() {
        ensure!(
            body.motion == BodyMotion::Kinematic
                && !body.sensor
                && matches!(body.shape, ColliderShape::Capsule { .. }),
            "character requires a solid kinematic capsule"
        );
        let r = entity.transform.rotation;
        ensure!(
            r[0].abs() < 1e-5 && r[2].abs() < 1e-5,
            "character must remain upright"
        );
    }
    Ok(())
}
