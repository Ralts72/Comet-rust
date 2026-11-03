//! Shared application host: native events, frame extraction and product-only startup.
mod application;
mod extract;
mod fonts;
mod icon;
mod input;
mod timing;
pub use application::*;
pub use comet_render::Camera;
pub use extract::{extract, world_transforms};
pub use fonts::configure_ui;
pub use input::{follow_target, sample_actions};
