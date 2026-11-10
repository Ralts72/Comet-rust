use super::controller::Machine;
use super::{
    AnimationEventOccurrence, AnimationState, Control, MAX_EVENTS_PER_STEP, channel_field,
    sample_channel,
};
use anyhow::{Result, ensure};
use comet_asset_data::{AnimationClipData, AnimationEvent};
use comet_core::{AssetId, EntityId, math::Quat};
use comet_scene::schema::FieldValue;
use std::{collections::BTreeMap, sync::Arc};

pub(super) type Pose = BTreeMap<(EntityId, u32), FieldValue>;
pub(super) struct Rig {
    pub targets: BTreeMap<EntityId, EntityId>,
    pub clips: BTreeMap<AssetId, Arc<AnimationClipData>>,
    pub rest: Pose,
}
impl Rig {
    pub fn sample(&self, clip: AssetId, time: f64) -> Result<Pose> {
        let mut pose = self.rest.clone();
        for channel in &self.clips[&clip].channels {
            if let Some(target) = self.targets.get(&channel.node) {
                pose.insert(
                    (*target, channel_field(&channel.values).field),
                    sample_channel(channel, time)?,
                );
            }
        }
        Ok(pose)
    }
}
pub(super) struct PoseSample {
    pub pose: Pose,
    pub time: f64,
    pub duration: f64,
    pub clip: AssetId,
    pub name: Option<Arc<str>>,
    pub blend: f32,
}
#[derive(Clone)]
pub(super) enum Mode {
    Clip {
        id: AssetId,
        looping: bool,
        cursor: f64,
        started: bool,
    },
    Controller(Machine),
}
#[derive(Clone)]
pub(super) struct Playback {
    pub rig: Arc<Rig>,
    pub mode: Mode,
    pub control: Control,
    pub pose: Arc<Pose>,
    pub published: bool,
}
impl Playback {
    pub fn sample(
        &mut self,
        owner: EntityId,
        delta: f64,
        events: &mut Vec<AnimationEventOccurrence>,
    ) -> Result<AnimationState> {
        let reset = self.control.reset || !self.published;
        let sample = match &mut self.mode {
            Mode::Clip {
                id,
                looping,
                cursor,
                started,
            } => {
                let clip = &self.rig.clips[id];
                let duration = f64::from(clip.duration);
                if reset {
                    *cursor = 0.;
                    *started = false;
                } else if self.control.playing {
                    let end = *cursor + delta * f64::from(self.control.speed);
                    ensure!(end.is_finite(), "animation clock overflow");
                    if end > *cursor && duration > 0. {
                        append_events(
                            owner,
                            &clip.events,
                            *cursor / duration,
                            end / duration,
                            *looping,
                            *started,
                            events,
                        )?;
                        *started = true;
                    }
                    *cursor = if duration == 0. {
                        0.
                    } else if *looping {
                        end.rem_euclid(duration)
                    } else {
                        end.min(duration)
                    };
                }
                if !*looping && *cursor == duration {
                    self.control.playing = false;
                }
                PoseSample {
                    pose: self.rig.sample(*id, *cursor)?,
                    time: *cursor,
                    duration,
                    clip: *id,
                    name: None,
                    blend: 1.,
                }
            }
            Mode::Controller(machine) => machine.sample(
                &self.rig,
                &self.control,
                reset,
                owner,
                delta,
                &self.pose,
                events,
            )?,
        };
        self.control.reset = false;
        self.pose = Arc::new(sample.pose);
        self.published = true;
        Ok(AnimationState {
            time: sample.time,
            duration: sample.duration,
            playing: self.control.playing,
            speed: self.control.speed,
            clip: sample.clip,
            state: sample.name,
            blend: sample.blend,
            parameters: self.control.parameters.clone(),
        })
    }
    pub fn can_resume(&self) -> bool {
        match &self.mode {
            Mode::Controller(_) => true,
            Mode::Clip {
                id,
                looping,
                cursor,
                ..
            } => self.control.reset || *looping || *cursor < f64::from(self.rig.clips[id].duration),
        }
    }
}

pub(super) fn blend_pose(a: &Pose, b: &Pose, weight: f64) -> Result<Pose> {
    ensure!(
        a.len() == b.len() && a.keys().eq(b.keys()),
        "animation pose domains differ"
    );
    a.iter()
        .map(|(key, value)| {
            let value = match (value, &b[key]) {
                (FieldValue::Vec3(a), FieldValue::Vec3(b)) => {
                    FieldValue::Vec3(std::array::from_fn(|i| {
                        ((1. - weight) * f64::from(a[i]) + weight * f64::from(b[i])) as f32
                    }))
                }
                (FieldValue::Quat(a), FieldValue::Quat(b)) => FieldValue::Quat(
                    Quat::from_array(*a)
                        .normalize()
                        .slerp(Quat::from_array(*b).normalize(), weight as f32)
                        .normalize()
                        .to_array(),
                ),
                _ => anyhow::bail!("animation blend requires transform fields"),
            };
            Ok((*key, value))
        })
        .collect()
}
pub(super) fn append_events(
    owner: EntityId,
    markers: &[AnimationEvent],
    from: f64,
    end: f64,
    looping: bool,
    started: bool,
    out: &mut Vec<AnimationEventOccurrence>,
) -> Result<()> {
    if markers.is_empty() || end <= from {
        return Ok(());
    }
    let end = if looping { end } else { end.min(1.) };
    let cycles = if looping { end.floor() } else { 0. };
    ensure!(
        cycles.is_finite() && cycles <= MAX_EVENTS_PER_STEP as f64,
        "animation event step budget exceeded"
    );
    for cycle in 0..=cycles as usize {
        for marker in markers {
            let time = cycle as f64 + f64::from(marker.phase);
            if time <= end && (time > from || (!started && time == 0.)) {
                ensure!(
                    out.len() < MAX_EVENTS_PER_STEP,
                    "animation event step budget exceeded"
                );
                out.push(AnimationEventOccurrence {
                    animator: owner,
                    name: marker.name.clone(),
                });
            }
        }
    }
    Ok(())
}
