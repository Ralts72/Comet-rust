//! Shared application host: native events, frame extraction and product-only startup.
mod application;
mod extract;
pub use application::*;
pub use comet_render::Camera;
pub use extract::{extract, world_transforms};
