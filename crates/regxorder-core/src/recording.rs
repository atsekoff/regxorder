use serde::{Deserialize, Serialize};
use std::io::Write;

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

/// Counts of canonical action kinds contained in a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecordingActionCounts {
    pub key_pressed_events: usize,
    pub key_released_events: usize,
    pub pointer_moved_events: usize,
    pub mouse_button_pressed_events: usize,
    pub mouse_button_released_events: usize,
    pub mouse_wheel_scrolled_events: usize,
}

impl RecordingActionCounts {
    pub const fn total_events(self) -> usize {
        self.key_pressed_events
            + self.key_released_events
            + self.pointer_moved_events
            + self.mouse_button_pressed_events
            + self.mouse_button_released_events
            + self.mouse_wheel_scrolled_events
    }
}

/// Lightweight metrics computed from an in-memory recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecordingMetrics {
    pub total_events: usize,
    pub duration_micros: u64,
    pub peak_events_per_second: usize,
    pub action_counts: RecordingActionCounts,
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

    /// Returns the number of ordered events stored in the recording.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Returns the total relative duration of the recording.
    pub fn duration(&self) -> crate::ElapsedTime {
        self.events
            .last()
            .map(|event| event.elapsed_time)
            .unwrap_or_else(|| crate::ElapsedTime::from_micros(0))
    }

    /// Returns lightweight metrics that summarize event density and action mix.
    pub fn metrics(&self) -> RecordingMetrics {
        let mut action_counts = RecordingActionCounts::default();
        let mut current_second_bucket = None;
        let mut current_second_count = 0_usize;
        let mut peak_events_per_second = 0_usize;

        for event in &self.events {
            match &event.action {
                crate::InputAction::KeyPressed { .. } => action_counts.key_pressed_events += 1,
                crate::InputAction::KeyReleased { .. } => action_counts.key_released_events += 1,
                crate::InputAction::PointerMoved { .. } => action_counts.pointer_moved_events += 1,
                crate::InputAction::MouseButtonPressed { .. } => {
                    action_counts.mouse_button_pressed_events += 1;
                }
                crate::InputAction::MouseButtonReleased { .. } => {
                    action_counts.mouse_button_released_events += 1;
                }
                crate::InputAction::MouseWheelScrolled { .. } => {
                    action_counts.mouse_wheel_scrolled_events += 1;
                }
            }

            let second_bucket = event.elapsed_time.as_micros() / 1_000_000;
            if current_second_bucket == Some(second_bucket) {
                current_second_count += 1;
            } else {
                peak_events_per_second = peak_events_per_second.max(current_second_count);
                current_second_bucket = Some(second_bucket);
                current_second_count = 1;
            }
        }

        peak_events_per_second = peak_events_per_second.max(current_second_count);

        RecordingMetrics {
            total_events: self.event_count(),
            duration_micros: self.duration().as_micros(),
            peak_events_per_second,
            action_counts,
        }
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

        let mut previous_elapsed_time = None;

        for (index, event) in self.events.iter().enumerate() {
            let expected_sequence = index as u64;

            if event.sequence != expected_sequence {
                return Err(ValidationError::NonConsecutiveSequence {
                    expected: expected_sequence,
                    found: event.sequence,
                });
            }

            if let Some(previous_elapsed_time_micros) = previous_elapsed_time {
                if event.elapsed_time.as_micros() < previous_elapsed_time_micros {
                    return Err(ValidationError::NonMonotonicElapsedTime {
                        previous_micros: previous_elapsed_time_micros,
                        found_micros: event.elapsed_time.as_micros(),
                    });
                }
            }

            previous_elapsed_time = Some(event.elapsed_time.as_micros());
        }

        Ok(())
    }

    /// Serializes the recording as canonical pretty-printed JSON.
    pub fn to_json_pretty(&self) -> Result<String, RecordingError> {
        crate::to_json_pretty(self)
    }

    /// Serializes the recording as canonical pretty-printed JSON into the provided writer.
    pub fn write_json_pretty<W>(&self, writer: W) -> Result<(), RecordingError>
    where
        W: Write,
    {
        crate::write_json_pretty(self, writer)
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
        AbsoluteScreenPoint, ElapsedTime, InputAction, KeyDescriptor, MouseButton, ScanCode,
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
                elapsed_time: ElapsedTime::from_micros(10_000),
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
                elapsed_time: ElapsedTime::from_micros(12_000),
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
    fn recording_validation_rejects_non_monotonic_elapsed_times() {
        let mut events = sample_events();
        events[1].elapsed_time = ElapsedTime::from_micros(9_000);

        let error = Recording::new(sample_metadata(), events)
            .expect_err("recording should reject reversed timing");

        assert_eq!(
            error,
            ValidationError::NonMonotonicElapsedTime {
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

    #[test]
    fn recording_duration_matches_the_last_event_elapsed_time() {
        let recording =
            Recording::new(sample_metadata(), sample_events()).expect("sample recording is valid");

        assert_eq!(recording.event_count(), 2);
        assert_eq!(recording.duration().as_micros(), 12_000);
    }

    #[test]
    fn recording_metrics_track_action_counts_and_peak_second_density() {
        let recording = Recording::new(
            sample_metadata(),
            vec![
                InputEvent {
                    sequence: 0,
                    elapsed_time: ElapsedTime::from_micros(10_000),
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
                    elapsed_time: ElapsedTime::from_micros(20_000),
                    action: InputAction::PointerMoved {
                        position: sample_metadata()
                            .display
                            .pointer_position_for_absolute(AbsoluteScreenPoint { x: 10, y: 20 })
                            .expect("sample position should normalize"),
                    },
                },
                InputEvent {
                    sequence: 2,
                    elapsed_time: ElapsedTime::from_micros(900_000),
                    action: InputAction::MouseWheelScrolled {
                        axis: crate::ScrollAxis::Vertical,
                        delta: 120,
                    },
                },
                InputEvent {
                    sequence: 3,
                    elapsed_time: ElapsedTime::from_micros(1_100_000),
                    action: InputAction::MouseButtonPressed {
                        button: MouseButton::Left,
                    },
                },
                InputEvent {
                    sequence: 4,
                    elapsed_time: ElapsedTime::from_micros(1_500_000),
                    action: InputAction::MouseButtonReleased {
                        button: MouseButton::Left,
                    },
                },
                InputEvent {
                    sequence: 5,
                    elapsed_time: ElapsedTime::from_micros(2_000_000),
                    action: InputAction::KeyReleased {
                        key: KeyDescriptor {
                            scan_code: ScanCode::new(30),
                            logical_name: Some(String::from("A")),
                            extended: false,
                        },
                    },
                },
            ],
        )
        .expect("metrics sample recording should be valid");

        let metrics = recording.metrics();

        assert_eq!(metrics.total_events, 6);
        assert_eq!(metrics.duration_micros, 2_000_000);
        assert_eq!(metrics.peak_events_per_second, 3);
        assert_eq!(metrics.action_counts.key_pressed_events, 1);
        assert_eq!(metrics.action_counts.key_released_events, 1);
        assert_eq!(metrics.action_counts.pointer_moved_events, 1);
        assert_eq!(metrics.action_counts.mouse_button_pressed_events, 1);
        assert_eq!(metrics.action_counts.mouse_button_released_events, 1);
        assert_eq!(metrics.action_counts.mouse_wheel_scrolled_events, 1);
        assert_eq!(metrics.action_counts.total_events(), 6);
    }
}
