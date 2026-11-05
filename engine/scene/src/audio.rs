use anyhow::{Result, ensure};
use bevy_ecs::prelude::Component;
use comet_core::AssetId;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioSource {
    pub clip: AssetId,
    pub gain: f32,
    pub looping: bool,
    pub autoplay: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub spatial: bool,
    #[serde(
        default = "default_min_distance",
        skip_serializing_if = "is_default_min_distance"
    )]
    pub min_distance: f32,
    #[serde(
        default = "default_max_distance",
        skip_serializing_if = "is_default_max_distance"
    )]
    pub max_distance: f32,
}
impl AudioSource {
    pub fn new(clip: AssetId) -> Self {
        Self {
            clip,
            gain: 1.,
            looping: false,
            autoplay: false,
            spatial: false,
            min_distance: default_min_distance(),
            max_distance: default_max_distance(),
        }
    }
    pub fn required_scene_version(&self) -> u32 {
        if self.spatial
            || !is_default_min_distance(&self.min_distance)
            || !is_default_max_distance(&self.max_distance)
        {
            8
        } else {
            7
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.gain.is_finite() && (0. ..=1.).contains(&self.gain),
            "audio gain must be finite and in [0, 1]"
        );
        ensure!(
            self.min_distance.is_finite()
                && self.max_distance.is_finite()
                && self.min_distance >= 0.
                && self.min_distance < self.max_distance
                && (self.max_distance - self.min_distance).is_normal(),
            "audio distances require finite 0 <= min < max and a normal distance span"
        );
        Ok(())
    }
}

fn is_false(value: &bool) -> bool {
    !value
}
fn default_min_distance() -> f32 {
    1.
}
fn default_max_distance() -> f32 {
    30.
}
fn is_default_min_distance(value: &f32) -> bool {
    *value == default_min_distance()
}
fn is_default_max_distance(value: &f32) -> bool {
    *value == default_max_distance()
}

// Missing components are omitted; an explicit null must not bypass the scene version gate.
pub(crate) fn deserialize_source<'de, D: serde::Deserializer<'de>>(
    value: D,
) -> std::result::Result<Option<AudioSource>, D::Error> {
    AudioSource::deserialize(value).map(Some)
}
