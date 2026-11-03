use anyhow::{Result, ensure};

pub const FIXED_DELTA: f64 = 1. / 60.;
pub const MAX_FRAME_STEPS: u32 = 8;
const MAX_FRAME_DELTA: f64 = 0.25;
const EPSILON: f64 = FIXED_DELTA * 1e-9;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Clock {
    remainder: f64,
    dropped: f64,
    steps: u64,
}
pub(crate) fn validate_delta(dt: f64) -> Result<()> {
    ensure!(dt.is_finite() && dt >= 0., "invalid runtime delta");
    Ok(())
}
impl Clock {
    /// Plan a bounded frame without mutating the accepted clock or input.
    pub fn frame(mut self, dt: f64) -> Result<(Self, u32)> {
        validate_delta(dt)?;
        let accepted = dt.min(MAX_FRAME_DELTA);
        let accumulated = self.remainder + accepted;
        let whole = ((accumulated + EPSILON) / FIXED_DELTA).floor() as u32;
        let steps = whole.min(MAX_FRAME_STEPS);
        ensure!(
            self.steps.checked_add(u64::from(steps)).is_some(),
            "step count exhausted"
        );
        self.dropped += dt - accepted + f64::from(whole - steps) * FIXED_DELTA;
        ensure!(self.dropped.is_finite(), "dropped time exhausted");
        self.remainder = (accumulated - f64::from(whole) * FIXED_DELTA).max(0.);
        Ok((self, steps))
    }
    pub fn count_step(&mut self) -> Result<()> {
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("step count exhausted"))?;
        Ok(())
    }
    pub fn clear_remainder(&mut self) {
        self.remainder = 0.;
    }
    pub fn steps(self) -> u64 {
        self.steps
    }
    pub fn elapsed(self) -> f64 {
        self.steps as f64 * FIXED_DELTA
    }
    pub fn dropped(self) -> f64 {
        self.dropped
    }
    pub fn alpha(self) -> f64 {
        (self.remainder / FIXED_DELTA).clamp(0., 1.)
    }
}
