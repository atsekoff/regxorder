//! Windows-specific recording, playback, and hotkey backends for regxorder.

mod controller;
mod diagnostics;
mod error;
mod hotkey_service;
mod hotkeys;
mod playback;
mod recording;

pub use controller::{
    ControlAction, ControlBindings, ControlController, PlaybackActionOutcome,
    RecordingActionOutcome,
};
pub use diagnostics::diagnose_windows_environment;
pub use error::WindowsBackendError;
pub use hotkey_service::{
    HotkeyWaitOutcome, StopHotkeyMonitor, wait_for_hotkey_binding,
    wait_for_hotkey_press_and_release, wait_for_hotkey_release,
};
pub use hotkeys::{
    HotkeyActivation, HotkeyModifiers, HotkeyRegistration, wait_for_hotkey_activation,
};
pub use playback::{PlaybackReport, play_recording};
pub use recording::{
    RecordingStrategy, record_with_low_level_hooks, record_with_raw_input, record_with_strategy,
};
