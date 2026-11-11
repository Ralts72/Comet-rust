//! Read-only CPU product store. It never opens authoring sources or imports assets.
mod resources;
mod store;
pub use store::AssetStore;
