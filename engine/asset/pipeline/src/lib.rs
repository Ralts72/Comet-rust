//! Authoring sources -> immutable CPU products -> one published index.
mod build;
mod gltf;
mod package;
pub use package::{PackageOutput, PackageRequest, package, package_scenes};
mod template;
pub use template::{AppTemplate, PackageFile};
mod icon;
mod import;
mod project;
mod source;
pub use build::{BuildReport, prepare};
pub use project::{ProjectSettings, create_scene};
pub use source::{SourceAsset, SourceCatalog, SourceMeta};
