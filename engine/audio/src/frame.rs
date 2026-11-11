use anyhow::{Context, Result, ensure};
use comet_asset_data::{AudioData, AudioLibrary};
use comet_core::{
    AssetId, EntityId, WorldId,
    math::{DVec3, Quat, Vec3},
};
use serde::Serialize;
use std::{collections::BTreeSet, sync::Arc};

pub const MAX_VOICES: usize = 64;
pub const MAX_TOTAL_SAMPLES: usize = 16_777_216;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SpatialVoice {
    /// World position in meters; scale does not change the distance range.
    pub position: [f32; 3],
    pub min_distance: f32,
    pub max_distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ListenerPose {
    pub position: [f32; 3],
    /// World xyzw rotation. Identity faces -Z with +X right and +Y up.
    pub rotation: [f32; 4],
}
impl ListenerPose {
    fn validate(self) -> Result<()> {
        ensure!(
            self.position.iter().all(|value| value.is_finite()),
            "audio listener position must be finite"
        );
        let rotation = Quat::from_array(self.rotation);
        ensure!(
            rotation.is_finite() && rotation.is_normalized(),
            "audio listener rotation must be a finite unit quaternion"
        );
        Ok(())
    }
    pub(crate) fn normalized_rotation(self) -> [f32; 4] {
        Quat::from_array(self.rotation).normalize().to_array()
    }
}
impl SpatialVoice {
    pub(crate) fn relative_position(self, listener: ListenerPose) -> Result<[f32; 3]> {
        ensure!(
            self.position.iter().all(|value| value.is_finite()),
            "spatial audio position must be finite"
        );
        ensure!(
            self.min_distance.is_finite()
                && self.max_distance.is_finite()
                && self.min_distance >= 0.
                && self.max_distance > self.min_distance
                && (self.max_distance - self.min_distance).is_normal(),
            "spatial audio requires 0 <= min distance < max distance and a normal f32 span"
        );
        let relative = DVec3::from_array(self.position.map(f64::from))
            - DVec3::from_array(listener.position.map(f64::from));
        let rotation = Quat::from_array(listener.normalized_rotation());
        // Kira uses f32 distances and ears at +/-0.1m. A relative frame preserves that ear
        // spacing under large world translations; headroom keeps its dot products finite.
        for ear in [
            Vec3::ZERO,
            rotation * (Vec3::NEG_X * 0.1),
            rotation * (Vec3::X * 0.1),
        ] {
            ensure!(
                (relative - ear.as_dvec3()).length_squared() <= f64::from(f32::MAX) / 4.,
                "spatial audio relative position exceeds the safe f32 distance range"
            );
        }
        Ok(relative.as_vec3().to_array())
    }
}

#[derive(Clone)]
pub struct AudioVoice {
    pub entity: EntityId,
    /// Zero has never played; only a larger generation starts or restarts a voice.
    pub generation: u64,
    pub clip: AssetId,
    pub data: Arc<AudioData>,
    pub playing: bool,
    pub looping: bool,
    pub gain: f32,
    pub spatial: Option<SpatialVoice>,
}

#[derive(Clone)]
pub struct AudioFrame {
    pub world: WorldId,
    pub paused: bool,
    pub voices: Arc<[AudioVoice]>,
    /// The host fills the accepted view's listener; runtime intents have no camera ownership.
    pub listener: Option<ListenerPose>,
}
impl AudioFrame {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.voices.len() <= MAX_VOICES,
            "audio frame exceeds voice budget"
        );
        let mut entities = BTreeSet::new();
        let mut library = AudioLibrary::new();
        if let Some(listener) = self.listener {
            listener.validate()?;
        }
        for voice in self.voices.iter() {
            if let Some(spatial) = voice.spatial {
                spatial.relative_position(
                    self.listener.context("spatial audio requires a listener")?,
                )?;
            }
            ensure!(entities.insert(voice.entity), "duplicate audio entity");
            ensure!(
                voice.gain.is_finite() && (0. ..=1.).contains(&voice.gain),
                "audio gain must be in [0, 1]"
            );
            ensure!(
                !voice.playing || voice.generation > 0,
                "playing audio requires a generation"
            );
            if let Some(previous) = library.insert(voice.clip, voice.data.clone()) {
                ensure!(
                    Arc::ptr_eq(&previous, &voice.data),
                    "audio clip has conflicting accepted data identities"
                );
            }
        }
        validate_library(&library)
    }
}

/// Shared clips count once, independently of how many sources play them.
pub fn validate_library(library: &AudioLibrary) -> Result<()> {
    ensure!(
        library.len() <= MAX_VOICES,
        "audio library exceeds clip budget"
    );
    let mut total = 0usize;
    for data in library.values() {
        data.validate()?;
        total = total
            .checked_add(data.samples.len())
            .ok_or_else(|| anyhow::anyhow!("audio sample budget overflow"))?;
        ensure!(
            total <= MAX_TOTAL_SAMPLES,
            "audio library exceeds total sample budget"
        );
    }
    Ok(())
}
