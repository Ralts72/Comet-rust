//! Persistent script configuration; compilation and execution belong to runtime consumers.
use crate::EntityRef;
use anyhow::{Result, ensure};
use bevy_ecs::prelude::Component;
use comet_core::AssetId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_SCRIPT_PARAMETERS: usize = 64;
pub const MAX_SCRIPT_KEY_BYTES: usize = 128;
pub const MAX_SCRIPT_TEXT_BYTES: usize = 4096;
pub type ScriptParameters = BTreeMap<String, ScriptValue>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ScriptValue {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    Vec3([f32; 3]),
    /// Deleted local targets become None; external references retain their explicit scene identity.
    Entity(Option<EntityRef>),
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    pub asset: AssetId,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub parameters: ScriptParameters,
}
fn enabled() -> bool {
    true
}

impl Script {
    pub fn new(asset: AssetId) -> Self {
        Self {
            asset,
            enabled: true,
            parameters: BTreeMap::new(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_parameters(&self.parameters)
    }
}

pub(crate) fn validate_parameters(parameters: &ScriptParameters) -> Result<()> {
    ensure!(
        parameters.len() <= MAX_SCRIPT_PARAMETERS,
        "script has too many parameters"
    );
    for (key, value) in parameters {
        ensure!(
            !key.trim().is_empty() && key.len() <= MAX_SCRIPT_KEY_BYTES,
            "invalid script parameter key"
        );
        match value {
            ScriptValue::Number(value) => {
                ensure!(value.is_finite(), "script number must be finite")
            }
            ScriptValue::Text(value) => ensure!(
                value.len() <= MAX_SCRIPT_TEXT_BYTES,
                "script text is too long"
            ),
            ScriptValue::Vec3(value) => ensure!(
                value.iter().all(|v| v.is_finite()),
                "script vector must be finite"
            ),
            _ => {}
        }
    }
    Ok(())
}
