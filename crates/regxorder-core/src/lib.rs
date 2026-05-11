//! Platform-agnostic domain types and serialization helpers for regxorder.

mod error;
mod event;
mod format;
mod hotkey;
mod recording;

pub use error::{RecordingError, ValidationError};
pub use event::{
    AbsoluteScreenPoint, DisplayMetadata, Dpi, ElapsedTime, InputAction, InputEvent, KeyDescriptor,
    MonitorDescriptor, MouseButton, NormalizedCoordinate, NormalizedScreenPoint, PointerPosition,
    ScanCode, SchemaVersion, ScreenSize, ScrollAxis, SpeedMultiplier,
};
pub use format::{from_json_str, to_json_pretty, write_json_pretty};
pub use hotkey::{HotkeyBinding, HotkeyKey, HotkeyModifier, HotkeyParseError};
pub use recording::{Recording, RecordingActionCounts, RecordingMetadata, RecordingMetrics};

/// The canonical schema version used for regxorder interchange data.
pub const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1);
