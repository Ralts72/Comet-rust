//! Small shared protocols. No scene, editor, platform or graphics dependencies.
mod checkpoint;
mod hud;
mod identity;
mod input;
pub mod math;
mod paths;
mod progress;
mod publication;
mod storage;

pub use checkpoint::{CHECKPOINT_VERSION, Checkpoint, MAX_CHECKPOINT_BYTES};
pub use hud::{HudFrame, HudLabel, MAX_HUD_BYTES, MAX_HUD_LABEL_BYTES, MAX_HUD_LABELS};
pub use identity::{AssetId, EntityId, GameId, InstanceId, WorldId};
pub use input::{Action, ActionEvent, ActionInput, Actions, MAX_INPUT_EVENTS};
pub use paths::{ProjectPaths, resolve_destination};
pub use progress::{
    MAX_PROGRESS_BYTES, MAX_PROGRESS_ENTRIES, MAX_PROGRESS_KEY_BYTES, MAX_PROGRESS_TEXT_BYTES,
    ProgressEntry, ProgressValue, ProgressValues, validate_progress_key,
};
pub use publication::DirectoryCandidate;
pub use storage::{atomic_create, atomic_write, digest, read_bounded, validate_digest};
