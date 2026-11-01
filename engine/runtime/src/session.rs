use anyhow::{Result, ensure};
use comet_core::{EntityId, WorldId};
use comet_scene::{
    Scene, SceneSnapshot,
    schema::{FieldKey, FieldValue},
};
pub struct PlaySession {
    scene: Scene,
    paused: bool,
    elapsed: f64,
    steps: u64,
}
impl PlaySession {
    pub fn start(snapshot: SceneSnapshot) -> Result<Self> {
        Ok(Self {
            scene: Scene::open(snapshot)?,
            paused: false,
            elapsed: 0.,
            steps: 0,
        })
    }
    pub fn world_id(&self) -> WorldId {
        self.scene.world_id()
    }
    pub fn snapshot(&mut self) -> SceneSnapshot {
        self.scene.snapshot()
    }
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
    pub fn is_paused(&self) -> bool {
        self.paused
    }
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }
    pub fn steps(&self) -> u64 {
        self.steps
    }
    pub fn advance(&mut self, dt: f64) -> Result<()> {
        ensure!(dt.is_finite() && dt >= 0., "invalid runtime delta");
        if !self.paused {
            self.step(dt.min(0.1))?;
        }
        Ok(())
    }
    pub fn single_step(&mut self) -> Result<()> {
        ensure!(self.paused, "pause before single step");
        self.step(1. / 60.)
    }
    fn step(&mut self, dt: f64) -> Result<()> {
        self.elapsed += dt;
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("step count exhausted"))?;
        Ok(())
    }
    pub fn set_field(
        &mut self,
        ids: &[EntityId],
        field: FieldKey,
        value: FieldValue,
    ) -> Result<()> {
        let mut next = self.snapshot();
        next.set_field(ids, field, value)?;
        self.scene.replace(next)
    }
}
