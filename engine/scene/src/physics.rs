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
    #[serde(
        default = "default_crouch_speed",
        skip_serializing_if = "is_default_crouch_speed"
    )]
    pub crouch_speed: f32,
    /// Total capsule height ratio; the radius sets the minimum possible height.
    #[serde(
        default = "default_crouch_height_ratio",
        skip_serializing_if = "is_default_crouch_height_ratio"
    )]
    pub crouch_height_ratio: f32,
}
fn default_crouch_speed() -> f32 {
    2.
}
fn default_crouch_height_ratio() -> f32 {
    0.6
}
fn is_default_crouch_speed(value: &f32) -> bool {
    *value == default_crouch_speed()
}
fn is_default_crouch_height_ratio(value: &f32) -> bool {
    *value == default_crouch_height_ratio()
}
impl Character {
    pub fn required_scene_version(&self) -> u32 {
        if self.has_custom_crouch() { 3 } else { 2 }
    }
    pub(crate) fn has_custom_crouch(&self) -> bool {
        !is_default_crouch_speed(&self.crouch_speed)
            || !is_default_crouch_height_ratio(&self.crouch_height_ratio)
    }
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
            crouch_speed: default_crouch_speed(),
            crouch_height_ratio: default_crouch_height_ratio(),
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
                half_extents.into_iter().all(positive),
                "invalid box dimensions"
            );
            ensure!(
                half_extents
                    .into_iter()
                    .zip(scale)
                    .all(|(h, s)| positive(h * s)),
                "box scale overflow"
            );
        }
        ColliderShape::Sphere { radius } | ColliderShape::Capsule { radius, .. } => {
            ensure!(
                positive(radius) && positive(radius * scale[0]),
                "invalid radius"
            );
            ensure!(
                scale.into_iter().all(|s| (s / scale[0] - 1.).abs() < 1e-6),
                "sphere and capsule require uniform scale"
            );
        }
    }
    if let ColliderShape::Capsule { half_height, .. } = body.shape {
        ensure!(
            nonnegative(half_height) && nonnegative(half_height * scale[0]),
            "invalid capsule height"
        );
    }
    ensure!(positive(body.mass), "physics mass must be positive");
    ensure!(nonnegative(body.friction), "invalid friction");
    ensure!(
        body.restitution.is_finite() && (0. ..=1.).contains(&body.restitution),
        "invalid restitution"
    );
    if let Some(c) = &entity.character {
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
        ensure!(
            [
                c.move_speed,
                c.run_multiplier,
                c.jump_speed,
                c.gravity,
                c.crouch_speed
            ]
            .into_iter()
            .all(positive),
            "invalid character speed or gravity"
        );
        ensure!(
            c.crouch_height_ratio.is_finite() && (0.1..=1.).contains(&c.crouch_height_ratio),
            "invalid character crouch height ratio"
        );
        ensure!(
            c.max_slope_degrees.is_finite() && (0. ..90.).contains(&c.max_slope_degrees),
            "invalid character slope"
        );
        ensure!(
            nonnegative(c.step_height) && nonnegative(c.snap_distance),
            "invalid step or snap distance"
        );
    }
    Ok(())
}
