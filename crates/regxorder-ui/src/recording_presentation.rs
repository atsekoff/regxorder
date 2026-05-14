use std::path::Path;

use regxorder_core::{
    CURRENT_SCHEMA_VERSION, Recording, SpeedMultiplier, diagnose_playback, diagnose_recording,
};
use slint::SharedString;

use crate::{EditingSession, RecordingLibraryState, build_issue_lines, format_duration};

pub(super) fn display_title_for_recording(path: &Path, recording: &Recording) -> String {
    recording
        .metadata()
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            path.file_name()
                .and_then(|file_name| file_name.to_str())
                .unwrap_or("<unnamed recording>")
                .to_string()
        })
}

pub(super) fn build_recording_title_items(
    recording_library: &RecordingLibraryState,
) -> Vec<SharedString> {
    if recording_library.recordings.is_empty() {
        return vec![SharedString::from("No validated recordings found yet")];
    }

    recording_library
        .recordings
        .iter()
        .map(|recording| SharedString::from(recording.display_title()))
        .collect()
}

pub(super) fn build_recording_subtitle_items(
    recording_library: &RecordingLibraryState,
) -> Vec<SharedString> {
    if recording_library.recordings.is_empty() {
        return vec![SharedString::from(
            "Start a recording or place a JSON session in the shared sessions folder, then refresh.",
        )];
    }

    recording_library
        .recordings
        .iter()
        .map(|recording| {
            SharedString::from(format!(
                "{} · {} · {} events",
                recording
                    .path
                    .file_name()
                    .and_then(|file_name| file_name.to_str())
                    .unwrap_or("<unnamed recording>"),
                format_duration(recording.recording.duration().as_micros()),
                recording.recording.event_count(),
            ))
        })
        .collect()
}

pub(super) fn build_invalid_recordings_summary(
    recording_library: &RecordingLibraryState,
) -> String {
    if recording_library.invalid_recordings.is_empty() {
        return String::new();
    }

    recording_library
        .invalid_recordings
        .iter()
        .map(|invalid_recording| {
            format!(
                "{}: {}",
                invalid_recording
                    .path
                    .file_name()
                    .and_then(|file_name| file_name.to_str())
                    .unwrap_or("<unnamed recording>"),
                invalid_recording.reason,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn build_selected_recording_sections(
    editing_session: Option<&EditingSession>,
) -> (String, String, String, String) {
    let Some(editing_session) = editing_session else {
        return (
            String::from("No session selected"),
            String::from("Choose a session from the library to edit it here."),
            format!(
                "Playback readiness appears here after you select a schema v{} recording.",
                CURRENT_SCHEMA_VERSION.get()
            ),
            String::from("No playback warnings or failures to show."),
        );
    };

    let recording = &editing_session.working_recording;
    let recording_metrics = recording.metrics();
    let recording_doctor = diagnose_recording(recording);
    let playback_doctor = diagnose_playback(
        recording,
        SpeedMultiplier::new(1.0).expect("default playback speed should be valid"),
    );

    let selected_recording_title = editing_session.display_title();
    let selected_recording_details = format!(
        "{}\n{} events · {} · {}\nDisplay {}x{} @ {},{}\nChecks {} pass · {} warn · {} fail",
        editing_session.source_path.display(),
        recording.event_count(),
        format_duration(recording.duration().as_micros()),
        if editing_session.is_dirty() {
            "unsaved changes"
        } else {
            "saved session"
        },
        recording.metadata().display.virtual_size.width,
        recording.metadata().display.virtual_size.height,
        recording.metadata().display.virtual_origin.x,
        recording.metadata().display.virtual_origin.y,
        recording_doctor.summary.pass_count,
        recording_doctor.summary.warn_count,
        recording_doctor.summary.fail_count,
    );

    match playback_doctor {
        Ok(playback_doctor) => {
            let playback_summary = format!(
                "Ready at 1.0x · {} events · {} peak/s · cleanup {} skipped / {} appended",
                playback_doctor.canonical_event_count,
                recording_metrics.peak_events_per_second,
                playback_doctor
                    .preparation_report
                    .skipped_unmatched_release_events,
                playback_doctor.preparation_report.appended_release_events,
            );
            let playback_checks =
                build_issue_lines(&playback_doctor.checks, "No playback warnings or failures.");

            (
                selected_recording_title,
                selected_recording_details,
                playback_summary,
                playback_checks,
            )
        }
        Err(error) => (
            selected_recording_title,
            selected_recording_details,
            String::from("Playback readiness unavailable."),
            error.to_string(),
        ),
    }
}
