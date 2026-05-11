use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

fn regxorder_cli_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_regxorder-cli"))
}

fn unique_temp_path(file_name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the unix epoch")
        .as_nanos();

    std::env::temp_dir().join(format!("regxorder-{timestamp}-{file_name}"))
}

#[test]
fn doctor_recording_returns_a_non_zero_exit_code_for_missing_files() {
    let missing_path = unique_temp_path("missing-recording.json");

    let output = regxorder_cli_binary()
        .args([
            "doctor",
            "recording",
            "--input",
            missing_path
                .to_str()
                .expect("temporary paths should be valid UTF-8"),
        ])
        .output()
        .expect("doctor recording command should run");

    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.contains("doctor target: recording"));
    assert!(stdout.contains("overall_status: fail"));
    assert!(stderr.contains("doctor reported one or more failing checks"));
}

#[test]
fn diagnostics_recording_json_includes_detailed_metadata_for_a_sample_recording() {
    let sample_path = unique_temp_path("diagnostics-sample.json");

    let sample_output = regxorder_cli_binary()
        .args([
            "sample",
            "--output",
            sample_path
                .to_str()
                .expect("temporary paths should be valid UTF-8"),
            "--title",
            "Integration sample",
        ])
        .output()
        .expect("sample command should run");

    assert!(sample_output.status.success());

    let diagnostics_output = regxorder_cli_binary()
        .args([
            "diagnostics",
            "--json",
            "recording",
            "--input",
            sample_path
                .to_str()
                .expect("temporary paths should be valid UTF-8"),
        ])
        .output()
        .expect("diagnostics recording command should run");

    assert!(diagnostics_output.status.success());

    let payload: Value = serde_json::from_slice(&diagnostics_output.stdout)
        .expect("diagnostics recording output should be valid JSON");

    assert_eq!(payload["target"], "recording");
    assert_eq!(payload["report"]["doctor"]["recording"]["event_count"], 2);
    assert_eq!(payload["report"]["details"]["first_sequence"], 0);
    assert_eq!(payload["report"]["details"]["last_sequence"], 1);
    assert!(
        payload["report"]["details"]["file_size_bytes"]
            .as_u64()
            .expect("sample file size should be present")
            > 0
    );

    let _ = fs::remove_file(&sample_path);
}

#[test]
fn diagnostics_environment_json_reports_process_elevation_status() {
    let output = regxorder_cli_binary()
        .args(["diagnostics", "--json", "environment"])
        .output()
        .expect("diagnostics environment command should run");

    assert!(output.status.success());

    let payload: Value = serde_json::from_slice(&output.stdout)
        .expect("diagnostics environment output should be valid JSON");
    let checks = payload["report"]["doctor"]["checks"]
        .as_array()
        .expect("environment diagnostics should include checks");

    assert_eq!(payload["target"], "environment");
    assert!(
        checks
            .iter()
            .any(|check| check["name"] == "process_elevation")
    );
}
