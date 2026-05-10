//! Windows-specific recording, playback, and hotkey backends for regxorder.

mod error;
mod playback;

pub use error::WindowsBackendError;
pub use playback::{PlaybackReport, play_recording};
