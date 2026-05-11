use serde::Serialize;

use crate::{
    HotkeyBinding, PlaybackPreparationReport, Recording, RecordingMetrics, SpeedMultiplier,
    ValidationError, prepare_playback_plan,
};

/// The severity of a single doctor or diagnostics check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStatus {
    Pass,
    Warn,
    Fail,
}

/// A single health check emitted by the doctor workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticCheck {
    pub name: String,
    pub status: DiagnosticStatus,
    pub summary: String,
}

impl DiagnosticCheck {
    /// Creates a passing diagnostic check.
    pub fn pass(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: DiagnosticStatus::Pass,
            summary: summary.into(),
        }
    }

    /// Creates a warning diagnostic check.
    pub fn warn(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: DiagnosticStatus::Warn,
            summary: summary.into(),
        }
    }

    /// Creates a failing diagnostic check.
    pub fn fail(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: DiagnosticStatus::Fail,
            summary: summary.into(),
        }
    }
}

/// A compact aggregate over a set of diagnostic checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct DiagnosticSummary {
    pub pass_count: usize,
    pub warn_count: usize,
    pub fail_count: usize,
}

impl DiagnosticSummary {
    /// Builds a summary from a list of checks.
    pub fn from_checks(checks: &[DiagnosticCheck]) -> Self {
        let mut summary = Self::default();

        for check in checks {
            match check.status {
                DiagnosticStatus::Pass => summary.pass_count += 1,
                DiagnosticStatus::Warn => summary.warn_count += 1,
                DiagnosticStatus::Fail => summary.fail_count += 1,
            }
        }

        summary
    }

    /// Returns the highest-severity status represented by this summary.
    pub const fn overall_status(self) -> DiagnosticStatus {
        if self.fail_count > 0 {
            DiagnosticStatus::Fail
        } else if self.warn_count > 0 {
            DiagnosticStatus::Warn
        } else {
            DiagnosticStatus::Pass
        }
    }

    /// Reports whether any failing checks are present.
    pub const fn has_failures(self) -> bool {
        self.fail_count > 0
    }
}

/// A shared environment report emitted by platform backends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvironmentDoctorReport {
    pub target_os: String,
    pub target_arch: String,
    pub recording_backends: Vec<String>,
    pub playback_backend: String,
    pub hotkey_backend: String,
    pub hotkey_probe: HotkeyBinding,
    pub summary: DiagnosticSummary,
    pub checks: Vec<DiagnosticCheck>,
}

impl EnvironmentDoctorReport {
    /// Creates a structured environment doctor report.
    pub fn new(
        target_os: String,
        target_arch: String,
        recording_backends: Vec<String>,
        playback_backend: String,
        hotkey_backend: String,
        hotkey_probe: HotkeyBinding,
        checks: Vec<DiagnosticCheck>,
    ) -> Self {
        let summary = DiagnosticSummary::from_checks(&checks);

        Self {
            target_os,
            target_arch,
            recording_backends,
            playback_backend,
            hotkey_backend,
            hotkey_probe,
            summary,
            checks,
        }
    }
}

/// A structured recording inspection report for doctor-style checks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordingDoctorReport {
    pub title: Option<String>,
    pub schema_version: u32,
    pub event_count: usize,
    pub duration_micros: u64,
    pub average_events_per_second: f64,
    pub metrics: RecordingMetrics,
    pub summary: DiagnosticSummary,
    pub checks: Vec<DiagnosticCheck>,
}

/// A structured playback readiness report for doctor-style checks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaybackDoctorReport {
    pub canonical_event_count: usize,
    pub prepared_event_count: usize,
    pub speed_multiplier: SpeedMultiplier,
    pub preparation_report: PlaybackPreparationReport,
    pub summary: DiagnosticSummary,
    pub checks: Vec<DiagnosticCheck>,
}

/// A structured control-loop readiness report shared across frontends.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ControlDoctorReport {
    pub start_record_hotkey: Option<HotkeyBinding>,
    pub start_play_hotkey: Option<HotkeyBinding>,
    pub stop_hotkey: HotkeyBinding,
    pub summary: DiagnosticSummary,
    pub checks: Vec<DiagnosticCheck>,
}

impl ControlDoctorReport {
    /// Creates a structured control-loop readiness report.
    pub fn new(
        start_record_hotkey: Option<HotkeyBinding>,
        start_play_hotkey: Option<HotkeyBinding>,
        stop_hotkey: HotkeyBinding,
        checks: Vec<DiagnosticCheck>,
    ) -> Self {
        let summary = DiagnosticSummary::from_checks(&checks);

        Self {
            start_record_hotkey,
            start_play_hotkey,
            stop_hotkey,
            summary,
            checks,
        }
    }
}

/// Builds a recording doctor report from a validated canonical recording.
pub fn diagnose_recording(recording: &Recording) -> RecordingDoctorReport {
    let metrics = recording.metrics();
    let mut checks = Vec::with_capacity(3);

    checks.push(DiagnosticCheck::pass(
        "schema_version",
        format!(
            "schema version {} matches this build",
            recording.metadata().schema_version.get()
        ),
    ));

    checks.push(DiagnosticCheck::pass(
        "event_ordering",
        format!(
            "validated canonical event ordering across {} events",
            recording.event_count()
        ),
    ));

    checks.push(if recording.event_count() == 0 {
        DiagnosticCheck::warn(
            "recording_content",
            "recording is valid but contains no events to replay",
        )
    } else {
        DiagnosticCheck::pass(
            "recording_content",
            format!(
                "recording spans {} us across {} events",
                recording.duration().as_micros(),
                recording.event_count()
            ),
        )
    });

    RecordingDoctorReport {
        title: recording.metadata().title.clone(),
        schema_version: recording.metadata().schema_version.get(),
        event_count: recording.event_count(),
        duration_micros: recording.duration().as_micros(),
        average_events_per_second: average_events_per_second(&metrics),
        metrics,
        summary: DiagnosticSummary::from_checks(&checks),
        checks,
    }
}

/// Builds a playback readiness report from a validated canonical recording and speed.
pub fn diagnose_playback(
    recording: &Recording,
    speed_multiplier: SpeedMultiplier,
) -> Result<PlaybackDoctorReport, ValidationError> {
    let prepared_playback_plan = prepare_playback_plan(recording)?;
    let preparation_report = prepared_playback_plan.report();
    let mut checks = Vec::with_capacity(3);

    checks.push(DiagnosticCheck::pass(
        "speed_multiplier",
        format!("playback speed is set to {}x", speed_multiplier.get()),
    ));

    checks.push(if recording.event_count() == 0 {
        DiagnosticCheck::warn(
            "canonical_recording",
            "recording is valid but contains no events to dispatch",
        )
    } else {
        DiagnosticCheck::pass(
            "canonical_recording",
            format!(
                "canonical recording contains {} events over {} us",
                recording.event_count(),
                recording.duration().as_micros()
            ),
        )
    });

    checks.push(
        if preparation_report.skipped_unmatched_release_events > 0
            || preparation_report.appended_release_events > 0
        {
            DiagnosticCheck::warn(
                "prepared_playback_plan",
                format!(
                    "prepared playback skipped {} unmatched releases and appended {} terminal cleanup releases",
                    preparation_report.skipped_unmatched_release_events,
                    preparation_report.appended_release_events
                ),
            )
        } else {
            DiagnosticCheck::pass(
                "prepared_playback_plan",
                format!(
                    "prepared playback matches the canonical recording with {} dispatchable events",
                    prepared_playback_plan.event_count()
                ),
            )
        },
    );

    Ok(PlaybackDoctorReport {
        canonical_event_count: recording.event_count(),
        prepared_event_count: prepared_playback_plan.event_count(),
        speed_multiplier,
        preparation_report,
        summary: DiagnosticSummary::from_checks(&checks),
        checks,
    })
}

fn average_events_per_second(metrics: &RecordingMetrics) -> f64 {
    if metrics.total_events == 0 {
        return 0.0;
    }

    let duration_micros = metrics.duration_micros.max(1);
    metrics.total_events as f64 * 1_000_000.0 / duration_micros as f64
}

#[cfg(test)]
mod tests {
    use crate::{
        AbsoluteScreenPoint, DisplayMetadata, ElapsedTime, InputAction, InputEvent, KeyDescriptor,
        MouseButton, Recording, ScanCode, SchemaVersion, ScreenSize, recording::RecordingMetadata,
    };

    use super::{DiagnosticStatus, diagnose_playback, diagnose_recording};

    fn sample_metadata() -> RecordingMetadata {
        RecordingMetadata {
            schema_version: SchemaVersion::new(1),
            title: Some(String::from("Doctor sample")),
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
    fn recording_doctor_warns_about_empty_recordings() {
        let recording = Recording::new(sample_metadata(), Vec::new())
            .expect("empty recordings are still valid doctor inputs");

        let report = diagnose_recording(&recording);

        assert_eq!(report.summary.warn_count, 1);
        assert_eq!(report.summary.overall_status(), DiagnosticStatus::Warn);
        assert_eq!(report.event_count, 0);
    }

    #[test]
    fn playback_doctor_reports_cleanup_when_preparation_changes_the_plan() {
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
                    action: InputAction::MouseButtonPressed {
                        button: MouseButton::Left,
                    },
                },
            ],
        )
        .expect("sample recording should be valid");

        let report = diagnose_playback(
            &recording,
            crate::SpeedMultiplier::new(1.0).expect("speed should be valid"),
        )
        .expect("doctor playback report should build");

        assert_eq!(report.summary.warn_count, 1);
        assert_eq!(report.summary.overall_status(), DiagnosticStatus::Warn);
        assert_eq!(
            report.preparation_report.skipped_unmatched_release_events,
            1
        );
        assert_eq!(report.preparation_report.appended_release_events, 1);
        assert_eq!(report.prepared_event_count, 2);
    }
}
