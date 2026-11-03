use std::time::Instant;

/// Host policy is independent of swapchain acquisition and smoke-test extents.
pub(crate) struct FrameTiming {
    last: Option<Instant>,
    focused: bool,
    occluded: bool,
    minimized: bool,
    suspended: bool,
    activity_changed: bool,
}

impl Default for FrameTiming {
    fn default() -> Self {
        Self {
            last: None,
            focused: true,
            occluded: false,
            minimized: false,
            suspended: false,
            activity_changed: true,
        }
    }
}
impl FrameTiming {
    pub fn active(&self) -> bool {
        self.focused && !self.occluded && !self.minimized && !self.suspended
    }
    pub fn sample(&mut self, now: Instant) -> f64 {
        if !self.active() {
            self.last = None;
            return 0.;
        }
        self.last
            .replace(now)
            .map_or(0., |last| now.saturating_duration_since(last).as_secs_f64())
    }
    pub fn take_activity_change(&mut self) -> bool {
        std::mem::take(&mut self.activity_changed)
    }
    fn update(flag: &mut bool, value: bool, last: &mut Option<Instant>, changed: &mut bool) {
        if *flag != value {
            *flag = value;
            *last = None;
            *changed = true;
        }
    }
    pub fn focused(&mut self, value: bool) {
        Self::update(
            &mut self.focused,
            value,
            &mut self.last,
            &mut self.activity_changed,
        );
    }
    pub fn occluded(&mut self, value: bool) {
        Self::update(
            &mut self.occluded,
            value,
            &mut self.last,
            &mut self.activity_changed,
        );
    }
    pub fn minimized(&mut self, value: bool) {
        Self::update(
            &mut self.minimized,
            value,
            &mut self.last,
            &mut self.activity_changed,
        );
    }
    pub fn suspended(&mut self, value: bool) {
        Self::update(
            &mut self.suspended,
            value,
            &mut self.last,
            &mut self.activity_changed,
        );
    }
}

#[cfg(test)]
#[path = "../tests/unit/timing.rs"]
mod tests;
