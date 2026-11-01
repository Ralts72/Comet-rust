//! Persistent scene content, independent of editing and simulation services.
pub mod components;
pub mod prefab;
pub mod scene;
pub mod schema;
pub use components::{EntityData, EntityRef, Links, Model, Surface, Transform};
pub use prefab::{ContentSource, FieldOverride, PrefabInstance, PrefabResolver};
pub use scene::{EntityHandle, Scene, SceneSnapshot};
