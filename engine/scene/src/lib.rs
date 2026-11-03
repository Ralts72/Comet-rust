//! Persistent scene content, independent of editing and simulation services.
pub mod components;
pub mod physics;
pub mod prefab;
pub mod scene;
pub mod schema;
pub use components::{EntityData, EntityRef, Links, Model, Surface, Transform};
pub use physics::{BodyMotion, Character, ColliderShape, PhysicsBody};
pub use prefab::{ContentSource, FieldOverride, PrefabInstance, PrefabResolver};
pub use scene::{EntityHandle, PoseUpdate, Scene, SceneSnapshot};
