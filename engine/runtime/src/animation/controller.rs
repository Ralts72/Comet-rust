use super::playback::{Pose, PoseSample, Rig, append_events, blend_pose};
use super::{AnimationEventOccurrence, Control};
use anyhow::{Result, ensure};
use comet_asset_data::{
    AnimationCompare as Compare, AnimationControllerData, AnimationParameter as Parameter,
};
use comet_core::EntityId;
use std::sync::Arc;

#[derive(Clone)]
struct Fade {
    from: Arc<Pose>,
    elapsed: f64,
    duration: f64,
}
#[derive(Clone)]
pub(super) struct Machine {
    pub definition: Arc<AnimationControllerData>,
    state: usize,
    phase: f64,
    smoothed: f64,
    started: bool,
    fade: Option<Fade>,
}
impl Machine {
    pub fn new(definition: Arc<AnimationControllerData>) -> Self {
        let state = definition
            .states
            .iter()
            .position(|s| s.name == definition.initial)
            .expect("validated initial state");
        Self {
            definition,
            state,
            phase: 0.,
            smoothed: 0.,
            started: false,
            fade: None,
        }
    }
    fn parameter(&self, control: &Control) -> f64 {
        self.definition.states[self.state]
            .parameter
            .as_ref()
            .map_or(0., |name| match control.parameters[name] {
                Parameter::Number(value) => f64::from(value),
                Parameter::Bool(_) => unreachable!("validated blend parameter"),
            })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &mut self,
        rig: &Rig,
        control: &Control,
        reset: bool,
        owner: EntityId,
        delta: f64,
        previous: &Arc<Pose>,
        events: &mut Vec<AnimationEventOccurrence>,
    ) -> Result<PoseSample> {
        if reset {
            self.state = self
                .definition
                .states
                .iter()
                .position(|s| s.name == self.definition.initial)
                .expect("validated state");
            self.phase = 0.;
            self.started = false;
            self.fade = None;
            self.smoothed = self.parameter(control);
        } else if control.playing && control.speed > 0. && delta > 0. {
            let state = &self.definition.states[self.state];
            let finished = !state.looping && self.phase >= 1.;
            let transition = self.definition.transitions.iter().find(|t| {
                t.to != state.name
                    && t.from.as_ref().is_none_or(|from| *from == state.name)
                    && (!t.after_finished || finished)
                    && t.conditions
                        .iter()
                        .all(|c| compare(control.parameters[&c.parameter], c.compare, c.value))
            });
            if let Some(transition) = transition {
                let duration = f64::from(transition.duration);
                self.state = self
                    .definition
                    .states
                    .iter()
                    .position(|s| s.name == transition.to)
                    .expect("validated transition");
                self.phase = 0.;
                self.started = false;
                self.smoothed = self.parameter(control);
                // One published source pose bounds interrupted transitions and preserves continuity.
                self.fade = (duration > 0.).then(|| Fade {
                    from: previous.clone(),
                    elapsed: 0.,
                    duration,
                });
            } else {
                let state = &self.definition.states[self.state];
                let target = self.parameter(control);
                let alpha = if state.smoothing == 0. {
                    1.
                } else {
                    1. - (-delta / f64::from(state.smoothing)).exp()
                };
                self.smoothed = (1. - alpha) * self.smoothed + alpha * target;
                let (_, _, _, period) = self.weights(rig);
                let end = self.phase + delta * f64::from(control.speed) / period;
                ensure!(end.is_finite(), "animation controller clock overflow");
                append_events(
                    owner,
                    &state.events,
                    self.phase,
                    end,
                    state.looping,
                    self.started,
                    events,
                )?;
                self.started = true;
                self.phase = if state.looping {
                    end.rem_euclid(1.)
                } else {
                    end.min(1.)
                };
                if let Some(fade) = &mut self.fade {
                    fade.elapsed =
                        (fade.elapsed + delta * f64::from(control.speed)).min(fade.duration);
                }
            }
        }
        let (a, b, weight, period) = self.weights(rig);
        let mut pose = rig.sample(a, self.phase * f64::from(rig.clips[&a].duration))?;
        if a != b {
            pose = blend_pose(
                &pose,
                &rig.sample(b, self.phase * f64::from(rig.clips[&b].duration))?,
                weight,
            )?;
        }
        let blend = self
            .fade
            .as_ref()
            .map_or(1., |fade| (fade.elapsed / fade.duration).min(1.));
        if let Some(fade) = &self.fade {
            pose = blend_pose(&fade.from, &pose, blend)?;
        }
        if blend == 1. {
            self.fade = None;
        }
        Ok(PoseSample {
            pose,
            time: self.phase * period,
            duration: period,
            clip: if weight < 0.5 { a } else { b },
            name: Some(Arc::from(self.definition.states[self.state].name.as_str())),
            blend: blend as f32,
        })
    }
    fn weights(&self, rig: &Rig) -> (comet_core::AssetId, comet_core::AssetId, f64, f64) {
        let motions = &self.definition.states[self.state].motions;
        let right = motions.partition_point(|m| f64::from(m.threshold) <= self.smoothed);
        let left = right.saturating_sub(1);
        let right = right.min(motions.len() - 1);
        let (a, b) = (&motions[left], &motions[right]);
        let weight = if left == right {
            0.
        } else {
            ((self.smoothed - f64::from(a.threshold))
                / (f64::from(b.threshold) - f64::from(a.threshold)))
            .clamp(0., 1.)
        };
        let period = (1. - weight) * f64::from(rig.clips[&a.clip].duration)
            + weight * f64::from(rig.clips[&b.clip].duration);
        (a.clip, b.clip, weight, period)
    }
}
fn compare(a: Parameter, op: Compare, b: Parameter) -> bool {
    match (a, b) {
        (Parameter::Bool(a), Parameter::Bool(b)) => match op {
            Compare::Equal => a == b,
            Compare::NotEqual => a != b,
            _ => false,
        },
        (Parameter::Number(a), Parameter::Number(b)) => match op {
            Compare::Equal => a == b,
            Compare::NotEqual => a != b,
            Compare::Greater => a > b,
            Compare::GreaterEqual => a >= b,
            Compare::Less => a < b,
            Compare::LessEqual => a <= b,
        },
        _ => false,
    }
}
