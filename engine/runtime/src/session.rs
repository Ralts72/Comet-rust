use crate::{
    animation::{Animation, AnimationState},
    audio::{Audio, AudioCommand},
    clock::{Clock, validate_delta},
    input::InputBuffer,
    navigation::{NavigationCommand, NavigationStatus},
    physics::Physics,
    script::{ScriptDiagnostic, Scripts},
};
use anyhow::{Context, Result, ensure};
use comet_asset_data::{SceneResources, ScriptLibrary};
use comet_core::{ActionInput, AssetId, EntityId, HudFrame, ProgressValues, WorldId};
use comet_scene::{
    FieldUpdate, PoseUpdate, Scene, SceneSnapshot,
    schema::{FieldKey, FieldValue},
};
use std::{collections::VecDeque, sync::Arc};

/// A committed script intent. Hosts resolve products without borrowing the running world.
#[derive(Debug)]
pub struct SceneRequest {
    pub(crate) target: AssetId,
    pub(crate) world: WorldId,
    pub(crate) entity: EntityId,
    pub(crate) script: AssetId,
    pub(crate) tick: u64,
    pub(crate) phase: &'static str,
}
impl SceneRequest {
    pub fn target(&self) -> AssetId {
        self.target
    }
}

pub struct PlaySession {
    baseline: Arc<SceneSnapshot>,
    scene_asset: Option<AssetId>,
    entry_progress: Arc<ProgressValues>,
    scene: Scene,
    physics: Physics,
    resources: Arc<SceneResources>,
    audio: Audio,
    animation: Animation,
    navigation: Option<crate::navigation::Navigation>,
    scripts: Scripts,
    stopped: bool,
    paused: bool,
    suspended: bool,
    step_requested: bool,
    clock: Clock,
    input: InputBuffer,
    last_input: ActionInput,
    frame_steps: u32,
    discard_next_delta: bool,
}
impl PlaySession {
    fn ensure_running(&self) -> Result<()> {
        ensure!(!self.stopped, "Play session is stopped");
        Ok(())
    }

    pub fn start(snapshot: SceneSnapshot) -> Result<Self> {
        Self::start_with_scripts(snapshot, ScriptLibrary::new())
    }
    pub fn start_with_scripts(snapshot: SceneSnapshot, sources: ScriptLibrary) -> Result<Self> {
        Self::start_with_resources(
            snapshot,
            SceneResources {
                scripts: sources,
                ..Default::default()
            },
        )
    }
    pub fn start_with_resources(
        snapshot: SceneSnapshot,
        resources: SceneResources,
    ) -> Result<Self> {
        Self::from_baseline(
            Arc::new(snapshot),
            Arc::new(resources),
            None,
            Arc::new(ProgressValues::default()),
        )
    }
    /// Start an isolated world with explicit source identity and an entry progress baseline.
    pub fn start_with_progress(
        snapshot: SceneSnapshot,
        resources: SceneResources,
        scene: AssetId,
        progress: Arc<ProgressValues>,
    ) -> Result<Self> {
        Self::from_baseline(
            Arc::new(snapshot),
            Arc::new(resources),
            Some(scene),
            progress,
        )
    }
    /// Prepare the next world from all progress successfully published in the current world.
    pub fn prepare_scene(
        &self,
        snapshot: SceneSnapshot,
        resources: SceneResources,
        target: AssetId,
    ) -> Result<Self> {
        self.ensure_running()?;
        Self::start_with_progress(snapshot, resources, target, self.progress_snapshot())
    }
    pub fn scene_asset(&self) -> Option<AssetId> {
        self.scene_asset
    }
    pub fn progress_snapshot(&self) -> Arc<ProgressValues> {
        self.scripts.progress_snapshot()
    }
    fn from_baseline(
        baseline: Arc<SceneSnapshot>,
        resources: Arc<SceneResources>,
        scene_asset: Option<AssetId>,
        entry_progress: Arc<ProgressValues>,
    ) -> Result<Self> {
        entry_progress.validate()?;
        baseline.validate_expanded()?;
        let mut physics = Physics::new(&baseline)?;
        let mut scene = Scene::open((*baseline).clone())?;
        let mut animation = Animation::new(
            &baseline,
            &resources.skeletons,
            &resources.animations,
            &resources.controllers,
        )?;
        animation.sample(&mut scene, 0.)?;
        let mut audio = Audio::new(&baseline, &resources.audio, scene.world_id())?;
        let mut navigation = crate::navigation::Navigation::new(
            &baseline,
            &physics,
            resources.navigation.as_deref(),
        )?;
        let mut scripts = Scripts::new(
            &baseline,
            &resources.scripts,
            resources.data.clone(),
            scene.world_id(),
            scene_asset,
            entry_progress.clone(),
        )?;
        scripts.start(
            &mut scene,
            &mut physics,
            navigation.as_mut(),
            &mut audio,
            &mut animation,
        )?;
        scripts.flush_destruction(
            0,
            &mut scene,
            &mut physics,
            navigation.as_mut(),
            &mut audio,
            &mut animation,
        )?;
        // on_start may change playback intents; publish their real t0 state before host acceptance.
        animation.sample(&mut scene, 0.)?;
        if let Some(data) = &resources.navigation {
            data.validate_for(&scene.shared_snapshot())?;
        }
        Ok(Self {
            physics,
            scene,
            resources,
            audio,
            animation,
            navigation,
            scripts,
            stopped: false,
            baseline,
            scene_asset,
            entry_progress,
            paused: false,
            suspended: false,
            step_requested: false,
            clock: Clock::default(),
            input: InputBuffer::default(),
            last_input: ActionInput::default(),
            frame_steps: 0,
            discard_next_delta: false,
        })
    }
    /// Return to this Play session's entry snapshot, preserving pause and host suspension.
    /// A new world invalidates old picks/handles; no simulation or input crosses the boundary.
    pub fn restart(&mut self) -> Result<()> {
        self.ensure_running()?;
        let candidate = Self::from_baseline(
            self.baseline.clone(),
            self.resources.clone(),
            self.scene_asset,
            self.entry_progress.clone(),
        )?;
        self.accept_candidate(candidate)
    }
    /// Hosts take at most one request after advancement, outside all callbacks and physics steps.
    pub fn take_scene_request(&mut self) -> Option<SceneRequest> {
        self.scripts.take_scene_request()
    }
    /// Report a recoverable load failure in the requesting world's bounded script diagnostics.
    pub fn reject_scene_request(
        &mut self,
        request: SceneRequest,
        reason: impl Into<String>,
    ) -> bool {
        self.scripts.reject_scene_request(request, reason.into())
    }
    /// Accept an initialized, host-validated candidate; failed preconditions keep this world alive.
    pub fn accept_candidate(&mut self, mut candidate: Self) -> Result<()> {
        self.ensure_running()?;
        ensure!(!candidate.stopped, "candidate Play session is stopped");
        ensure!(
            candidate.world_id() != self.world_id(),
            "candidate belongs to the current runtime world"
        );
        candidate.paused = self.paused;
        candidate.suspended = self.suspended;
        candidate.discard_next_delta = true;
        self.stop();
        candidate.scripts.inherit_diagnostics(&self.scripts);
        *self = candidate;
        Ok(())
    }
    pub fn world_id(&self) -> WorldId {
        self.scene.world_id()
    }
    pub fn shared_snapshot(&mut self) -> Arc<SceneSnapshot> {
        self.scene.shared_snapshot()
    }
    pub fn snapshot(&mut self) -> SceneSnapshot {
        self.scene.snapshot()
    }
    pub fn set_paused(&mut self, paused: bool) {
        if !self.stopped && self.paused != paused {
            self.paused = paused;
            self.reset_frame();
        }
    }
    pub fn is_paused(&self) -> bool {
        self.paused
    }
    pub fn elapsed(&self) -> f64 {
        self.clock.elapsed()
    }
    pub fn steps(&self) -> u64 {
        self.clock.steps()
    }
    pub fn advance(&mut self, dt: f64) -> Result<()> {
        self.advance_with_input(dt, ActionInput::default())
    }
    pub fn advance_with_input(&mut self, dt: f64, input: ActionInput) -> Result<()> {
        self.ensure_running()?;
        validate_delta(dt)?;
        input.validate()?;
        self.physics.begin_frame();
        if self.suspended {
            self.reset_frame();
            return Ok(());
        }
        if self.input.restart_requested(&input) {
            return self.restart();
        }
        // Restart construction belongs to the host operation, not the new world's time.
        let dt = if self.discard_next_delta && dt > 0. {
            self.discard_next_delta = false;
            0.
        } else {
            dt
        };
        if self.paused {
            self.input.synchronize(input)?;
            self.frame_steps = 0;
            if self.step_requested {
                self.single_step()?;
            }
            return Ok(());
        }
        let (clock, steps) = self.clock.frame(dt)?;
        self.input.sample(input)?;
        self.clock = clock;
        self.frame_steps = 0;
        for _ in 0..steps {
            self.fixed_step()?;
        }
        Ok(())
    }
    pub fn single_step(&mut self) -> Result<()> {
        self.ensure_running()?;
        ensure!(self.paused, "pause before single step");
        ensure!(!self.suspended, "cannot step while host is suspended");
        self.physics.begin_frame();
        self.frame_steps = 0;
        self.fixed_step()?;
        self.clock.clear_remainder();
        self.step_requested = false;
        Ok(())
    }
    /// UI commands are applied after this frame's input authorization, before extraction.
    pub fn request_single_step(&mut self) -> Result<()> {
        self.ensure_running()?;
        ensure!(self.paused, "pause before single step");
        ensure!(!self.suspended, "cannot step while host is suspended");
        self.step_requested = true;
        Ok(())
    }
    pub fn set_suspended(&mut self, suspended: bool) {
        self.synchronize_host(!suspended, false);
    }
    /// Also resynchronize when loss and recovery both happened between two frames.
    pub fn synchronize_host(&mut self, active: bool, activity_changed: bool) {
        let suspended = !active;
        if !self.stopped && (self.suspended != suspended || activity_changed) {
            self.suspended = suspended;
            self.reset_frame();
        }
    }
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }
    pub fn frame_steps(&self) -> u32 {
        self.frame_steps
    }
    pub fn dropped_time(&self) -> f64 {
        self.clock.dropped()
    }
    pub fn interpolation_alpha(&self) -> f64 {
        self.clock.alpha()
    }
    /// Input of the last completed fixed tick; pair with steps(), not render frames.
    pub fn input(&self) -> &ActionInput {
        &self.last_input
    }
    fn reset_frame(&mut self) {
        self.clock.clear_remainder();
        self.input.reset();
        self.last_input = ActionInput::default();
        self.frame_steps = 0;
        self.step_requested = false;
        self.physics.begin_frame();
    }
    fn fixed_step(&mut self) -> Result<()> {
        self.clock.count_step()?;
        self.last_input = self.input.tick();
        self.scripts.fixed(
            self.clock.steps(),
            &self.last_input,
            &mut self.scene,
            &mut self.physics,
            self.navigation.as_mut(),
            &mut self.audio,
            &mut self.animation,
        );
        let velocities = if let Some(nav) = &mut self.navigation {
            Some(nav.velocities(&self.scene.shared_snapshot(), &self.physics)?)
        } else {
            None
        };
        self.physics.step(
            &self.last_input,
            self.clock.steps(),
            &mut self.scene,
            velocities,
        )?;
        self.animation
            .drive_characters(&mut self.scene, &self.physics)?;
        let animation_events = self.animation.sample(&mut self.scene, crate::FIXED_DELTA)?;
        self.scripts.triggers(
            self.clock.steps(),
            &self.last_input,
            &mut self.scene,
            &mut self.physics,
            self.navigation.as_mut(),
            &mut self.audio,
            &mut self.animation,
        );
        self.scripts.animation_events(
            &animation_events,
            self.clock.steps(),
            &self.last_input,
            &mut self.scene,
            &mut self.physics,
            self.navigation.as_mut(),
            &mut self.audio,
            &mut self.animation,
        );
        self.scripts.flush_destruction(
            self.clock.steps(),
            &mut self.scene,
            &mut self.physics,
            self.navigation.as_mut(),
            &mut self.audio,
            &mut self.animation,
        )?;
        self.frame_steps += 1;
        Ok(())
    }
    pub fn player_character(&self) -> Option<&crate::CharacterState> {
        self.physics.player()
    }
    pub fn character(&self, id: EntityId) -> Option<&crate::CharacterState> {
        self.physics.character(id)
    }
    /// The last steering result; queued goals take effect on the next fixed step.
    pub fn navigation_status(&self, entity: EntityId) -> Option<NavigationStatus> {
        self.navigation.as_ref()?.status(entity)
    }
    /// Override the author's target with a world-space point for this Play session.
    pub fn navigate_to(&mut self, entity: EntityId, point: [f32; 3]) -> Result<()> {
        self.navigation_command(NavigationCommand::NavigateTo(entity, point))
    }
    /// Follow current motor feet or the target's world transform on every fixed step.
    pub fn follow_navigation(&mut self, entity: EntityId, target: EntityId) -> Result<()> {
        self.navigation_command(NavigationCommand::Follow(entity, target))
    }
    /// Keep suppressing the author's target until another runtime goal or a restart.
    pub fn stop_navigation(&mut self, entity: EntityId) -> Result<()> {
        self.navigation_command(NavigationCommand::Stop(entity))
    }
    fn navigation_command(&mut self, command: NavigationCommand) -> Result<()> {
        self.ensure_running()?;
        let snapshot = self.scene.shared_snapshot();
        let navigation = self
            .navigation
            .as_mut()
            .context("world has no navigation")?;
        navigation.validate_commands(&snapshot, &[command])?;
        navigation.apply_commands(&[command]);
        Ok(())
    }
    pub fn joint_status(
        &self,
        entity: EntityId,
        kind: crate::JointKind,
    ) -> Option<crate::JointStatus> {
        self.physics.joint_status(entity, kind)
    }
    /// Events from all fixed ticks of the latest advance; a new advance replaces them.
    pub fn trigger_events(&self) -> &[crate::TriggerEvent] {
        self.physics.events()
    }
    pub fn stop(&mut self) {
        if !self.stopped {
            self.stopped = true;
            self.scripts.stop(
                self.clock.steps(),
                &mut self.scene,
                &mut self.physics,
                self.navigation.as_mut(),
                &mut self.audio,
                &mut self.animation,
            );
            self.input.reset();
            self.navigation = None;
            self.step_requested = false;
        }
    }
    pub fn script_diagnostics(&self) -> &VecDeque<ScriptDiagnostic> {
        self.scripts.diagnostics()
    }
    pub fn hud_frame(&self) -> Option<HudFrame> {
        self.scripts.hud_frame()
    }
    /// Seconds of the last accepted clip sample; bind pose and stopped worlds have no playback.
    pub fn animation_time(&self, entity: EntityId) -> Option<f64> {
        self.animation_state(entity).map(|state| state.time)
    }
    pub fn animation_state(&self, entity: EntityId) -> Option<AnimationState> {
        if self.stopped {
            None
        } else {
            self.animation.state(entity)
        }
    }
    /// Queue a typed controller parameter; queries change only after the next successful sample.
    pub fn set_animation_parameter(
        &mut self,
        entity: EntityId,
        name: &str,
        value: comet_asset_data::AnimationParameter,
    ) -> Result<()> {
        self.ensure_running()?;
        let prepared =
            self.animation
                .prepare_commands(&[crate::animation::AnimationCommand::Parameter(
                    entity,
                    name.into(),
                    value,
                )])?;
        self.animation.publish_commands(prepared);
        Ok(())
    }
    pub fn is_animation_controlled(&self, entity: EntityId, key: FieldKey) -> bool {
        !self.stopped && self.animation.is_controlled(entity, key)
    }
    pub fn audio_frame(&mut self) -> Result<Option<comet_audio::AudioFrame>> {
        if self.stopped {
            Ok(None)
        } else {
            let revision = self.scene.revision();
            if self.audio.needs_positions(revision) {
                self.audio
                    .sync_positions(&self.scene.shared_snapshot(), revision)?;
            }
            self.audio.frame(self.paused || self.suspended)
        }
    }
    pub fn play_sound(&mut self, entity: EntityId) -> Result<()> {
        self.audio_command(AudioCommand::Play(entity))
    }
    pub fn stop_sound(&mut self, entity: EntityId) -> Result<()> {
        self.audio_command(AudioCommand::Stop(entity))
    }
    pub fn set_sound_volume(&mut self, entity: EntityId, gain: f32) -> Result<()> {
        self.audio_command(AudioCommand::Gain(entity, gain))
    }
    fn audio_command(&mut self, command: AudioCommand) -> Result<()> {
        self.ensure_running()?;
        self.audio.validate_commands(&[command])?;
        self.audio.apply_commands(&[command]);
        Ok(())
    }
    pub fn set_linear_velocity(&mut self, id: EntityId, velocity: [f32; 3]) -> Result<()> {
        self.ensure_running()?;
        self.physics.set_velocity(id, velocity)
    }
    pub fn apply_impulse(&mut self, id: EntityId, impulse: [f32; 3]) -> Result<()> {
        self.ensure_running()?;
        self.physics.apply_impulse(id, impulse)
    }
    /// Explicit discontinuity; resets velocity and character grounding.
    pub fn teleport(&mut self, update: PoseUpdate) -> Result<()> {
        self.ensure_running()?;
        let mut candidate = self.snapshot();
        let entity = candidate
            .entities
            .iter_mut()
            .find(|e| e.id == update.entity)
            .ok_or_else(|| anyhow::anyhow!("stale teleport entity"))?;
        entity.transform.translation = update.translation;
        entity.transform.rotation = update.rotation;
        candidate.validate()?;
        ensure!(
            self.physics.contains(update.entity),
            "entity has no physics body"
        );
        self.physics.teleport(&update)?;
        self.scene.update_poses(&[update])
    }
    pub fn set_field(
        &mut self,
        ids: &[EntityId],
        field: FieldKey,
        value: FieldValue,
    ) -> Result<()> {
        self.set_fields(
            &ids.iter()
                .map(|id| FieldUpdate {
                    entity: *id,
                    field,
                    value: value.clone(),
                })
                .collect::<Vec<_>>(),
        )
    }
    /// Validate all editor fields before publishing one runtime scene update.
    pub fn set_fields(&mut self, updates: &[FieldUpdate]) -> Result<()> {
        self.ensure_running()?;
        self.animation.validate_updates(updates)?;
        for update in updates {
            ensure!(
                comet_scene::schema::value_entity_references(&update.value)
                    .iter()
                    .all(|reference| !matches!(
                        reference,
                        comet_scene::EntityRef::InInstance { .. }
                    )),
                "runtime fields cannot contain unresolved prefab instance references"
            );
            ensure!(
                !matches!(update.field.component, 4..=14),
                "physics, navigation, script, audio and animation configuration are immutable during Play"
            );
            ensure!(
                update.field.component != 1 || !self.physics.contains(update.entity),
                "physics owns body transforms during Play; use teleport"
            );
        }
        if self.navigation.is_some() {
            let snapshot = self.scene.shared_snapshot();
            for update in updates {
                ensure!(
                    update.field.component != 1
                        || snapshot
                            .entities
                            .iter()
                            .find(|e| e.id == update.entity)
                            .is_none_or(|e| e.navigation_region.is_none()),
                    "navigation region transforms are immutable during Play"
                );
            }
        }
        self.scene.set_fields(updates)
    }
}
impl Drop for PlaySession {
    fn drop(&mut self) {
        self.stop();
    }
}
