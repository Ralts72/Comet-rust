//! Immutable rig products feed per-world playback; candidate poses publish as one scene update.
use anyhow::{Context, Result, ensure};
use comet_asset_data::{
    AnimationInterpolation, AnimationLibrary, AnimationParameter, AnimationValues,
    ControllerLibrary, SkeletonLibrary,
};
use comet_core::{AssetId, EntityId, math::Quat};
use comet_scene::{
    EntityRef, FieldUpdate, Scene, SceneSnapshot, UnboundRig,
    schema::{self, FieldKey, FieldValue},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
mod character;
mod controller;
mod playback;
use playback::{Mode, Playback, Rig};

pub const MAX_ANIMATED_NODES: usize = 8192;
const MAX_EVENTS_PER_STEP: usize = 1024;
type TransformClaims = BTreeSet<(EntityId, u32)>;
pub(crate) struct AnimationEventOccurrence {
    pub animator: EntityId,
    pub name: String,
}

/// Last published playback, independent of the whole world's pause/suspend state.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationState {
    pub time: f64,
    pub duration: f64,
    pub playing: bool,
    pub speed: f32,
    pub clip: AssetId,
    pub state: Option<Arc<str>>,
    pub blend: f32,
    pub parameters: Arc<BTreeMap<String, AnimationParameter>>,
}
pub(crate) type AnimationStates = BTreeMap<EntityId, AnimationState>;
#[derive(Clone, Debug)]
pub(crate) enum AnimationCommand {
    Play(EntityId),
    Pause(EntityId),
    Resume(EntityId),
    Speed(EntityId, f32),
    Parameter(EntityId, String, AnimationParameter),
}
#[derive(Clone, PartialEq)]
struct Control {
    playing: bool,
    speed: f32,
    reset: bool,
    parameters: Arc<BTreeMap<String, AnimationParameter>>,
}
pub(crate) struct PreparedAnimationControls(BTreeMap<EntityId, Control>);
pub fn validate_animations(
    snapshot: &SceneSnapshot,
    skeletons: &SkeletonLibrary,
    clips: &AnimationLibrary,
    controllers: &ControllerLibrary,
) -> Result<()> {
    Animation::new(snapshot, skeletons, clips, controllers).map(|_| ())
}
pub(crate) struct Animation {
    playbacks: BTreeMap<EntityId, Playback>,
    claims: TransformClaims,
    states: Arc<AnimationStates>,
    drivers: Vec<character::Driver>,
}
impl Animation {
    pub fn new(
        snapshot: &SceneSnapshot,
        skeletons: &SkeletonLibrary,
        clips: &AnimationLibrary,
        controllers: &ControllerLibrary,
    ) -> Result<Self> {
        snapshot.validate()?;
        let entities: BTreeMap<_, _> = snapshot.entities.iter().map(|e| (e.id, e)).collect();
        let mut unbound = BTreeSet::new();
        let mut claims = TransformClaims::new();
        let mut driven = BTreeSet::new();
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
            let nodes: BTreeMap<_, _> = skeleton.nodes.iter().map(|n| (n.id, &n.rest)).collect();
            let keys_match = animator.bindings.len() == nodes.len()
                && animator
                    .bindings
                    .keys()
                    .all(|node| nodes.contains_key(node));
            if !keys_match {
                unbound.insert(entity.id);
            }
            let mut parameters = Arc::new(BTreeMap::new());
            let (mode, ids) = if let Some(id) = animator.controller {
                let definition = controllers
                    .get(&id)
                    .with_context(|| format!("animator {} requires controller {id}", entity.id))?
                    .clone();
                definition.validate()?;
                ensure!(
                    definition.skeleton == animator.skeleton,
                    "animation controller belongs to a different skeleton"
                );
                parameters = Arc::new(definition.parameters.clone());
                let ids = definition.clips();
                (
                    Some(Mode::Controller(controller::Machine::new(definition))),
                    ids,
                )
            } else if let Some(id) = animator.clip {
                (
                    Some(Mode::Clip {
                        id,
                        looping: animator.looping,
                        cursor: 0.,
                        started: false,
                    }),
                    [id].into(),
                )
            } else {
                (None, BTreeSet::new())
            };
            let mut prepared = BTreeMap::new();
            for id in ids {
                let clip = clips
                    .get(&id)
                    .with_context(|| format!("animator {} requires clip {id}", entity.id))?;
                ensure!(
                    clip.skeleton == animator.skeleton,
                    "animation clip belongs to a different skeleton"
                );
                clip.validate_for(skeleton)?;
                if animator.controller.is_some() {
                    ensure!(
                        clip.duration > 0.,
                        "controller clip requires positive duration"
                    );
                }
                prepared.insert(id, clip.clone());
            }
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
                if let Some(target) = targets.get(&node.id) {
                    let expected = node
                        .parent
                        .map_or(Some(entity.id), |p| targets.get(&p).copied());
                    if keys_match && let Some(parent) = expected {
                        ensure!(
                            entities[target].parent == Some(parent),
                            "animation target hierarchy differs from its skeleton"
                        );
                    }
                }
            }
            let mut rest = BTreeMap::new();
            for clip in prepared.values() {
                for channel in &clip.channels {
                    let Some(target) = targets.get(&channel.node).copied() else {
                        continue;
                    };
                    let data = entities[&target];
                    ensure!(
                        data.physics.is_none()
                            && data.character.is_none()
                            && data.navigation_region.is_none(),
                        "animation cannot drive a physics body, character or navigation region transform"
                    );
                    let field = channel_field(&channel.values).field;
                    let transform = nodes[&channel.node];
                    let value = match &channel.values {
                        AnimationValues::Translation(_) => FieldValue::Vec3(transform.translation),
                        AnimationValues::Scale(_) => FieldValue::Vec3(transform.scale),
                        AnimationValues::Rotation(_) => FieldValue::Quat(transform.rotation),
                    };
                    rest.insert((target, field), value);
                }
            }
            for (target, field) in rest.keys() {
                ensure!(
                    claims.insert((*target, *field)),
                    "multiple animators drive the same transform field"
                );
                driven.insert(*target);
                ensure!(
                    driven.len() <= MAX_ANIMATED_NODES,
                    "animated node budget exceeded"
                );
            }
            if let Some(mode) = mode {
                let rig = Arc::new(Rig {
                    targets,
                    clips: prepared,
                    rest,
                });
                let mut playback = Playback {
                    pose: Arc::new(rig.rest.clone()),
                    rig,
                    mode,
                    control: Control {
                        playing: true,
                        speed: animator.speed,
                        reset: false,
                        parameters,
                    },
                    published: false,
                };
                let mut candidate = playback.clone();
                candidate.sample(entity.id, 0., &mut Vec::new())?;
                playback.pose = candidate.pose;
                playbacks.insert(entity.id, playback);
            }
        }
        let drivers = character::prepare(snapshot, &playbacks, &claims, &mut unbound)?;
        for driver in &drivers {
            claims.insert((driver.owner, schema::TRANSLATION.field));
            claims.insert((driver.owner, schema::ROTATION.field));
        }
        let mut updates = Vec::new();
        for playback in playbacks.values() {
            for ((entity, field), value) in playback.pose.iter() {
                updates.push(FieldUpdate {
                    entity: *entity,
                    field: FieldKey {
                        component: 1,
                        field: *field,
                    },
                    value: value.clone(),
                });
            }
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
        Ok(Self {
            playbacks,
            claims,
            states: Arc::new(BTreeMap::new()),
            drivers,
        })
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
        self.states.get(&owner).cloned()
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
            let owner = match command {
                AnimationCommand::Play(id)
                | AnimationCommand::Pause(id)
                | AnimationCommand::Resume(id)
                | AnimationCommand::Speed(id, _)
                | AnimationCommand::Parameter(id, _, _) => *id,
            };
            let playback = self
                .playbacks
                .get(&owner)
                .context("entity has no selected animation")?;
            let control = controls
                .entry(owner)
                .or_insert_with(|| playback.control.clone());
            match command {
                AnimationCommand::Play(_) => {
                    control.playing = true;
                    control.reset = true;
                }
                AnimationCommand::Pause(_) => control.playing = false,
                AnimationCommand::Resume(_) => {
                    if control.reset || playback.can_resume() {
                        control.playing = true;
                    }
                }
                AnimationCommand::Speed(_, speed) => {
                    ensure!(
                        speed.is_finite() && *speed >= 0.,
                        "animation speed must be finite and nonnegative"
                    );
                    control.speed = *speed;
                }
                AnimationCommand::Parameter(_, name, value) => {
                    value.validate()?;
                    let previous = control
                        .parameters
                        .get(name)
                        .context("unknown animation controller parameter")?;
                    ensure!(
                        previous.same_type(*value),
                        "animation controller parameter type mismatch"
                    );
                    if previous != value {
                        Arc::make_mut(&mut control.parameters).insert(name.clone(), *value);
                    }
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
    pub fn sample(
        &mut self,
        scene: &mut Scene,
        delta: f64,
    ) -> Result<Vec<AnimationEventOccurrence>> {
        ensure!(delta.is_finite() && delta >= 0., "invalid animation delta");
        let mut events = Vec::new();
        let mut updates = Vec::new();
        let mut candidates = Vec::new();
        let mut states = Vec::new();
        for (owner, playback) in &self.playbacks {
            let mut candidate = playback.clone();
            let state = if playback.published
                && !playback.control.reset
                && (delta == 0. || !playback.control.playing || playback.control.speed == 0.)
            {
                let mut state = self.states[owner].clone();
                state.playing = playback.control.playing;
                state.speed = playback.control.speed;
                state.parameters = playback.control.parameters.clone();
                state
            } else {
                candidate.sample(*owner, delta, &mut events)?
            };
            for ((entity, field), value) in candidate.pose.iter() {
                if !playback.published || playback.pose.get(&(*entity, *field)) != Some(value) {
                    updates.push(FieldUpdate {
                        entity: *entity,
                        field: FieldKey {
                            component: 1,
                            field: *field,
                        },
                        value: value.clone(),
                    });
                }
            }
            if *candidate.pose == *playback.pose {
                candidate.pose = playback.pose.clone();
            }
            if self.states.get(owner) != Some(&state) {
                states.push((*owner, state));
            }
            candidates.push((*owner, candidate));
        }
        scene.set_fields(&updates)?;
        for (owner, playback) in candidates {
            self.playbacks.insert(owner, playback);
        }
        if !states.is_empty() {
            let published = Arc::make_mut(&mut self.states);
            for (owner, state) in states {
                published.insert(owner, state);
            }
        }
        Ok(events)
    }
    pub fn drive_characters(
        &mut self,
        scene: &mut Scene,
        physics: &crate::physics::Physics,
    ) -> Result<()> {
        let (commands, updates) =
            character::drive(&self.drivers, &scene.shared_snapshot(), physics)?;
        let prepared = self.prepare_commands(&commands)?;
        scene.set_fields(&updates)?;
        self.publish_commands(prepared);
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
            let value = sample_values(values, channel, left, right, alpha).map(|v| v as f32);
            ensure!(
                value.iter().all(|x| x.is_finite()),
                "animation vector overflow"
            );
            Ok(FieldValue::Vec3(value))
        }
        AnimationValues::Rotation(values) => {
            if channel.interpolation == AnimationInterpolation::CubicSpline {
                let value = sample_values(values, channel, left, right, alpha);
                let length = value.iter().map(|v| v * v).sum::<f64>().sqrt();
                ensure!(
                    length.is_finite() && length > 0.,
                    "animation rotation is invalid"
                );
                return Ok(FieldValue::Quat(value.map(|v| (v / length) as f32)));
            }
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

fn sample_values<const N: usize>(
    values: &[[f32; N]],
    channel: &comet_asset_data::AnimationChannel,
    left: usize,
    right: usize,
    alpha: f64,
) -> [f64; N] {
    if channel.interpolation != AnimationInterpolation::CubicSpline {
        return std::array::from_fn(|i| {
            f64::from(values[left][i]) * (1. - alpha) + f64::from(values[right][i]) * alpha
        });
    }
    if alpha == 0. {
        return values[left * 3 + 1].map(f64::from);
    }
    // glTF tangents are derivatives in seconds, not normalized segment time.
    let duration = f64::from(channel.times[right]) - f64::from(channel.times[left]);
    let t2 = alpha * alpha;
    let t3 = t2 * alpha;
    std::array::from_fn(|i| {
        (2. * t3 - 3. * t2 + 1.) * f64::from(values[left * 3 + 1][i])
            + duration * (t3 - 2. * t2 + alpha) * f64::from(values[left * 3 + 2][i])
            + (-2. * t3 + 3. * t2) * f64::from(values[right * 3 + 1][i])
            + duration * (t3 - t2) * f64::from(values[right * 3][i])
    })
}
