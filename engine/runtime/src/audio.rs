//! Runtime sound intents own no device and keep old submitted frames immutable.
use anyhow::{Context, Result, ensure};
use comet_audio::{AudioFrame, AudioLibrary, AudioVoice, MAX_VOICES, SpatialVoice};
use comet_core::{EntityId, WorldId};
use comet_scene::SceneSnapshot;
use std::{collections::BTreeMap, sync::Arc};

pub fn validate_audio(snapshot: &SceneSnapshot, library: &AudioLibrary) -> Result<()> {
    snapshot.validate()?;
    let mut required = AudioLibrary::new();
    let mut voices = 0;
    for entity in &snapshot.entities {
        if let Some(source) = &entity.audio_source {
            voices += 1;
            ensure!(voices <= MAX_VOICES, "audio sources exceed voice budget");
            let data = library.get(&source.clip).with_context(|| {
                format!("audio source {} requires clip {}", entity.id, source.clip)
            })?;
            required.insert(source.clip, data.clone());
        }
    }
    comet_audio::validate_library(&required)
}

#[derive(Clone, Copy)]
pub(crate) enum AudioCommand {
    Play(EntityId),
    Stop(EntityId),
    Gain(EntityId, f32),
}
pub(crate) struct Audio {
    world: WorldId,
    voices: Arc<[AudioVoice]>,
    indices: BTreeMap<EntityId, usize>,
    spatial: Vec<SpatialSource>,
    positions_revision: Option<u64>,
}
struct SpatialSource {
    index: usize,
    min_distance: f32,
    max_distance: f32,
}
impl Audio {
    pub fn new(snapshot: &SceneSnapshot, library: &AudioLibrary, world: WorldId) -> Result<Self> {
        validate_audio(snapshot, library)?;
        let mut entities: Vec<_> = snapshot
            .entities
            .iter()
            .filter_map(|entity| {
                entity
                    .audio_source
                    .as_ref()
                    .map(|source| (entity.id, source))
            })
            .collect();
        entities.sort_by_key(|(id, _)| *id);
        let spatial = entities
            .iter()
            .enumerate()
            .filter_map(|(index, (_, source))| {
                source.spatial.then_some(SpatialSource {
                    index,
                    min_distance: source.min_distance,
                    max_distance: source.max_distance,
                })
            })
            .collect();
        let voices: Arc<[AudioVoice]> = entities
            .into_iter()
            .map(|(entity, source)| AudioVoice {
                entity,
                generation: u64::from(source.autoplay),
                clip: source.clip,
                data: library[&source.clip].clone(),
                playing: source.autoplay,
                looping: source.looping,
                gain: source.gain,
                // The first spatial frame is unavailable until all world positions are resolved.
                spatial: None,
            })
            .collect();
        let indices = voices
            .iter()
            .enumerate()
            .map(|(index, voice)| (voice.entity, index))
            .collect();
        Ok(Self {
            world,
            voices,
            indices,
            spatial,
            positions_revision: None,
        })
    }
    pub fn needs_positions(&self, revision: u64) -> bool {
        !self.spatial.is_empty() && self.positions_revision != Some(revision)
    }
    pub fn sync_positions(&mut self, snapshot: &SceneSnapshot, revision: u64) -> Result<()> {
        if !self.needs_positions(revision) {
            return Ok(());
        }
        let transforms = comet_scene::world_transforms(snapshot)?;
        let positions = self
            .spatial
            .iter()
            .map(|source| {
                let entity = self.voices[source.index].entity;
                let matrix = transforms
                    .get(&entity)
                    .with_context(|| format!("spatial audio entity {entity} is missing"))?;
                Ok((
                    source.index,
                    SpatialVoice {
                        position: matrix.w_axis.truncate().to_array(),
                        min_distance: source.min_distance,
                        max_distance: source.max_distance,
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        if positions
            .iter()
            .any(|(index, position)| self.voices[*index].spatial != Some(*position))
        {
            let voices = Arc::make_mut(&mut self.voices);
            for (index, position) in positions {
                voices[index].spatial = Some(position);
            }
        }
        self.positions_revision = Some(revision);
        Ok(())
    }
    pub fn validate_commands(&self, commands: &[AudioCommand]) -> Result<()> {
        let mut generations = BTreeMap::new();
        for command in commands {
            let id = match *command {
                AudioCommand::Play(id) | AudioCommand::Stop(id) | AudioCommand::Gain(id, _) => id,
            };
            let index = self
                .indices
                .get(&id)
                .context("entity has no audio source")?;
            match command {
                AudioCommand::Play(_) => {
                    let generation = generations
                        .entry(id)
                        .or_insert(self.voices[*index].generation);
                    *generation = generation
                        .checked_add(1)
                        .context("audio generation exhausted")?;
                }
                AudioCommand::Gain(_, gain) => ensure!(
                    gain.is_finite() && (0. ..=1.).contains(gain),
                    "sound volume must be finite and in [0, 1]"
                ),
                AudioCommand::Stop(_) => {}
            }
        }
        Ok(())
    }
    pub fn apply_commands(&mut self, commands: &[AudioCommand]) {
        for command in commands {
            let id = match *command {
                AudioCommand::Play(id) | AudioCommand::Stop(id) | AudioCommand::Gain(id, _) => id,
            };
            let index = self.indices[&id];
            if matches!(command, AudioCommand::Stop(_) if !self.voices[index].playing)
                || matches!(command, AudioCommand::Gain(_, gain) if *gain == self.voices[index].gain)
            {
                continue;
            }
            let voice = &mut Arc::make_mut(&mut self.voices)[index];
            match *command {
                AudioCommand::Play(_) => {
                    voice.generation += 1;
                    voice.playing = true;
                }
                AudioCommand::Stop(_) => voice.playing = false,
                AudioCommand::Gain(_, gain) => voice.gain = gain,
            }
        }
    }
    pub fn frame(&self, paused: bool) -> Result<Option<AudioFrame>> {
        ensure!(
            self.spatial.is_empty() || self.positions_revision.is_some(),
            "spatial audio positions have not been synchronized"
        );
        Ok((!self.voices.is_empty()).then(|| AudioFrame {
            world: self.world,
            paused,
            voices: self.voices.clone(),
            listener: None,
        }))
    }
}

#[cfg(test)]
mod tests;
