//! Platform-agnostic domain types and serialization helpers for regxorder.

mod diagnostics;
mod error;
mod event;
mod format;
mod hotkey;
mod recording;
mod session;

pub use diagnostics::{
    ControlDoctorReport, DiagnosticCheck, DiagnosticStatus, DiagnosticSummary,
    EnvironmentDoctorReport, PlaybackDoctorReport, RecordingDoctorReport, diagnose_playback,
    diagnose_recording,
};
pub use error::{RecordingError, ValidationError};
pub use event::{
    AbsoluteScreenPoint, DisplayMetadata, Dpi, ElapsedTime, InputAction, InputEvent, KeyDescriptor,
    MonitorDescriptor, MouseButton, NormalizedCoordinate, NormalizedScreenPoint, PointerPosition,
    ScanCode, SchemaVersion, ScreenSize, ScrollAxis, SpeedMultiplier,
};
pub use format::{from_json_str, to_json_pretty, write_json_pretty};
pub use hotkey::{HotkeyBinding, HotkeyKey, HotkeyModifier, HotkeyParseError};
pub use recording::{Recording, RecordingActionCounts, RecordingMetadata, RecordingMetrics};
pub use session::{
    PlaybackPreparationReport, PreparedPlaybackPlan, RecordingFinalizationReport,
    finalize_recording_session, prepare_playback_plan,
};

/// The canonical schema version used for regxorder interchange data.
pub const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1);
