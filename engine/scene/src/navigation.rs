use crate::{EntityData, EntityRef};
use anyhow::{Result, ensure};
use bevy_ecs::prelude::Component;
use comet_core::AssetId;
use serde::{Deserialize, Serialize};

/// One axis-aligned baking volume, centered on the owner's translation, in world meters.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationRegion {
    pub data: Option<AssetId>,
    pub half_extents: [f32; 3],
    pub cell_size: f32,
    pub agent_radius: f32,
    pub agent_height: f32,
    pub max_climb: f32,
    pub max_slope_degrees: f32,
}
impl Default for NavigationRegion {
    fn default() -> Self {
        Self {
            data: None,
            half_extents: [20., 5., 20.],
            cell_size: 0.2,
            agent_radius: 0.4,
            agent_height: 1.8,
            max_climb: 0.3,
            max_slope_degrees: 45.,
        }
    }
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationAgent {
    pub enabled: bool,
    pub target: Option<EntityRef>,
    pub arrival_distance: f32,
}
impl Default for NavigationAgent {
    fn default() -> Self {
        Self {
            enabled: true,
            target: None,
            arrival_distance: 0.35,
        }
    }
}

pub(crate) fn validate(entity: &EntityData) -> Result<()> {
    if let Some(region) = &entity.navigation_region {
        ensure!(
            entity.parent.is_none(),
            "navigation region must be a scene root"
        );
        ensure!(
            entity.transform.scale == [1.; 3]
                && entity.transform.rotation[0..3] == [0.; 3]
                && entity.transform.rotation[3].abs() == 1.,
            "navigation region requires unit rotation and scale"
        );
        ensure!(
            entity.character.is_none(),
            "navigation region cannot be a character"
        );
        ensure!(
            entity
                .physics
                .as_ref()
                .is_none_or(|body| body.motion == crate::BodyMotion::Fixed),
            "navigation region cannot be a moving body"
        );
        ensure!(
            region.agent_height >= 2. * region.agent_radius,
            "navigation agent height must cover its diameter"
        );
        ensure!(
            region.max_climb <= region.agent_height,
            "navigation climb exceeds agent height"
        );
    }
    if entity.navigation_agent.is_some() {
        ensure!(
            entity
                .character
                .as_ref()
                .is_some_and(|c| !c.player_controlled),
            "navigation agent requires a non-player character"
        );
    }
    Ok(())
}
