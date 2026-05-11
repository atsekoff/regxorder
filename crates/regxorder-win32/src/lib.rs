//! Windows-specific recording, playback, and hotkey backends for regxorder.

mod error;
mod hotkey_service;
mod hotkeys;
mod playback;
mod recording;

pub use error::WindowsBackendError;
pub use hotkey_service::{HotkeyWaitOutcome, StopHotkeyMonitor, wait_for_hotkey_binding};
pub use hotkeys::{
    HotkeyActivation, HotkeyModifiers, HotkeyRegistration, wait_for_hotkey_activation,
};
pub use playback::{PlaybackReport, play_recording};
pub use recording::{
    RecordingStrategy, record_with_low_level_hooks, record_with_raw_input, record_with_strategy,
};
