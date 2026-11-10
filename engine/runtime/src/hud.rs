//! Script-owned HUD labels publish alongside the rest of a successful callback.
use anyhow::Result;
use comet_core::{EntityId, HudFrame, HudLabel, WorldId};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum HudCommand {
    Set(EntityId, Arc<str>),
    Clear(EntityId),
}

pub(crate) struct Hud {
    frame: HudFrame,
}
impl Hud {
    pub fn new(world: WorldId) -> Self {
        Self {
            frame: HudFrame {
                world,
                labels: Arc::from([]),
            },
        }
    }
    pub fn frame(&self) -> Option<HudFrame> {
        (!self.frame.labels.is_empty()).then(|| self.frame.clone())
    }
    pub fn prepare(&self, commands: &[HudCommand]) -> Result<Arc<[HudLabel]>> {
        if commands.is_empty() {
            return Ok(self.frame.labels.clone());
        }
        let mut labels = self.frame.labels.to_vec();
        for command in commands {
            match command {
                HudCommand::Set(owner, text) => {
                    let label = HudLabel {
                        owner: *owner,
                        text: text.clone(),
                    };
                    label.validate()?;
                    match labels.binary_search_by_key(owner, |label| label.owner) {
                        Ok(index) => labels[index] = label,
                        Err(index) => labels.insert(index, label),
                    }
                }
                HudCommand::Clear(owner) => {
                    if let Ok(index) = labels.binary_search_by_key(owner, |label| label.owner) {
                        labels.remove(index);
                    }
                }
            }
        }
        if labels.as_slice() == self.frame.labels.as_ref() {
            return Ok(self.frame.labels.clone());
        }
        let candidate = HudFrame {
            world: self.frame.world,
            labels: labels.into(),
        };
        candidate.validate()?;
        Ok(candidate.labels)
    }
    pub fn publish(&mut self, labels: Arc<[HudLabel]>) {
        self.frame.labels = labels;
    }
    pub(crate) fn remove_entities(&mut self, removed: &std::collections::BTreeSet<EntityId>) {
        if self
            .frame
            .labels
            .iter()
            .any(|label| removed.contains(&label.owner))
        {
            self.frame.labels = self
                .frame
                .labels
                .iter()
                .filter(|label| !removed.contains(&label.owner))
                .cloned()
                .collect();
        }
    }
}
