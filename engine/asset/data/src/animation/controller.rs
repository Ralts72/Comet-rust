//! A controller owns state/transition definitions; runtime instances own clocks and parameters.
use super::{AnimationEvent, validate_events};
use anyhow::{Result, ensure};
use comet_core::AssetId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_CONTROLLER_BYTES: usize = 256 * 1024;
pub type ControllerLibrary = BTreeMap<AssetId, std::sync::Arc<AnimationControllerData>>;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnimationParameter {
    Bool(bool),
    Number(f32),
}
impl AnimationParameter {
    pub fn same_type(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
    pub fn validate(self) -> Result<()> {
        ensure!(
            !matches!(self, Self::Number(value) if !value.is_finite()),
            "animation parameter must be finite"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationMotion {
    pub threshold: f32,
    pub clip: AssetId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationControllerState {
    pub name: String,
    pub motions: Vec<AnimationMotion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
    #[serde(default)]
    pub smoothing: f32,
    pub looping: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<AnimationEvent>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimationCompare {
    #[serde(rename = "==")]
    Equal,
    #[serde(rename = "!=")]
    NotEqual,
    #[serde(rename = ">")]
    Greater,
    #[serde(rename = ">=")]
    GreaterEqual,
    #[serde(rename = "<")]
    Less,
    #[serde(rename = "<=")]
    LessEqual,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationCondition {
    pub parameter: String,
    pub compare: AnimationCompare,
    pub value: AnimationParameter,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationTransition {
    /// None means any state; a transition never re-enters its current state.
    pub from: Option<String>,
    pub to: String,
    pub duration: f32,
    #[serde(default)]
    pub after_finished: bool,
    #[serde(default)]
    pub conditions: Vec<AnimationCondition>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationControllerData {
    pub version: u32,
    pub skeleton: AssetId,
    #[serde(deserialize_with = "parameters")]
    pub parameters: BTreeMap<String, AnimationParameter>,
    pub initial: String,
    pub states: Vec<AnimationControllerState>,
    pub transitions: Vec<AnimationTransition>,
}
fn parameters<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, AnimationParameter>, D::Error> {
    struct Parameters;
    impl<'de> serde::de::Visitor<'de> for Parameters {
        type Value = BTreeMap<String, AnimationParameter>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 32 unique animation parameters")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            use serde::de::Error;
            let mut values = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if values.len() == 32 || values.contains_key(&key) {
                    return Err(M::Error::custom(
                        "animation parameter budget or duplicate name",
                    ));
                }
                values.insert(key, map.next_value()?);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Parameters)
}
fn name(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
}
impl AnimationControllerData {
    pub fn validate_clip(&self, clip: &super::AnimationClipData) -> Result<()> {
        clip.validate()?;
        ensure!(
            clip.skeleton == self.skeleton && clip.duration > 0.,
            "controller clip requires the same skeleton and positive duration"
        );
        Ok(())
    }
    pub fn clips(&self) -> BTreeSet<AssetId> {
        self.states
            .iter()
            .flat_map(|s| s.motions.iter().map(|m| m.clip))
            .collect()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported animation controller version"
        );
        ensure!(
            self.parameters.len() <= 32
                && !self.states.is_empty()
                && self.states.len() <= 32
                && self.transitions.len() <= 128,
            "animation controller definition budget exceeded"
        );
        for (key, value) in &self.parameters {
            ensure!(name(key), "invalid animation parameter name");
            value.validate()?;
        }
        let mut names = BTreeSet::new();
        for state in &self.states {
            ensure!(
                name(&state.name) && names.insert(state.name.as_str()),
                "invalid or duplicate animation state name"
            );
            ensure!(
                !state.motions.is_empty() && state.motions.len() <= 8,
                "animation state motion budget exceeded"
            );
            ensure!(
                state.smoothing.is_finite() && (0. ..=30.).contains(&state.smoothing),
                "invalid animation parameter smoothing"
            );
            ensure!(
                state.motions.iter().all(|m| m.threshold.is_finite())
                    && state
                        .motions
                        .windows(2)
                        .all(|m| m[0].threshold < m[1].threshold),
                "animation motion thresholds must be finite and strictly increasing"
            );
            if let Some(parameter) = &state.parameter {
                ensure!(
                    matches!(
                        self.parameters.get(parameter),
                        Some(AnimationParameter::Number(_))
                    ),
                    "animation blend parameter must name a numeric parameter"
                );
            } else {
                ensure!(
                    state.motions.len() == 1 && state.smoothing == 0.,
                    "multiple animation motions need a blend parameter"
                );
            }
            validate_events(&state.events)?;
        }
        ensure!(
            names.contains(self.initial.as_str()),
            "missing initial animation state"
        );
        ensure!(
            self.clips().len() <= 64,
            "animation controller clip budget exceeded"
        );
        for transition in &self.transitions {
            ensure!(
                names.contains(transition.to.as_str())
                    && transition
                        .from
                        .as_ref()
                        .is_none_or(|name| names.contains(name.as_str())),
                "animation transition references an unknown state"
            );
            ensure!(
                transition.from.as_ref() != Some(&transition.to),
                "animation transition cannot target itself"
            );
            ensure!(
                transition.duration.is_finite() && (0. ..=30.).contains(&transition.duration),
                "invalid animation transition duration"
            );
            ensure!(
                transition.conditions.len() <= 32,
                "animation transition condition budget exceeded"
            );
            for condition in &transition.conditions {
                let value = self.parameters.get(&condition.parameter).ok_or_else(|| {
                    anyhow::anyhow!("animation condition references an unknown parameter")
                })?;
                condition.value.validate()?;
                ensure!(
                    value.same_type(condition.value),
                    "animation condition parameter type mismatch"
                );
                ensure!(
                    matches!(value, AnimationParameter::Number(_))
                        || matches!(
                            condition.compare,
                            AnimationCompare::Equal | AnimationCompare::NotEqual
                        ),
                    "boolean animation conditions support only equality"
                );
            }
        }
        Ok(())
    }
}
