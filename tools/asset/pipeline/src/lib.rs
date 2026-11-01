//! Authoring sources -> immutable CPU products -> one published index.
mod build;
mod gltf;
mod package;
pub use package::package;
mod import;
pub use build::{BuildReport, SourceMeta, prepare};
