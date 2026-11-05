//! Bounded checkpoint bytes. Storage locations and publication belong to the host.
use crate::{AssetId, GameId, ProgressValues};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const CHECKPOINT_VERSION: u32 = 1;
pub const MAX_CHECKPOINT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub version: u32,
    pub game_id: GameId,
    pub scene: AssetId,
    pub progress: ProgressValues,
}
impl Checkpoint {
    pub fn new(game_id: GameId, scene: AssetId, mut progress: ProgressValues) -> Result<Self> {
        progress.canonicalize();
        let checkpoint = Self {
            version: CHECKPOINT_VERSION,
            game_id,
            scene,
            progress,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == CHECKPOINT_VERSION,
            "unsupported checkpoint version {}",
            self.version
        );
        self.progress.validate()
    }
    pub fn validate_for(&self, game_id: GameId) -> Result<()> {
        self.validate()?;
        ensure!(
            self.game_id == game_id,
            "checkpoint belongs to another game"
        );
        Ok(())
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_CHECKPOINT_BYTES,
            "checkpoint byte budget exceeded"
        );
        let mut checkpoint: Self = serde_json::from_slice(bytes).context("parse checkpoint")?;
        checkpoint.validate()?;
        checkpoint.progress.canonicalize();
        Ok(checkpoint)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut checkpoint = self.clone();
        checkpoint.progress.canonicalize();
        let bytes = serde_json::to_vec(&checkpoint).context("encode checkpoint")?;
        ensure!(
            bytes.len() <= MAX_CHECKPOINT_BYTES,
            "checkpoint byte budget exceeded"
        );
        Ok(bytes)
    }
}
