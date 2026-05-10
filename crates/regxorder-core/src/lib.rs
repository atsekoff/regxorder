//! Platform-agnostic domain types and serialization helpers for regxorder.

mod error;
mod event;
mod format;
mod recording;

pub use error::{RecordingError, ValidationError};
pub use event::{
    AbsoluteScreenPoint, DisplayMetadata, Dpi, EventOffset, InputAction, InputEvent, KeyDescriptor,
    MonitorDescriptor, MouseButton, NormalizedCoordinate, NormalizedScreenPoint, PointerPosition,
    ScanCode, SchemaVersion, ScreenSize, ScrollAxis, SpeedMultiplier,
};
pub use format::{from_json_str, to_json_pretty};
pub use recording::{Recording, RecordingMetadata};

/// The canonical schema version used for regxorder interchange data.
pub const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1);
