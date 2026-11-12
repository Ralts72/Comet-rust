//! Shared application host: native events, frame extraction and product-only startup.
mod application;
mod audio;
mod hud;
pub use comet_audio::AudioFrame;
mod extract;
mod fonts;
mod icon;
mod input;
#[cfg(target_os = "macos")]
mod macos;
mod navigation;
mod timing;
pub use application::*;
pub use comet_render::{Camera, PickRequest, Projection};
pub use comet_scene::world_transforms;
pub use extract::{extract, selection_bounds};
pub use fonts::configure_ui;
pub use input::{CheckpointAction, CheckpointInput, follow_target, sample_actions};
pub use navigation::navigate_camera;
