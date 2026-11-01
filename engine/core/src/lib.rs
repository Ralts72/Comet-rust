//! Small shared protocols. No scene, editor, platform or graphics dependencies.
mod identity;
mod storage;

pub use identity::{AssetId, EntityId, InstanceId, WorldId};
pub use storage::{atomic_write, digest, validate_digest};
