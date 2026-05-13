use regxorder_core::{DiagnosticCheck, EnvironmentDoctorReport, HotkeyBinding};

use crate::{
    HotkeyRegistration, ProcessElevationStatus, WindowsBackendError,
    current_process_elevation_status, hotkeys::probe_hotkey_registration,
};

const DOCTOR_HOTKEY_PROBE_IDENTIFIER: i32 = 91;

/// Builds a structured environment doctor report for the Windows backend surface.
pub fn diagnose_windows_environment(hotkey_probe: HotkeyBinding) -> EnvironmentDoctorReport {
    let hotkey_probe_result = probe_hotkey_registration(HotkeyRegistration::from_binding(
        DOCTOR_HOTKEY_PROBE_IDENTIFIER,
        &hotkey_probe,
    ));
    let process_elevation_result = current_process_elevation_status();

    build_environment_report(hotkey_probe, hotkey_probe_result, process_elevation_result)
}

fn build_environment_report(
    hotkey_probe: HotkeyBinding,
    hotkey_probe_result: Result<(), WindowsBackendError>,
    process_elevation_result: Result<ProcessElevationStatus, WindowsBackendError>,
) -> EnvironmentDoctorReport {
    let mut checks = Vec::with_capacity(5);

    checks.push(DiagnosticCheck::pass(
        "recording_backends",
        "Raw Input and low-level hook recording backends are compiled into this build",
    ));
    checks.push(DiagnosticCheck::pass(
        "playback_backend",
        "SendInput playback backend is compiled into this build",
    ));
    checks.push(DiagnosticCheck::pass(
        "hotkey_backend",
        "RegisterHotKey global hotkey backend is compiled into this build",
    ));
    checks.push(match hotkey_probe_result {
        Ok(()) => DiagnosticCheck::pass(
            "hotkey_probe",
            format!(
                "temporary RegisterHotKey probe succeeded for {}",
                hotkey_probe
            ),
        ),
        Err(error) => DiagnosticCheck::fail(
            "hotkey_probe",
            format!(
                "temporary RegisterHotKey probe failed for {}: {}",
                hotkey_probe, error
            ),
        ),
    });
    checks.push(match process_elevation_result {
        Ok(ProcessElevationStatus::Elevated) => DiagnosticCheck::pass(
            "process_elevation",
            "the current regxorder process is running elevated",
        ),
        Ok(ProcessElevationStatus::NotElevated) => DiagnosticCheck::warn(
            "process_elevation",
            "the current regxorder process is not elevated; playback into elevated targets may be blocked by UIPI",
        ),
        Err(error) => DiagnosticCheck::fail(
            "process_elevation",
            format!("failed to query current process elevation: {error}"),
        ),
    });

    EnvironmentDoctorReport::new(
        std::env::consts::OS.to_string(),
        std::env::consts::ARCH.to_string(),
        vec![String::from("raw_input"), String::from("low_level_hooks")],
        String::from("send_input"),
        String::from("register_hotkey"),
        hotkey_probe,
        checks,
    )
}

#[cfg(test)]
mod tests {
    use regxorder_core::HotkeyBinding;

    use super::{ProcessElevationStatus, build_environment_report};
    use crate::WindowsBackendError;

    #[test]
    fn environment_doctor_marks_failed_hotkey_probes_as_failures() {
        let hotkey_probe = "ctrl+alt+shift+f12"
            .parse::<HotkeyBinding>()
            .expect("doctor probe hotkey should parse");

        let report = build_environment_report(
            hotkey_probe,
            Err(WindowsBackendError::Internal("probe failed")),
            Ok(ProcessElevationStatus::Elevated),
        );

        assert_eq!(report.summary.fail_count, 1);
        assert_eq!(report.checks[3].name, "hotkey_probe");
    }

    #[test]
    fn environment_doctor_warns_when_the_process_is_not_elevated() {
        let hotkey_probe = "ctrl+alt+shift+f12"
            .parse::<HotkeyBinding>()
            .expect("doctor probe hotkey should parse");

        let report = build_environment_report(
            hotkey_probe,
            Ok(()),
            Ok(ProcessElevationStatus::NotElevated),
        );

        assert_eq!(report.summary.warn_count, 1);
        assert_eq!(report.checks[4].name, "process_elevation");
    }
}
