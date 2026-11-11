//! Authoring sources -> immutable CPU products -> one published index.
mod audio;
mod build;
mod gltf;
mod package;
pub use package::{PackageOutput, PackageRequest, package, package_scenes};
mod template;
pub use template::{AppTemplate, PackageFile};
mod game_data;
mod icon;
mod import;
mod navigation;
pub use navigation::{
    NavigationTarget, bake_navigation, bake_navigation_source, publish_navigation,
};
mod project;
mod source;
pub use build::{BuildReport, prepare};
pub use project::{
    DEFAULT_SCENE, ProjectSettings, create_prefab, create_prefab_with_content, create_project,
    create_scene,
};
pub use source::{SourceAsset, SourceCatalog, SourceMeta};
