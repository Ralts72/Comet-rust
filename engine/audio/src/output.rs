use crate::{AudioFrame, AudioVoice, ListenerPose, MAX_VOICES, SpatialVoice};
use anyhow::{Result, ensure};
use comet_asset_data::AudioData;
use comet_core::{AssetId, EntityId, WorldId};
use kira::{
    AudioManager, AudioManagerSettings, Capacities, Decibels, Easing, Frame, Tween,
    backend::{Backend, cpal::CpalBackend},
    listener::ListenerHandle,
    sound::{
        PlaybackState,
        static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings},
    },
    track::{MainTrackBuilder, SpatialTrackBuilder, SpatialTrackHandle},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{Debug, Display},
    sync::{Arc, Weak},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioState {
    Idle,
    Ready,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
pub struct AudioVoiceReport {
    pub entity: EntityId,
    pub generation: u64,
    pub position_seconds: f64,
    pub playing: bool,
    pub pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spatial: Option<SpatialVoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_position: Option<[f32; 3]>,
}
#[derive(Clone, Debug, Serialize)]
pub struct AudioReport {
    pub state: AudioState,
    pub error: Option<String>,
    pub world: Option<WorldId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listener: Option<ListenerPose>,
    pub active_voices: usize,
    pub pending_voices: usize,
    pub cached_clips: usize,
    pub generations_started: u64,
    /// Accepted host output target; the device callback consumes volume changes asynchronously.
    pub master_gain: f32,
    pub voices: Vec<AudioVoiceReport>,
}

/// The only production backend is CPAL. Construction does not open a device.
pub struct Output {
    inner: OutputState<CpalBackend>,
}
impl Default for Output {
    fn default() -> Self {
        Self {
            inner: OutputState::new(|backend| backend.pop_error().map(|error| error.to_string())),
        }
    }
}
impl Output {
    /// Changes host output volume without opening a device or consuming a voice generation.
    pub fn set_master_gain(&mut self, gain: f32) -> Result<()> {
        self.inner.set_master_gain(gain)
    }
    pub fn submit(&mut self, frame: Option<&AudioFrame>) -> Result<()> {
        self.inner.submit(frame)
    }
    /// Rejects host-side preparation without submitting a fabricated frame or opening a device.
    pub fn reject(&mut self, world: WorldId, error: impl Display) {
        self.inner.reject(Some(world), error);
    }
    pub fn report(&self) -> AudioReport {
        self.inner.report()
    }
    /// Stops this world's voices without forgetting consumed generations or device diagnostics.
    pub fn stop(&mut self) {
        self.inner.stop();
    }
    pub fn shutdown(&mut self) {
        self.inner.stop();
        self.inner.manager = None;
    }
}

struct Clip {
    source: Arc<AudioData>,
    sound: StaticSoundData,
}
struct Voice {
    generation: u64,
    clip: AssetId,
    data: Weak<AudioData>,
    handle: Option<StaticSoundHandle>,
    track: Option<SpatialTrackHandle>,
    spatial: Option<SpatialVoice>,
    relative_position: Option<[f32; 3]>,
    pending: bool,
    requested: bool,
    paused: bool,
    gain: f32,
    looping: bool,
}
impl Voice {
    fn stop(&mut self) {
        if let Some(handle) = &mut self.handle {
            handle.stop(immediate());
        }
        self.pending = false;
        self.requested = false;
        self.paused = false;
    }
    fn release_route(&mut self) {
        self.stop();
        self.track = None;
    }
}

// Private generic state shares the real Kira path with Renderer-only tests, never a public backend mode.
struct OutputState<B: Backend> {
    manager: Option<AudioManager<B>>,
    poll_error: fn(&mut B) -> Option<String>,
    world: Option<WorldId>,
    report_world: Option<WorldId>,
    voices: BTreeMap<EntityId, Voice>,
    clips: BTreeMap<AssetId, Clip>,
    listener: Option<ListenerHandle>,
    last_listener: Option<ListenerPose>,
    error: Option<String>,
    generations_started: u64,
    master_gain: f32,
}
impl<B: Backend> OutputState<B>
where
    B::Settings: Default,
    B::Error: Debug,
{
    fn new(poll_error: fn(&mut B) -> Option<String>) -> Self {
        Self {
            manager: None,
            poll_error,
            world: None,
            report_world: None,
            voices: BTreeMap::new(),
            clips: BTreeMap::new(),
            listener: None,
            last_listener: None,
            error: None,
            generations_started: 0,
            master_gain: 1.,
        }
    }
    fn set_master_gain(&mut self, gain: f32) -> Result<()> {
        ensure!(
            gain.is_finite() && (0.0..=1.0).contains(&gain),
            "master audio gain must be finite and between zero and one"
        );
        if self.master_gain != gain {
            if let Some(manager) = &mut self.manager {
                manager.main_track().set_volume(volume(gain), immediate());
            }
            self.master_gain = gain;
        }
        Ok(())
    }
    fn submit(&mut self, frame: Option<&AudioFrame>) -> Result<()> {
        let result = self.submit_inner(frame);
        if let Err(error) = &result {
            self.reject(frame.map(|frame| frame.world), error);
        }
        result
    }
    fn reject(&mut self, world: Option<WorldId>, error: impl Display) {
        self.stop();
        self.manager = None;
        if let Some(world) = world {
            self.world = Some(world);
        }
        self.report_world = world;
        self.error = Some(bounded(format!("{error:#}")));
    }
    fn submit_inner(&mut self, frame: Option<&AudioFrame>) -> Result<()> {
        let Some(frame) = frame else {
            self.stop();
            return Ok(());
        };
        frame.validate()?;
        let relative_positions = frame
            .voices
            .iter()
            .filter_map(|voice| {
                voice.spatial.map(|spatial| {
                    spatial
                        .relative_position(frame.listener.unwrap())
                        .map(|position| (voice.entity, position))
                })
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        if self.world == Some(frame.world) {
            if let Some(error) = &self.error {
                anyhow::bail!("{error}");
            }
            let additional = frame
                .voices
                .iter()
                .filter(|voice| !self.voices.contains_key(&voice.entity))
                .count();
            ensure!(
                self.voices.len() + additional <= MAX_VOICES,
                "audio world exceeds retained voice identity budget"
            );
            for voice in frame.voices.iter() {
                if let Some(previous) = self.voices.get(&voice.entity) {
                    ensure!(
                        voice.generation >= previous.generation,
                        "audio generation moved backwards"
                    );
                }
            }
        }
        if self.world != Some(frame.world) {
            self.stop();
            self.voices.clear();
            self.world = Some(frame.world);
            self.error = None;
        }
        if let Some(manager) = &mut self.manager
            && let Some(error) = (self.poll_error)(manager.backend_mut())
        {
            anyhow::bail!("audio device stream failed: {error}");
        }
        self.report_world = Some(frame.world);
        if let (Some(listener), Some(pose)) = (&mut self.listener, frame.listener)
            && self.last_listener.map(ListenerPose::normalized_rotation)
                != Some(pose.normalized_rotation())
        {
            listener.set_orientation(pose.normalized_rotation(), immediate());
        }
        self.last_listener = frame.listener;
        let present: BTreeSet<_> = frame.voices.iter().map(|voice| voice.entity).collect();
        for (id, voice) in &mut self.voices {
            if !present.contains(id) {
                voice.release_route();
            }
        }
        let needed: BTreeMap<_, _> = frame
            .voices
            .iter()
            .map(|voice| (voice.clip, &voice.data))
            .collect();
        self.clips.retain(|id, clip| {
            needed
                .get(id)
                .is_some_and(|data| Arc::ptr_eq(&clip.source, data))
        });
        for incoming in frame.voices.iter() {
            let voice = self.voices.entry(incoming.entity).or_insert_with(|| Voice {
                generation: 0,
                clip: incoming.clip,
                data: Arc::downgrade(&incoming.data),
                handle: None,
                track: None,
                spatial: incoming.spatial,
                relative_position: None,
                pending: false,
                requested: false,
                paused: false,
                gain: incoming.gain,
                looping: incoming.looping,
            });
            let replaced =
                voice.clip != incoming.clip || voice.data.as_ptr() != Arc::as_ptr(&incoming.data);
            let route_changed = voice
                .spatial
                .map(|spatial| (spatial.min_distance, spatial.max_distance))
                != incoming
                    .spatial
                    .map(|spatial| (spatial.min_distance, spatial.max_distance));
            if replaced || route_changed {
                voice.release_route();
            }
            if incoming.generation > voice.generation || replaced || route_changed {
                voice.stop();
                voice.handle = None;
                // A different accepted clip alone cannot replay an already consumed generation.
                voice.pending = incoming.generation > voice.generation && incoming.playing;
                voice.generation = incoming.generation;
                voice.clip = incoming.clip;
                voice.data = Arc::downgrade(&incoming.data);
            }
            let relative_position = relative_positions.get(&incoming.entity).copied();
            if voice.relative_position != relative_position
                && let (Some(track), Some(position)) = (&mut voice.track, relative_position)
            {
                track.set_position(position, immediate());
            }
            voice.spatial = incoming.spatial;
            voice.relative_position = relative_position;
            voice.requested = incoming.playing;
            if !incoming.playing {
                voice.stop();
            }
            if let Some(handle) = &mut voice.handle {
                if voice.gain != incoming.gain {
                    handle.set_volume(volume(incoming.gain), immediate());
                }
                if voice.looping != incoming.looping {
                    if incoming.looping {
                        handle.set_loop_region(0.0..);
                    } else {
                        handle.set_loop_region(None);
                    }
                }
                if incoming.playing
                    && frame.paused != voice.paused
                    && handle.state() != PlaybackState::Stopped
                {
                    if frame.paused {
                        handle.pause(immediate());
                    } else {
                        handle.resume(immediate());
                    }
                    voice.paused = frame.paused;
                }
            }
            voice.gain = incoming.gain;
            voice.looping = incoming.looping;
            if voice.pending && !frame.paused {
                let sound = if let Some(clip) = self.clips.get(&incoming.clip) {
                    clip.sound.clone()
                } else {
                    let sound = convert(incoming);
                    self.clips.insert(
                        incoming.clip,
                        Clip {
                            source: incoming.data.clone(),
                            sound: sound.clone(),
                        },
                    );
                    sound
                };
                if self.manager.is_none() {
                    self.manager = Some(
                        AudioManager::new(AudioManagerSettings::<B> {
                            capacities: Capacities {
                                sub_track_capacity: MAX_VOICES * 2,
                                listener_capacity: 2,
                                ..Default::default()
                            },
                            main_track_builder: MainTrackBuilder::new()
                                .volume(volume(self.master_gain))
                                .sound_capacity(MAX_VOICES * 2),
                            ..Default::default()
                        })
                        .map_err(|error| anyhow::anyhow!("open audio output device: {error:?}"))?,
                    );
                }
                let sound = sound.volume(volume(incoming.gain));
                let sound = if incoming.looping {
                    sound.loop_region(0.0..)
                } else {
                    sound
                };
                let manager = self.manager.as_mut().unwrap();
                voice.handle = Some(
                    if let Some(spatial) = incoming.spatial {
                        if self.listener.is_none() {
                            self.listener = Some(
                                manager
                                    .add_listener(
                                        [0.; 3],
                                        frame.listener.unwrap().normalized_rotation(),
                                    )
                                    .map_err(|error| {
                                        anyhow::anyhow!("create audio listener: {error:?}")
                                    })?,
                            );
                        }
                        if voice.track.is_none() {
                            voice.track = Some(
                                manager
                                    .add_spatial_sub_track(
                                        self.listener.as_ref().unwrap(),
                                        relative_position.unwrap(),
                                        SpatialTrackBuilder::new()
                                            .sound_capacity(2)
                                            .sub_track_capacity(0)
                                            .persist_until_sounds_finish(false)
                                            .distances((spatial.min_distance, spatial.max_distance))
                                            .attenuation_function(Easing::Linear)
                                            .spatialization_strength(1.0),
                                    )
                                    .map_err(|error| {
                                        anyhow::anyhow!("create spatial audio track: {error:?}")
                                    })?,
                            );
                        }
                        voice.track.as_mut().unwrap().play(sound)
                    } else {
                        manager.play(sound)
                    }
                    .map_err(|error| anyhow::anyhow!("start audio voice: {error:?}"))?,
                );
                voice.pending = false;
                voice.paused = false;
                self.generations_started = self.generations_started.saturating_add(1);
            }
        }
        if self.voices.values().all(|voice| voice.track.is_none()) {
            self.listener = None;
        }
        Ok(())
    }
    fn stop(&mut self) {
        for voice in self.voices.values_mut() {
            voice.release_route();
        }
        self.listener = None;
        self.clips.clear();
        self.report_world = None;
    }
    fn report(&self) -> AudioReport {
        let voices: Vec<_> = self
            .voices
            .iter()
            .map(|(&entity, voice)| AudioVoiceReport {
                entity,
                generation: voice.generation,
                position_seconds: voice
                    .handle
                    .as_ref()
                    .map_or(0., StaticSoundHandle::position),
                playing: voice.requested
                    && voice.handle.as_ref().is_some_and(|handle| {
                        matches!(
                            handle.state(),
                            PlaybackState::Playing | PlaybackState::Resuming
                        )
                    }),
                pending: voice.pending,
                spatial: voice.spatial,
                relative_position: voice.relative_position,
            })
            .collect();
        AudioReport {
            state: if self.error.is_some() {
                AudioState::Unavailable
            } else if self.manager.is_some() {
                AudioState::Ready
            } else {
                AudioState::Idle
            },
            error: self.error.clone(),
            world: self.report_world,
            listener: self.last_listener,
            active_voices: voices.iter().filter(|voice| voice.playing).count(),
            pending_voices: voices.iter().filter(|voice| voice.pending).count(),
            cached_clips: self.clips.len(),
            generations_started: self.generations_started,
            master_gain: self.master_gain,
            voices,
        }
    }
}
fn immediate() -> Tween {
    Tween {
        duration: Duration::ZERO,
        ..Default::default()
    }
}
fn volume(gain: f32) -> Decibels {
    if gain == 0. {
        Decibels::SILENCE
    } else {
        Decibels(20. * gain.log10())
    }
}
fn bounded(error: String) -> String {
    if error.len() <= 4096 {
        return error;
    }
    let mut end = 4096;
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    error[..end].to_owned()
}
fn convert(voice: &AudioVoice) -> StaticSoundData {
    let frames: Vec<_> = if voice.data.channels == 1 {
        voice
            .data
            .samples
            .iter()
            .map(|&sample| Frame::from_mono(f32::from(sample) / 32768.))
            .collect()
    } else {
        voice
            .data
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|sample| Frame::new(f32::from(sample[0]) / 32768., f32::from(sample[1]) / 32768.))
            .collect()
    };
    StaticSoundData {
        sample_rate: voice.data.sample_rate,
        frames: frames.into(),
        settings: StaticSoundSettings::default(),
        slice: None,
    }
}

#[cfg(test)]
mod tests;
