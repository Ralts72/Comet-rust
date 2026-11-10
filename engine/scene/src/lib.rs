//! Persistent scene content, independent of editing and simulation services.
mod animation;
mod audio;
pub use animation::{
    Animator, AnimatorBindings, CharacterAnimation, MAX_ANIMATOR_BINDINGS, SkinBinding, UnboundRig,
};
pub mod components;
pub use audio::AudioSource;
mod joints;
mod navigation;
pub mod physics;
pub mod prefab;
pub mod scene;
pub mod schema;
mod script;
mod transform;
pub use components::{EntityData, EntityRef, Links, Model, Surface, Transform};
pub use joints::{
    HingeJoint, JointConnection, JointFrame, JointLimits, JointMotor, JointSettings, PointJoint,
    SliderJoint,
};
pub use navigation::{NavigationAgent, NavigationRegion};
pub use physics::{BodyMotion, Character, ColliderShape, PhysicsBody};
pub use prefab::{ContentSource, FieldOverride, PrefabInstance, PrefabResolver, PrefabRoot};
pub use scene::{EntityHandle, FieldUpdate, PoseUpdate, Scene, SceneSnapshot};
pub use script::{
    MAX_SCRIPT_KEY_BYTES, MAX_SCRIPT_PARAMETERS, MAX_SCRIPT_TEXT_BYTES, Script, ScriptParameters,
    ScriptValue,
};
pub use transform::{checked_transform_inverse, world_transforms};
