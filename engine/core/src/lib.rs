//! Small shared protocols. No scene, editor, platform or graphics dependencies.
mod identity;
mod input;
pub mod math;
mod paths;
mod publication;
mod storage;

pub use identity::{AssetId, EntityId, InstanceId, WorldId};
pub use input::{Action, ActionEvent, ActionInput, Actions, MAX_INPUT_EVENTS};
pub use paths::{ProjectPaths, resolve_destination};
pub use publication::DirectoryCandidate;
pub use storage::{atomic_create, atomic_write, digest, validate_digest};

pub const BUILD_ID: &str = env!("COMET_BUILD_ID");
pub const BUILD_TARGET: &str = env!("COMET_BUILD_TARGET");
