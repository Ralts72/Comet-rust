//! Authoring sources -> immutable CPU products -> one published index.
mod build;
mod gltf;
mod package;
pub use package::{PackageOutput, PackageRequest, package, package_scenes};
mod template;
pub use template::{PackageNotice, PlayerTemplate};
mod import;
pub use build::{BuildReport, SourceMeta, prepare};
