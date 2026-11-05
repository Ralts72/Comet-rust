//! Immutable, plain-text game HUD data shared by simulation and presentation.
use crate::{EntityId, WorldId};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::{collections::BTreeSet, sync::Arc};

pub const MAX_HUD_LABELS: usize = 32;
pub const MAX_HUD_LABEL_BYTES: usize = 4096;
pub const MAX_HUD_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HudLabel {
    pub owner: EntityId,
    pub text: Arc<str>,
}
impl HudLabel {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.text.len() <= MAX_HUD_LABEL_BYTES,
            "HUD label exceeds its text byte budget"
        );
        ensure!(!self.text.contains('\0'), "HUD text contains NUL");
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HudFrame {
    pub world: WorldId,
    pub labels: Arc<[HudLabel]>,
}
impl HudFrame {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.labels.len() <= MAX_HUD_LABELS,
            "HUD label budget exceeded"
        );
        let mut owners = BTreeSet::new();
        let mut bytes = 0;
        for label in self.labels.iter() {
            label.validate()?;
            ensure!(owners.insert(label.owner), "HUD contains a duplicate owner");
            bytes += label.text.len();
            ensure!(
                bytes <= MAX_HUD_BYTES,
                "HUD total text byte budget exceeded"
            );
        }
        Ok(())
    }
}
