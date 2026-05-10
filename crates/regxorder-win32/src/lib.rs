//! Windows-specific recording, playback, and hotkey backends for regxorder.

mod error;
mod playback;
mod recording;

pub use error::WindowsBackendError;
pub use playback::{PlaybackReport, play_recording};
pub use recording::record_with_low_level_hooks;
