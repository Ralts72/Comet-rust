//! Clips drive the existing scene nodes; products and binding indices stay immutable per world.
use anyhow::{Context, Result, ensure};
use comet_asset_data::{
    AnimationClipData, AnimationInterpolation, AnimationLibrary, AnimationValues, SkeletonLibrary,
};
use comet_core::{EntityId, math::Quat};
use comet_scene::{
    EntityRef, FieldUpdate, Scene, SceneSnapshot, UnboundRig,
    schema::{self, FieldKey, FieldValue},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const MAX_ANIMATED_NODES: usize = 8192;
type TransformClaims = BTreeSet<(EntityId, u32)>;

/// The last successfully published clip sample, independent of the world's pause/suspend state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationState {
    /// Wrapped/clamped clip seconds, not the world's elapsed time or an unconsumed reset intent.
    pub time: f64,
    /// Selected clip length in seconds.
    pub duration: f64,
    /// A clip may be playing at speed zero or while its whole Play world is paused.
    pub playing: bool,
    /// Published playback multiplier; runtime overrides never change the author component.
    pub speed: f32,
}
pub(crate) type AnimationStates = BTreeMap<EntityId, AnimationState>;
#[derive(Clone, Copy, Debug)]
pub(crate) enum AnimationCommand {
    Play(EntityId),
    Pause(EntityId),
    Resume(EntityId),
    Speed(EntityId, f32),
}
#[derive(Clone, Copy, PartialEq)]
struct Control {
    playing: bool,
    speed: f32,
    reset: bool,
}
pub(crate) struct PreparedAnimationControls(BTreeMap<EntityId, Control>);

pub fn validate_animations(
    snapshot: &SceneSnapshot,
    skeletons: &SkeletonLibrary,
    animations: &AnimationLibrary,
) -> Result<()> {
    Animation::new(snapshot, skeletons, animations).map(|_| ())
}

struct Playback {
    clip: Arc<AnimationClipData>,
    targets: Vec<(usize, EntityId)>,
    looping: bool,
    cursor: f64,
    control: Control,
}
pub(crate) struct Animation {
    playbacks: BTreeMap<EntityId, Playback>,
    claims: TransformClaims,
    states: Arc<AnimationStates>,
}
impl Animation {
    pub fn new(
        snapshot: &SceneSnapshot,
        skeletons: &SkeletonLibrary,
        animations: &AnimationLibrary,
    ) -> Result<Self> {
        snapshot.validate()?;
        let entities: BTreeMap<_, _> = snapshot.entities.iter().map(|e| (e.id, e)).collect();
        let mut unbound = BTreeSet::new();
        let mut claims = TransformClaims::new();
        let mut driven_nodes = BTreeSet::new();
        let mut playbacks = BTreeMap::new();
        for entity in &snapshot.entities {
            if let Some(skin) = &entity.skin {
                match &skin.animator {
                    None => {
                        unbound.insert(entity.id);
                    }
                    Some(EntityRef::External { .. } | EntityRef::InInstance { .. }) => {
                        anyhow::bail!("skin animator is not loaded in this world")
                    }
                    Some(EntityRef::Local { entity: owner }) => ensure!(
                        entities.get(owner).is_some_and(|e| e.animator.is_some()),
                        "skin references an entity without an animator"
                    ),
                }
            }
            let Some(animator) = &entity.animator else {
                continue;
            };
            let skeleton = skeletons.get(&animator.skeleton).with_context(|| {
                format!(
                    "animator {} requires skeleton {}",
                    entity.id, animator.skeleton
                )
            })?;
            skeleton.validate()?;
            let nodes: BTreeSet<_> = skeleton.nodes.iter().map(|node| node.id).collect();
            let keys_match = animator.bindings.len() == nodes.len()
                && animator.bindings.keys().all(|node| nodes.contains(node));
            if !keys_match {
                unbound.insert(entity.id);
            }
            let clip = animator
                .clip
                .map(|id| -> Result<_> {
                    let clip = animations
                        .get(&id)
                        .with_context(|| format!("animator {} requires clip {id}", entity.id))?;
                    ensure!(
                        clip.skeleton == animator.skeleton,
                        "animation clip belongs to a different skeleton"
                    );
                    clip.validate_for(skeleton)?;
                    Ok(clip.clone())
                })
                .transpose()?;
            let mut targets = BTreeMap::new();
            let mut unique = BTreeSet::new();
            for node in &skeleton.nodes {
                match animator.bindings.get(&node.id).and_then(Option::as_ref) {
                    None => {
                        unbound.insert(entity.id);
                    }
                    Some(EntityRef::External { .. } | EntityRef::InInstance { .. }) => {
                        anyhow::bail!("animation target is not loaded in this world")
                    }
                    Some(EntityRef::Local { entity: target }) => {
                        ensure!(
                            *target != entity.id,
                            "animator cannot bind its own transform"
                        );
                        ensure!(entities.contains_key(target), "animation target is missing");
                        ensure!(
                            unique.insert(*target),
                            "animator binds multiple source nodes to one target"
                        );
                        targets.insert(node.id, *target);
                    }
                }
            }
            for node in &skeleton.nodes {
                let Some(target) = targets.get(&node.id) else {
                    continue;
                };
                let expected = match node.parent {
                    Some(parent) => targets.get(&parent).copied(),
                    None => Some(entity.id),
                };
                // A changed source topology needs an explicit binding-table repair first.
                if keys_match && let Some(parent) = expected {
                    ensure!(
                        entities[target].parent == Some(parent),
                        "animation target hierarchy differs from its skeleton"
                    );
                }
            }
            if let Some(clip) = clip {
                let mut bound = Vec::with_capacity(clip.channels.len());
                for (index, channel) in clip.channels.iter().enumerate() {
                    let Some(target) = targets.get(&channel.node).copied() else {
                        continue;
                    };
                    let target_data = entities[&target];
                    ensure!(
                        target_data.physics.is_none()
                            && target_data.character.is_none()
                            && target_data.navigation_region.is_none(),
                        "animation cannot drive a physics body, character or navigation region transform"
                    );
                    let field = channel_field(&channel.values);
                    ensure!(
                        claims.insert((target, field.field)),
                        "multiple animators drive the same transform field"
                    );
                    driven_nodes.insert(target);
                    ensure!(
                        driven_nodes.len() <= MAX_ANIMATED_NODES,
                        "animated node budget exceeded"
                    );
                    bound.push((index, target));
                }
                playbacks.insert(
                    entity.id,
                    Playback {
                        clip,
                        targets: bound,
                        looping: animator.looping,
                        cursor: 0.,
                        control: Control {
                            playing: true,
                            speed: animator.speed,
                            reset: false,
                        },
                    },
                );
            }
        }
        let animation = Self {
            playbacks,
            claims,
            states: Arc::new(BTreeMap::new()),
        };
        // Even an incomplete rig must not hide a bad initial pose in another valid binding.
        let mut updates = Vec::new();
        for (owner, playback) in &animation.playbacks {
            playback.append_pose(*owner, 0., &mut updates)?;
        }
        if !updates.is_empty() {
            let mut candidate = snapshot.clone();
            candidate.set_fields(&updates)?;
        }
        if !unbound.is_empty() {
            return Err(UnboundRig {
                entities: unbound.into_iter().collect(),
            }
            .into());
        }
        Ok(animation)
    }
    pub fn is_controlled(&self, entity: EntityId, key: FieldKey) -> bool {
        key.component == 1 && self.claims.contains(&(entity, key.field))
    }
    pub fn validate_updates(&self, updates: &[FieldUpdate]) -> Result<()> {
        for update in updates {
            ensure!(
                !self.is_controlled(update.entity, update.field),
                "animation owns this transform field"
            );
        }
        Ok(())
    }
    pub fn state(&self, owner: EntityId) -> Option<AnimationState> {
        self.states.get(&owner).copied()
    }
    pub fn states(&self) -> Arc<AnimationStates> {
        self.states.clone()
    }
    pub fn prepare_commands(
        &self,
        commands: &[AnimationCommand],
    ) -> Result<PreparedAnimationControls> {
        let mut controls = BTreeMap::new();
        for command in commands {
            let owner = match *command {
                AnimationCommand::Play(owner)
                | AnimationCommand::Pause(owner)
                | AnimationCommand::Resume(owner)
                | AnimationCommand::Speed(owner, _) => owner,
            };
            let playback = self
                .playbacks
                .get(&owner)
                .context("entity has no selected animation")?;
            let control = controls.entry(owner).or_insert(playback.control);
            match *command {
                AnimationCommand::Play(_) => {
                    control.playing = true;
                    control.reset = true;
                }
                AnimationCommand::Pause(_) => control.playing = false,
                AnimationCommand::Resume(_) => {
                    if control.reset
                        || playback.looping
                        || playback.cursor < f64::from(playback.clip.duration)
                    {
                        control.playing = true;
                    }
                }
                AnimationCommand::Speed(_, speed) => {
                    ensure!(
                        speed.is_finite() && speed >= 0.,
                        "animation speed must be finite and nonnegative"
                    );
                    control.speed = speed;
                }
            }
        }
        Ok(PreparedAnimationControls(controls))
    }
    pub fn publish_commands(&mut self, prepared: PreparedAnimationControls) {
        for (owner, control) in prepared.0 {
            self.playbacks
                .get_mut(&owner)
                .expect("validated animation owner")
                .control = control;
        }
    }
    pub fn sample(&mut self, scene: &mut Scene, delta: f64) -> Result<()> {
        ensure!(delta.is_finite() && delta >= 0., "invalid animation delta");
        let mut updates = Vec::new();
        let mut cursors = Vec::new();
        let mut states = Vec::new();
        for (owner, playback) in &self.playbacks {
            let mut control = playback.control;
            let duration = f64::from(playback.clip.duration);
            let cursor = if control.reset || !self.states.contains_key(owner) {
                0.
            } else if control.playing {
                let cursor = playback.cursor + delta * f64::from(control.speed);
                ensure!(cursor.is_finite(), "animation clock overflow");
                if duration == 0. {
                    0.
                } else if playback.looping {
                    cursor.rem_euclid(duration)
                } else {
                    cursor.min(duration)
                }
            } else {
                playback.cursor
            };
            control.reset = false;
            if !playback.looping && cursor == duration {
                control.playing = false;
            }
            let state = AnimationState {
                time: cursor,
                duration,
                playing: control.playing,
                speed: control.speed,
            };
            let previous = self.states.get(owner);
            // Claimed fields cannot change outside this sampler, even when the clip is paused.
            if previous.is_none_or(|state| state.time != cursor) {
                playback.append_pose(*owner, cursor, &mut updates)?;
            }
            if previous != Some(&state) {
                states.push((*owner, state));
            }
            if playback.cursor != cursor || playback.control != control {
                cursors.push((*owner, cursor, control));
            }
        }
        scene.set_fields(&updates)?;
        for (owner, cursor, control) in cursors {
            let playback = self
                .playbacks
                .get_mut(&owner)
                .expect("known animation owner");
            playback.cursor = cursor;
            playback.control = control;
        }
        if !states.is_empty() {
            let published = Arc::make_mut(&mut self.states);
            for (owner, state) in states {
                published.insert(owner, state);
            }
        }
        Ok(())
    }
}
impl Playback {
    fn append_pose(
        &self,
        owner: EntityId,
        time: f64,
        updates: &mut Vec<FieldUpdate>,
    ) -> Result<()> {
        for (index, target) in &self.targets {
            let channel = &self.clip.channels[*index];
            let value = sample_channel(channel, time)
                .with_context(|| format!("sample animator {owner}"))?;
            updates.push(FieldUpdate {
                entity: *target,
                field: channel_field(&channel.values),
                value,
            });
        }
        Ok(())
    }
}

fn channel_field(values: &AnimationValues) -> FieldKey {
    match values {
        AnimationValues::Translation(_) => schema::TRANSLATION,
        AnimationValues::Rotation(_) => schema::ROTATION,
        AnimationValues::Scale(_) => schema::SCALE,
    }
}
fn sample_channel(channel: &comet_asset_data::AnimationChannel, time: f64) -> Result<FieldValue> {
    let right = channel.times.partition_point(|key| f64::from(*key) <= time);
    let left = right.saturating_sub(1);
    let right = right.min(channel.times.len() - 1);
    let alpha = if left == right || matches!(channel.interpolation, AnimationInterpolation::Step) {
        0.
    } else {
        (time - f64::from(channel.times[left]))
            / (f64::from(channel.times[right]) - f64::from(channel.times[left]))
    };
    match &channel.values {
        AnimationValues::Translation(values) | AnimationValues::Scale(values) => {
            let value = std::array::from_fn(|i| {
                (f64::from(values[left][i]) * (1. - alpha) + f64::from(values[right][i]) * alpha)
                    as f32
            });
            ensure!(
                value.iter().all(|x| x.is_finite()),
                "animation vector overflow"
            );
            Ok(FieldValue::Vec3(value))
        }
        AnimationValues::Rotation(values) => {
            let left = Quat::from_array(values[left]);
            let right = Quat::from_array(values[right]);
            let value = if alpha == 0. {
                left
            } else {
                left.slerp(right, alpha as f32)
            }
            .normalize()
            .to_array();
            ensure!(
                value.iter().all(|x| x.is_finite()),
                "animation rotation is invalid"
            );
            Ok(FieldValue::Quat(value))
        }
    }
}
