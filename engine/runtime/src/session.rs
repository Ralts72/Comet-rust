use crate::{
    clock::{Clock, validate_delta},
    input::InputBuffer,
    physics::Physics,
};
use anyhow::{Result, ensure};
use comet_core::{ActionInput, EntityId, WorldId};
use comet_scene::{
    PoseUpdate, Scene, SceneSnapshot,
    schema::{FieldKey, FieldValue},
};
pub struct PlaySession {
    scene: Scene,
    physics: Physics,
    paused: bool,
    suspended: bool,
    step_requested: bool,
    clock: Clock,
    input: InputBuffer,
    last_input: ActionInput,
    frame_steps: u32,
}
impl PlaySession {
    pub fn start(snapshot: SceneSnapshot) -> Result<Self> {
        Ok(Self {
            physics: Physics::new(&snapshot)?,
            scene: Scene::open(snapshot)?,
            paused: false,
            suspended: false,
            step_requested: false,
            clock: Clock::default(),
            input: InputBuffer::default(),
            last_input: ActionInput::default(),
            frame_steps: 0,
        })
    }
    pub fn world_id(&self) -> WorldId {
        self.scene.world_id()
    }
    pub fn shared_snapshot(&mut self) -> std::sync::Arc<SceneSnapshot> {
        self.scene.shared_snapshot()
    }
    pub fn snapshot(&mut self) -> SceneSnapshot {
        self.scene.snapshot()
    }
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
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
        validate_delta(dt)?;
        input.validate()?;
        self.physics.begin_frame();
        if self.suspended {
            self.reset_frame();
            return Ok(());
        }
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
        if self.suspended != suspended || activity_changed {
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
        self.physics
            .step(&self.last_input, self.clock.steps(), &mut self.scene)?;
        self.frame_steps += 1;
        Ok(())
    }
    pub fn player_character(&self) -> Option<&crate::CharacterState> {
        self.physics.player()
    }
    pub fn character(&self, id: EntityId) -> Option<&crate::CharacterState> {
        self.physics.character(id)
    }
    /// Events from all fixed ticks of the latest advance; a new advance replaces them.
    pub fn trigger_events(&self) -> &[crate::TriggerEvent] {
        self.physics.events()
    }
    pub fn set_linear_velocity(&mut self, id: EntityId, velocity: [f32; 3]) -> Result<()> {
        self.physics.set_velocity(id, velocity)
    }
    pub fn apply_impulse(&mut self, id: EntityId, impulse: [f32; 3]) -> Result<()> {
        self.physics.apply_impulse(id, impulse)
    }
    /// Explicit discontinuity; resets velocity and character grounding.
    pub fn teleport(&mut self, update: PoseUpdate) -> Result<()> {
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
        ensure!(
            !matches!(field.component, 4 | 5),
            "physics configuration is immutable during Play"
        );
        ensure!(
            field.component != 1 || ids.iter().all(|id| !self.physics.contains(*id)),
            "physics owns body transforms during Play; use teleport"
        );
        let mut next = self.snapshot();
        next.set_field(ids, field, value)?;
        self.scene.replace(next)
    }
}
