//! Device output consumes immutable CPU audio frames, independently of scenes and asset stores.
mod frame;
mod output;
pub use comet_asset_data::AudioLibrary;
pub use frame::{
    AudioFrame, AudioVoice, ListenerPose, MAX_TOTAL_SAMPLES, MAX_VOICES, SpatialVoice,
    validate_library,
};
pub use output::{AudioReport, AudioState, AudioVoiceReport, Output};
