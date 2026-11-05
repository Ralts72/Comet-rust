//! Explicit progress is an immutable callback snapshot, independent of scene and VM state.
use anyhow::{Context, Result, ensure};
use comet_core::{
    AssetId, EntityId, ProgressEntry, ProgressValue, ProgressValues, validate_progress_key,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum ProgressCommand {
    Set(EntityId, String, ProgressValue),
    Remove(EntityId, String),
}
pub(crate) struct Progress {
    scope: Option<AssetId>,
    values: Arc<ProgressValues>,
}
impl Progress {
    pub fn new(scope: Option<AssetId>, values: Arc<ProgressValues>) -> Result<Self> {
        values.validate()?;
        Ok(Self { scope, values })
    }
    pub fn scope(&self) -> Option<AssetId> {
        self.scope
    }
    pub fn snapshot(&self) -> Arc<ProgressValues> {
        self.values.clone()
    }
    pub fn prepare(&self, commands: &[ProgressCommand]) -> Result<Arc<ProgressValues>> {
        if commands.is_empty() {
            return Ok(self.snapshot());
        }
        let scene = self
            .scope
            .context("progress requires a scene asset scope")?;
        let mut candidate = (*self.values).clone();
        for command in commands {
            let (owner, key) = match command {
                ProgressCommand::Set(owner, key, _) | ProgressCommand::Remove(owner, key) => {
                    (*owner, key)
                }
            };
            validate_progress_key(key)?;
            let index = candidate.entries.iter().position(|entry| {
                entry.scene == scene && entry.owner == owner && entry.key == *key
            });
            match command {
                ProgressCommand::Set(_, _, value) => {
                    value.validate()?;
                    if let Some(index) = index {
                        ensure!(
                            candidate.entries[index].value.same_type(value),
                            "progress value type does not match its existing value"
                        );
                        candidate.entries[index].value = value.clone();
                    } else {
                        candidate.entries.push(ProgressEntry {
                            scene,
                            owner,
                            key: key.clone(),
                            value: value.clone(),
                        });
                    }
                }
                ProgressCommand::Remove(..) => {
                    if let Some(index) = index {
                        candidate.entries.remove(index);
                    }
                }
            }
        }
        candidate.canonicalize();
        candidate.validate()?;
        if candidate == *self.values {
            Ok(self.snapshot())
        } else {
            Ok(Arc::new(candidate))
        }
    }
    pub fn publish(&mut self, values: Arc<ProgressValues>) {
        self.values = values;
    }
}
