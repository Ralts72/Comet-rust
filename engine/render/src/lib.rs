//! Owned frame contracts and native GPU execution. No mutable Scene or AssetStore access.
mod backend;
mod frame;
mod shader;
mod worker;
pub use backend::Surface;
pub use frame::*;
pub use worker::RenderWorker;
