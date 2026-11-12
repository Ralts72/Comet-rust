//! Headless authoring document and edit history. UI and runtime instances are consumers.
mod document;
pub use document::{
    CloneMap, Document, FieldEdit, FieldTarget, JointEdit, MAX_PLACEMENT_COPIES, ObjectSnapshot,
    ObjectTarget, OverrideReset,
};
