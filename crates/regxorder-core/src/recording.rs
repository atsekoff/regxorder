use serde::{Deserialize, Serialize};

use crate::{
    CURRENT_SCHEMA_VERSION, DisplayMetadata, InputEvent, RecordingError, SchemaVersion,
    ValidationError,
};

fn default_schema_version() -> SchemaVersion {
    CURRENT_SCHEMA_VERSION
}

/// Recording-level metadata that informs validation and replay policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingMetadata {
    #[serde(default = "default_schema_version")]
    pub schema_version: SchemaVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub display: DisplayMetadata,
}

/// A canonical regxorder recording with stable ordering and validated metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    metadata: RecordingMetadata,
    #[serde(default)]
    events: Vec<InputEvent>,
}

impl Recording {
    /// Creates a validated recording.
    pub fn new(
        metadata: RecordingMetadata,
        events: Vec<InputEvent>,
    ) -> Result<Self, ValidationError> {
        let recording = Self { metadata, events };
        recording.validate()?;
        Ok(recording)
    }

    /// Returns immutable access to the recording metadata.
    pub fn metadata(&self) -> &RecordingMetadata {
        &self.metadata
    }

    /// Returns immutable access to the ordered event stream.
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }

    /// Validates the recording's invariants.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.metadata.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ValidationError::UnsupportedSchemaVersion {
                expected: CURRENT_SCHEMA_VERSION.get(),
                found: self.metadata.schema_version.get(),
            });
        }

        if self
            .metadata
            .title
            .as_ref()
            .is_some_and(|title| title.trim().is_empty())
        {
            return Err(ValidationError::EmptyTitle);
        }

        self.metadata.display.validate()?;

        let mut previous_offset = None;

        for (index, event) in self.events.iter().enumerate() {
            let expected_sequence = index as u64;

            if event.sequence != expected_sequence {
                return Err(ValidationError::NonConsecutiveSequence {
                    expected: expected_sequence,
                    found: event.sequence,
                });
            }

            if let Some(previous_offset_micros) = previous_offset {
                if event.offset.as_micros() < previous_offset_micros {
                    return Err(ValidationError::NonMonotonicOffset {
                        previous_micros: previous_offset_micros,
                        found_micros: event.offset.as_micros(),
                    });
                }
            }

            previous_offset = Some(event.offset.as_micros());
        }

        Ok(())
    }

    /// Serializes the recording as canonical pretty-printed JSON.
    pub fn to_json_pretty(&self) -> Result<String, RecordingError> {
        crate::to_json_pretty(self)
    }

    /// Parses a validated recording from canonical JSON.
    pub fn from_json_str(input: &str) -> Result<Self, RecordingError> {
        crate::from_json_str(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AbsoluteScreenPoint, EventOffset, InputAction, KeyDescriptor, MouseButton, ScanCode,
        ScreenSize,
    };

    fn sample_metadata() -> RecordingMetadata {
        RecordingMetadata {
            schema_version: SchemaVersion::new(1),
            title: Some(String::from("Validation sample")),
            display: DisplayMetadata {
                virtual_origin: AbsoluteScreenPoint { x: 0, y: 0 },
                virtual_size: ScreenSize {
                    width: 2560,
                    height: 1440,
                },
                monitors: Vec::new(),
            },
        }
    }

    fn sample_events() -> Vec<InputEvent> {
        vec![
            InputEvent {
                sequence: 0,
                offset: EventOffset::from_micros(10_000),
                action: InputAction::KeyPressed {
                    key: KeyDescriptor {
                        scan_code: ScanCode::new(30),
                        logical_name: Some(String::from("A")),
                        extended: false,
                    },
                },
            },
            InputEvent {
                sequence: 1,
                offset: EventOffset::from_micros(12_000),
                action: InputAction::MouseButtonPressed {
                    button: MouseButton::Left,
                },
            },
        ]
    }

    #[test]
    fn recording_validation_rejects_non_consecutive_sequences() {
        let mut events = sample_events();
        events[1].sequence = 3;

        let error = Recording::new(sample_metadata(), events)
            .expect_err("recording should reject sequence gaps");

        assert_eq!(
            error,
            ValidationError::NonConsecutiveSequence {
                expected: 1,
                found: 3,
            }
        );
    }

    #[test]
    fn recording_validation_rejects_non_monotonic_offsets() {
        let mut events = sample_events();
        events[1].offset = EventOffset::from_micros(9_000);

        let error = Recording::new(sample_metadata(), events)
            .expect_err("recording should reject reversed timing");

        assert_eq!(
            error,
            ValidationError::NonMonotonicOffset {
                previous_micros: 10_000,
                found_micros: 9_000,
            }
        );
    }

    #[test]
    fn recording_validation_rejects_zero_sized_virtual_displays() {
        let mut metadata = sample_metadata();
        metadata.display.virtual_size.width = 0;

        let error = Recording::new(metadata, sample_events())
            .expect_err("recording should reject zero-sized displays");

        assert_eq!(
            error,
            ValidationError::ZeroSizedSurface {
                context: "virtual display",
                width: 0,
                height: 1440,
            }
        );
    }
}
