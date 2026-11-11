//! CPU asset protocols and scene resources, independent of importers, stores and GPU handles.
pub mod animation;
pub mod audio;
pub mod game_data;
pub mod navigation;
pub mod package;
pub mod product;
mod resources;
pub use animation::AnimationLibraries;
pub use animation::controller::{
    AnimationCompare, AnimationCondition, AnimationControllerData, AnimationControllerState,
    AnimationMotion, AnimationParameter, AnimationTransition, ControllerLibrary,
};
pub use animation::{
    ANIMATION_CLIP_VERSION, ANIMATION_VERSION, AnimationChannel, AnimationClipData, AnimationEvent,
    AnimationInterpolation, AnimationLibrary, AnimationValues, MAX_ANIMATION_BYTES,
    MAX_ANIMATION_CHANNELS, MAX_ANIMATION_KEYS, MAX_SKIN_JOINTS, MAX_SKIN_VERTICES, SkeletonData,
    SkeletonLibrary, SkeletonNode, SkinData, SkinJoint,
};
pub use audio::AudioData;
pub use navigation::{NavigationData, NavigationInput, NavigationProfile};
pub use resources::{AudioLibrary, MAX_SCENE_DATA_ASSETS, SceneResources, ScriptLibrary};
