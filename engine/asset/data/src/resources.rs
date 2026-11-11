//! Preloaded CPU resources for one scene; no live world, VM, output or loading policy.
use crate::{
    AnimationLibrary, AudioData, ControllerLibrary, NavigationData, SkeletonLibrary,
    game_data::DataLibrary,
};
use comet_core::AssetId;
use std::{collections::BTreeMap, sync::Arc};

pub type ScriptLibrary = BTreeMap<AssetId, String>;
pub type AudioLibrary = BTreeMap<AssetId, Arc<AudioData>>;
pub const MAX_SCENE_DATA_ASSETS: usize = 64;

#[derive(Clone, Default)]
pub struct SceneResources {
    pub data: DataLibrary,
    pub scripts: ScriptLibrary,
    pub navigation: Option<Arc<NavigationData>>,
    pub audio: AudioLibrary,
    pub skeletons: SkeletonLibrary,
    pub animations: AnimationLibrary,
    pub controllers: ControllerLibrary,
}
