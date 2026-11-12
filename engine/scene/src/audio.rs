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
    pub spatial: bool,
    pub min_distance: f32,
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
            min_distance: 1.,
            max_distance: 30.,
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

// An absent component is omitted; an explicit null is not a component configuration.
pub(crate) fn deserialize_source<'de, D: serde::Deserializer<'de>>(
    value: D,
) -> std::result::Result<Option<AudioSource>, D::Error> {
    AudioSource::deserialize(value).map(Some)
}
