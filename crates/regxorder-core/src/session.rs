use serde::Serialize;

use crate::{
    ElapsedTime, InputAction, InputEvent, KeyDescriptor, MouseButton, Recording, ValidationError,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PressedInputState {
    keys: Vec<KeyDescriptor>,
    buttons: Vec<MouseButton>,
}

impl PressedInputState {
    fn note_action(&mut self, action: &InputAction) {
        match action {
            InputAction::KeyPressed { key } => {
                if !self.is_key_pressed(key) {
                    self.keys.push(key.clone());
                }
            }
            InputAction::KeyReleased { key } => {
                self.keys.retain(|pressed| pressed != key);
            }
            InputAction::MouseButtonPressed { button } => {
                if !self.is_button_pressed(*button) {
                    self.buttons.push(*button);
                }
            }
            InputAction::MouseButtonReleased { button } => {
                self.buttons.retain(|pressed| pressed != button);
            }
            InputAction::PointerMoved { .. } | InputAction::MouseWheelScrolled { .. } => {}
        }
    }

    fn is_key_pressed(&self, key: &KeyDescriptor) -> bool {
        self.keys.iter().any(|pressed| pressed == key)
    }

    fn is_button_pressed(&self, button: MouseButton) -> bool {
        self.buttons.contains(&button)
    }

    fn append_terminal_release_events(
        &self,
        events: &mut Vec<InputEvent>,
        elapsed_time: ElapsedTime,
    ) -> usize {
        let mut appended_release_events = 0_usize;

        for button in self.buttons.iter().rev() {
            push_action(
                events,
                elapsed_time,
                InputAction::MouseButtonReleased { button: *button },
            );
            appended_release_events += 1;
        }

        for key in self.keys.iter().rev() {
            push_action(
                events,
                elapsed_time,
                InputAction::KeyReleased { key: key.clone() },
            );
            appended_release_events += 1;
        }

        appended_release_events
    }
}

fn push_action(events: &mut Vec<InputEvent>, elapsed_time: ElapsedTime, action: InputAction) {
    events.push(InputEvent {
        sequence: events.len() as u64,
        elapsed_time,
        action,
    });
}

/// A summary of synthetic releases appended while finalizing a captured recording session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct RecordingFinalizationReport {
    pub appended_release_events: usize,
}

/// Finalizes a captured recording so the canonical event stream ends in a fully released state.
pub fn finalize_recording_session(
    recording: Recording,
) -> Result<(Recording, RecordingFinalizationReport), ValidationError> {
    let boundary_elapsed_time = recording.duration();
    let (metadata, mut events) = recording.into_parts();
    let mut pressed_input_state = PressedInputState::default();

    for event in &events {
        pressed_input_state.note_action(&event.action);
    }

    let appended_release_events =
        pressed_input_state.append_terminal_release_events(&mut events, boundary_elapsed_time);

    Ok((
        Recording::new(metadata, events)?,
        RecordingFinalizationReport {
            appended_release_events,
        },
    ))
}

/// A summary of the semantic cleanup applied while building a derived playback plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct PlaybackPreparationReport {
    pub skipped_unmatched_release_events: usize,
    pub appended_release_events: usize,
}

/// A derived playback plan that can differ from the canonical saved recording.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedPlaybackPlan {
    recording: Recording,
    report: PlaybackPreparationReport,
}

impl PreparedPlaybackPlan {
    /// Returns the validated recording-shaped event stream to dispatch.
    pub fn recording(&self) -> &Recording {
        &self.recording
    }

    /// Returns a summary of the cleanup rules applied during plan preparation.
    pub const fn report(&self) -> PlaybackPreparationReport {
        self.report
    }

    /// Returns the number of prepared events that will be dispatched.
    pub fn event_count(&self) -> usize {
        self.recording.event_count()
    }
}

/// Builds a derived playback plan without mutating the canonical saved recording.
pub fn prepare_playback_plan(
    recording: &Recording,
) -> Result<PreparedPlaybackPlan, ValidationError> {
    let mut prepared_events = Vec::with_capacity(recording.event_count());
    let mut pressed_input_state = PressedInputState::default();
    let mut skipped_unmatched_release_events = 0_usize;

    for event in recording.events() {
        if should_skip_release_event(&pressed_input_state, &event.action) {
            skipped_unmatched_release_events += 1;
            continue;
        }

        push_action(
            &mut prepared_events,
            event.elapsed_time,
            event.action.clone(),
        );
        pressed_input_state.note_action(&event.action);
    }

    let appended_release_events = pressed_input_state
        .append_terminal_release_events(&mut prepared_events, recording.duration());

    Ok(PreparedPlaybackPlan {
        recording: Recording::new(recording.metadata().clone(), prepared_events)?,
        report: PlaybackPreparationReport {
            skipped_unmatched_release_events,
            appended_release_events,
        },
    })
}

fn should_skip_release_event(
    pressed_input_state: &PressedInputState,
    action: &InputAction,
) -> bool {
    match action {
        InputAction::KeyReleased { key } => !pressed_input_state.is_key_pressed(key),
        InputAction::MouseButtonReleased { button } => {
            !pressed_input_state.is_button_pressed(*button)
        }
        InputAction::KeyPressed { .. }
        | InputAction::MouseButtonPressed { .. }
        | InputAction::PointerMoved { .. }
        | InputAction::MouseWheelScrolled { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AbsoluteScreenPoint, DisplayMetadata, ElapsedTime, InputAction, InputEvent, KeyDescriptor,
        MouseButton, Recording, ScanCode, SchemaVersion, ScreenSize, recording::RecordingMetadata,
    };

    use super::{finalize_recording_session, prepare_playback_plan};

    fn sample_metadata() -> RecordingMetadata {
        RecordingMetadata {
            schema_version: SchemaVersion::new(1),
            title: Some(String::from("Session sample")),
            display: DisplayMetadata {
                virtual_origin: AbsoluteScreenPoint { x: 0, y: 0 },
                virtual_size: ScreenSize {
                    width: 1920,
                    height: 1080,
                },
                monitors: Vec::new(),
            },
        }
    }

    fn sample_key_descriptor() -> KeyDescriptor {
        KeyDescriptor {
            scan_code: ScanCode::new(30),
            logical_name: Some(String::from("A")),
            extended: false,
        }
    }

    #[test]
    fn recording_finalization_appends_balancing_release_events() {
        let recording = Recording::new(
            sample_metadata(),
            vec![
                InputEvent {
                    sequence: 0,
                    elapsed_time: ElapsedTime::from_micros(0),
                    action: InputAction::KeyPressed {
                        key: sample_key_descriptor(),
                    },
                },
                InputEvent {
                    sequence: 1,
                    elapsed_time: ElapsedTime::from_micros(5_000),
                    action: InputAction::MouseButtonPressed {
                        button: MouseButton::Left,
                    },
                },
            ],
        )
        .expect("sample recording should be valid");

        let (finalized_recording, report) =
            finalize_recording_session(recording).expect("recording finalization should succeed");

        assert_eq!(report.appended_release_events, 2);
        assert_eq!(finalized_recording.event_count(), 4);
        assert!(matches!(
            finalized_recording.events()[2].action,
            InputAction::MouseButtonReleased {
                button: MouseButton::Left
            }
        ));
        assert!(matches!(
            finalized_recording.events()[3].action,
            InputAction::KeyReleased { .. }
        ));
        assert_eq!(
            finalized_recording.events()[2].elapsed_time.as_micros(),
            5_000
        );
        assert_eq!(
            finalized_recording.events()[3].elapsed_time.as_micros(),
            5_000
        );
    }

    #[test]
    fn playback_preparation_skips_unmatched_releases_and_appends_terminal_cleanup() {
        let recording = Recording::new(
            sample_metadata(),
            vec![
                InputEvent {
                    sequence: 0,
                    elapsed_time: ElapsedTime::from_micros(0),
                    action: InputAction::KeyReleased {
                        key: sample_key_descriptor(),
                    },
                },
                InputEvent {
                    sequence: 1,
                    elapsed_time: ElapsedTime::from_micros(1_000),
                    action: InputAction::KeyPressed {
                        key: sample_key_descriptor(),
                    },
                },
                InputEvent {
                    sequence: 2,
                    elapsed_time: ElapsedTime::from_micros(2_000),
                    action: InputAction::MouseButtonReleased {
                        button: MouseButton::Left,
                    },
                },
                InputEvent {
                    sequence: 3,
                    elapsed_time: ElapsedTime::from_micros(3_000),
                    action: InputAction::MouseButtonPressed {
                        button: MouseButton::Left,
                    },
                },
            ],
        )
        .expect("sample recording should be valid");

        let prepared_plan =
            prepare_playback_plan(&recording).expect("playback preparation should succeed");

        assert_eq!(recording.event_count(), 4);
        assert_eq!(prepared_plan.report().skipped_unmatched_release_events, 2);
        assert_eq!(prepared_plan.report().appended_release_events, 2);
        assert_eq!(prepared_plan.event_count(), 4);
        assert!(matches!(
            prepared_plan.recording().events()[0].action,
            InputAction::KeyPressed { .. }
        ));
        assert!(matches!(
            prepared_plan.recording().events()[1].action,
            InputAction::MouseButtonPressed {
                button: MouseButton::Left
            }
        ));
        assert!(matches!(
            prepared_plan.recording().events()[2].action,
            InputAction::MouseButtonReleased {
                button: MouseButton::Left
            }
        ));
        assert!(matches!(
            prepared_plan.recording().events()[3].action,
            InputAction::KeyReleased { .. }
        ));
    }

    #[test]
    fn playback_preparation_preserves_repeated_key_presses() {
        let recording = Recording::new(
            sample_metadata(),
            vec![
                InputEvent {
                    sequence: 0,
                    elapsed_time: ElapsedTime::from_micros(0),
                    action: InputAction::KeyPressed {
                        key: sample_key_descriptor(),
                    },
                },
                InputEvent {
                    sequence: 1,
                    elapsed_time: ElapsedTime::from_micros(500),
                    action: InputAction::KeyPressed {
                        key: sample_key_descriptor(),
                    },
                },
                InputEvent {
                    sequence: 2,
                    elapsed_time: ElapsedTime::from_micros(1_000),
                    action: InputAction::KeyReleased {
                        key: sample_key_descriptor(),
                    },
                },
            ],
        )
        .expect("sample recording should be valid");

        let prepared_plan =
            prepare_playback_plan(&recording).expect("playback preparation should succeed");

        assert_eq!(prepared_plan.report().skipped_unmatched_release_events, 0);
        assert_eq!(prepared_plan.report().appended_release_events, 0);
        assert_eq!(prepared_plan.event_count(), 3);
        assert!(matches!(
            prepared_plan.recording().events()[1].action,
            InputAction::KeyPressed { .. }
        ));
    }
}
