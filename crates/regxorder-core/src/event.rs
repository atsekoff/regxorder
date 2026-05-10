use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{ValidationError, CURRENT_SCHEMA_VERSION};

/// A version marker for the canonical regxorder recording schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchemaVersion(u32);

impl SchemaVersion {
    /// Creates a new schema version value.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw schema version number.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Default for SchemaVersion {
    fn default() -> Self {
        CURRENT_SCHEMA_VERSION
    }
}

/// A physical keyboard scan code suitable for deterministic replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScanCode(u16);

impl ScanCode {
    /// Creates a new physical scan code.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the raw scan code value.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A relative time offset stored in microseconds from recording start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventOffset(u64);

impl EventOffset {
    /// Creates a relative time offset in microseconds.
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros)
    }

    /// Returns the raw offset value in microseconds.
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// Converts the offset to a standard library duration.
    pub fn as_duration(self) -> Duration {
        Duration::from_micros(self.0)
    }
}

/// A validated playback speed multiplier.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct SpeedMultiplier(f64);

impl SpeedMultiplier {
    /// Validates and creates a playback speed multiplier.
    pub fn new(value: f64) -> Result<Self, ValidationError> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(ValidationError::InvalidSpeedMultiplier { value })
        }
    }

    /// Returns the inner multiplier value.
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for SpeedMultiplier {
    type Error = ValidationError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SpeedMultiplier> for f64 {
    fn from(value: SpeedMultiplier) -> Self {
        value.0
    }
}

/// A validated normalized screen coordinate in the unit interval.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct NormalizedCoordinate(f64);

impl NormalizedCoordinate {
    /// Validates and creates a normalized coordinate.
    pub fn new(value: f64, axis: &'static str) -> Result<Self, ValidationError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ValidationError::InvalidNormalizedCoordinate { axis, value })
        }
    }

    /// Returns the normalized coordinate value.
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for NormalizedCoordinate {
    type Error = ValidationError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value, "value")
    }
}

impl From<NormalizedCoordinate> for f64 {
    fn from(value: NormalizedCoordinate) -> Self {
        value.0
    }
}

/// A point on the virtual desktop in absolute screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbsoluteScreenPoint {
    pub x: i32,
    pub y: i32,
}

/// A point normalized against the recorded virtual desktop bounds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormalizedScreenPoint {
    pub x: NormalizedCoordinate,
    pub y: NormalizedCoordinate,
}

/// Exact and portable cursor coordinates captured for a pointer event.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerPosition {
    pub absolute: AbsoluteScreenPoint,
    pub normalized: NormalizedScreenPoint,
}

/// A width and height pair for the recorded virtual desktop or monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenSize {
    pub width: u32,
    pub height: u32,
}

impl ScreenSize {
    pub(crate) fn validate(self, context: &'static str) -> Result<(), ValidationError> {
        if self.width == 0 || self.height == 0 {
            Err(ValidationError::ZeroSizedSurface {
                context,
                width: self.width,
                height: self.height,
            })
        } else {
            Ok(())
        }
    }
}

/// Horizontal and vertical DPI values captured for a monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dpi {
    pub x: u32,
    pub y: u32,
}

/// Monitor metadata captured with a recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorDescriptor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub origin: AbsoluteScreenPoint,
    pub size: ScreenSize,
    pub dpi: Dpi,
}

impl MonitorDescriptor {
    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        self.size.validate("monitor")
    }
}

/// The virtual desktop layout that a recording was captured against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayMetadata {
    pub virtual_origin: AbsoluteScreenPoint,
    pub virtual_size: ScreenSize,
    #[serde(default)]
    pub monitors: Vec<MonitorDescriptor>,
}

impl DisplayMetadata {
    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        self.virtual_size.validate("virtual display")?;

        for monitor in &self.monitors {
            monitor.validate()?;
        }

        Ok(())
    }
}

/// A key description that remains stable enough for deterministic playback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyDescriptor {
    pub scan_code: ScanCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_name: Option<String>,
    #[serde(default)]
    pub extended: bool,
}

/// Mouse buttons supported by the canonical recording model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

/// Scroll axes used by wheel events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollAxis {
    Vertical,
    Horizontal,
}

/// A single logical user input action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputAction {
    KeyPressed { key: KeyDescriptor },
    KeyReleased { key: KeyDescriptor },
    PointerMoved { position: PointerPosition },
    MouseButtonPressed { button: MouseButton },
    MouseButtonReleased { button: MouseButton },
    MouseWheelScrolled { axis: ScrollAxis, delta: i32 },
}

/// A canonical input event with stable ordering and relative timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputEvent {
    pub sequence: u64,
    pub offset: EventOffset,
    pub action: InputAction,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_coordinate_rejects_values_outside_the_unit_interval() {
        for invalid in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            let result = NormalizedCoordinate::new(invalid, "x");
            assert!(
                matches!(
                    result,
                    Err(ValidationError::InvalidNormalizedCoordinate { .. })
                ),
                "unexpected validation result for {invalid}"
            );
        }
    }

    #[test]
    fn speed_multiplier_rejects_zero_negative_and_non_finite_values() {
        for invalid in [0.0, -0.5, f64::NAN, f64::NEG_INFINITY] {
            let result = SpeedMultiplier::new(invalid);
            assert!(
                matches!(result, Err(ValidationError::InvalidSpeedMultiplier { .. })),
                "unexpected validation result for {invalid}"
            );
        }
    }
}
