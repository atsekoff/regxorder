use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    mem::size_of,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use regxorder_core::{
    AbsoluteScreenPoint, ControlDoctorReport, DiagnosticCheck, DiagnosticStatus, DiagnosticSummary,
    DisplayMetadata, ElapsedTime, EnvironmentDoctorReport, HotkeyBinding, HotkeyKey,
    HotkeyModifier, HotkeyParseError, InputAction, InputEvent, KeyDescriptor, PlaybackDoctorReport,
    Recording, RecordingDoctorReport, RecordingError, RecordingMetadata, RecordingMetrics,
    ScanCode, SchemaVersion, ScreenSize, SpeedMultiplier, ValidationError, diagnose_playback,
    diagnose_recording,
};
use regxorder_win32::{
    ControlAction, ControlBindings, ControlController, HotkeyRegistration, HotkeyWaitOutcome,
    RecordingStrategy, WindowsBackendError, diagnose_windows_environment,
    wait_for_hotkey_activation, wait_for_hotkey_press_and_release,
};
use serde::Serialize;
use serde_json::json;
use thiserror::Error;

const HOTKEY_SMOKE_IDENTIFIER: i32 = 1;
const START_ACTION_HOTKEY_IDENTIFIER: i32 = 1;

const SESSION_DIRECTORY_NAME: &str = "sessions";

#[derive(Debug, Parser)]
#[command(author, version, about = "Command-line tools for regxorder recordings")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate a small sample recording that can be inspected or replayed later.
    Sample {
        /// Path to write the sample recording JSON file. A bare filename is written under sessions/.
        #[arg(long, value_hint = ValueHint::FilePath)]
        output: PathBuf,

        /// Optional human-readable title for the recording.
        #[arg(long)]
        title: Option<String>,
    },

    /// Validate that a recording file parses and satisfies semantic invariants.
    Validate {
        /// Path to the recording JSON file. A bare filename is resolved from sessions/ if present.
        #[arg(long, value_hint = ValueHint::FilePath)]
        input: PathBuf,
    },

    /// Print a compact summary of a recording file.
    Inspect {
        /// Path to the recording JSON file. A bare filename is resolved from sessions/ if present.
        #[arg(long, value_hint = ValueHint::FilePath)]
        input: PathBuf,
    },

    /// Replay a recording file with the Windows playback backend.
    Play {
        /// Path to the recording JSON file. A bare filename is resolved from sessions/ if present.
        #[arg(long, value_hint = ValueHint::FilePath)]
        input: PathBuf,

        /// Playback speed multiplier. Values above 1.0 speed up playback.
        #[arg(long, default_value_t = 1.0)]
        speed: f64,

        /// Optional hotkey that must be pressed before playback starts, for example ctrl+shift+f9.
        #[arg(long, value_name = "HOTKEY")]
        start_hotkey: Option<HotkeyBinding>,

        /// Optional hotkey that stops playback early, for example ctrl+shift+f10.
        #[arg(long, value_name = "HOTKEY")]
        stop_hotkey: Option<HotkeyBinding>,
    },

    /// Record keyboard and mouse input using the selected Windows recording strategy.
    Record {
        /// Path to write the captured recording JSON file. A bare filename is written under sessions/.
        #[arg(long, value_hint = ValueHint::FilePath)]
        output: PathBuf,

        /// Optional title embedded in the captured recording.
        #[arg(long)]
        title: Option<String>,

        /// Optional duration limit in seconds. If omitted, recording stops on Ctrl+C.
        #[arg(long)]
        duration_seconds: Option<f64>,

        /// Recording strategy to use for input capture.
        #[arg(long, value_enum, default_value_t = RecordingStrategyArgument::RawInput)]
        strategy: RecordingStrategyArgument,

        /// Optional hotkey that must be pressed before recording starts, for example ctrl+shift+f9.
        #[arg(long, value_name = "HOTKEY")]
        start_hotkey: Option<HotkeyBinding>,

        /// Optional hotkey that stops recording, for example ctrl+shift+f10.
        #[arg(long, value_name = "HOTKEY")]
        stop_hotkey: Option<HotkeyBinding>,
    },

    /// Run a long-lived control loop that starts recording or playback from global hotkeys.
    Control {
        /// Path to write captured recordings when the record hotkey fires.
        #[arg(long, value_hint = ValueHint::FilePath)]
        record_output: Option<PathBuf>,

        /// Optional title embedded in recordings captured by the control loop.
        #[arg(long)]
        record_title: Option<String>,

        /// Optional duration limit applied to each control-loop recording action.
        #[arg(long)]
        record_duration_seconds: Option<f64>,

        /// Recording strategy to use when the record hotkey fires.
        #[arg(long, value_enum, default_value_t = RecordingStrategyArgument::RawInput)]
        record_strategy: RecordingStrategyArgument,

        /// Hotkey that starts a recording action inside the control loop.
        #[arg(long, value_name = "HOTKEY")]
        start_record_hotkey: Option<HotkeyBinding>,

        /// Path to the recording file to play when the playback hotkey fires.
        #[arg(long, value_hint = ValueHint::FilePath)]
        play_input: Option<PathBuf>,

        /// Playback speed multiplier applied to control-loop playback actions.
        #[arg(long, default_value_t = 1.0)]
        play_speed: f64,

        /// Hotkey that starts a playback action inside the control loop.
        #[arg(long, value_name = "HOTKEY")]
        start_play_hotkey: Option<HotkeyBinding>,

        /// Hotkey that stops the active recording or playback action.
        #[arg(long, value_name = "HOTKEY")]
        stop_hotkey: HotkeyBinding,
    },

    /// Run preflight checks for environment, recordings, playback, or control-loop configuration.
    Doctor {
        /// Emit machine-readable JSON instead of human-readable text.
        #[arg(long, default_value_t = false)]
        json: bool,

        #[command(subcommand)]
        command: DoctorCommand,
    },

    /// Register a global hotkey and print when it triggers.
    WatchHotkey {
        /// Modifier keys for the global hotkey.
        #[arg(long, value_enum, value_delimiter = ',', num_args = 1.., default_values_t = [HotkeyModifierArgument::Control, HotkeyModifierArgument::Shift])]
        modifiers: Vec<HotkeyModifierArgument>,

        /// Base key for the global hotkey.
        #[arg(long, value_enum, default_value_t = HotkeyKeyArgument::F9)]
        key: HotkeyKeyArgument,

        /// Optional duration limit in seconds. If omitted, waits until Ctrl+C or the hotkey fires.
        #[arg(long)]
        timeout_seconds: Option<f64>,

        /// Allow key auto-repeat to retrigger the hotkey while it is held down.
        #[arg(long, default_value_t = false)]
        allow_auto_repeat: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RecordingStrategyArgument {
    RawInput,
    LowLevelHooks,
}

#[derive(Debug, Subcommand)]
enum DoctorCommand {
    /// Check whether the current Windows environment can arm the supported backends.
    Environment,

    /// Check whether a recording file is readable, valid, and well-formed for replay.
    Recording {
        /// Path to the recording JSON file. A bare filename is resolved from sessions/ if present.
        #[arg(long, value_hint = ValueHint::FilePath)]
        input: PathBuf,
    },

    /// Check whether playback preparation succeeds for a recording at the requested speed.
    Playback {
        /// Path to the recording JSON file. A bare filename is resolved from sessions/ if present.
        #[arg(long, value_hint = ValueHint::FilePath)]
        input: PathBuf,

        /// Playback speed multiplier. Values above 1.0 speed up playback.
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
    },

    /// Check whether a control-loop configuration is internally consistent before arming hotkeys.
    Control {
        /// Path to write captured recordings when the record hotkey fires.
        #[arg(long, value_hint = ValueHint::FilePath)]
        record_output: Option<PathBuf>,

        /// Hotkey that starts a recording action inside the control loop.
        #[arg(long, value_name = "HOTKEY")]
        start_record_hotkey: Option<HotkeyBinding>,

        /// Path to the recording file to play when the playback hotkey fires.
        #[arg(long, value_hint = ValueHint::FilePath)]
        play_input: Option<PathBuf>,

        /// Playback speed multiplier applied to control-loop playback actions.
        #[arg(long, default_value_t = 1.0)]
        play_speed: f64,

        /// Hotkey that starts a playback action inside the control loop.
        #[arg(long, value_name = "HOTKEY")]
        start_play_hotkey: Option<HotkeyBinding>,

        /// Hotkey that stops the active recording or playback action.
        #[arg(long, value_name = "HOTKEY")]
        stop_hotkey: HotkeyBinding,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct ControlCommandConfiguration {
    record_output: Option<PathBuf>,
    record_title: Option<String>,
    record_duration_seconds: Option<f64>,
    record_strategy: RecordingStrategyArgument,
    start_record_hotkey: Option<HotkeyBinding>,
    play_input: Option<PathBuf>,
    play_speed: f64,
    start_play_hotkey: Option<HotkeyBinding>,
    stop_hotkey: HotkeyBinding,
}

#[derive(Debug, Clone, PartialEq)]
struct DoctorControlConfiguration {
    record_output: Option<PathBuf>,
    start_record_hotkey: Option<HotkeyBinding>,
    play_input: Option<PathBuf>,
    play_speed: f64,
    start_play_hotkey: Option<HotkeyBinding>,
    stop_hotkey: HotkeyBinding,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct DoctorRecordingOutput {
    input_path: PathBuf,
    summary: DiagnosticSummary,
    checks: Vec<DiagnosticCheck>,
    recording: Option<RecordingDoctorReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct DoctorPlaybackOutput {
    input_path: PathBuf,
    requested_speed: f64,
    summary: DiagnosticSummary,
    checks: Vec<DiagnosticCheck>,
    recording: Option<RecordingDoctorReport>,
    playback: Option<PlaybackDoctorReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct DoctorControlOutput {
    record_output_path: Option<PathBuf>,
    play_input_path: Option<PathBuf>,
    requested_play_speed: f64,
    summary: DiagnosticSummary,
    checks: Vec<DiagnosticCheck>,
    control: ControlDoctorReport,
    playback: Option<PlaybackDoctorReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HotkeyModifierArgument {
    Control,
    Alt,
    Shift,
    Win,
}

impl HotkeyModifierArgument {
    const fn into_hotkey_modifier(self) -> HotkeyModifier {
        match self {
            Self::Control => HotkeyModifier::Control,
            Self::Alt => HotkeyModifier::Alt,
            Self::Shift => HotkeyModifier::Shift,
            Self::Win => HotkeyModifier::Win,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HotkeyKeyArgument {
    Escape,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl HotkeyKeyArgument {
    const fn into_hotkey_key(self) -> HotkeyKey {
        match self {
            Self::Escape => HotkeyKey::Escape,
            Self::F1 => HotkeyKey::F1,
            Self::F2 => HotkeyKey::F2,
            Self::F3 => HotkeyKey::F3,
            Self::F4 => HotkeyKey::F4,
            Self::F5 => HotkeyKey::F5,
            Self::F6 => HotkeyKey::F6,
            Self::F7 => HotkeyKey::F7,
            Self::F8 => HotkeyKey::F8,
            Self::F9 => HotkeyKey::F9,
            Self::F10 => HotkeyKey::F10,
            Self::F11 => HotkeyKey::F11,
            Self::F12 => HotkeyKey::F12,
        }
    }
}

impl RecordingStrategyArgument {
    const fn into_backend_strategy(self) -> RecordingStrategy {
        match self {
            Self::RawInput => RecordingStrategy::RawInput,
            Self::LowLevelHooks => RecordingStrategy::LowLevelHooks,
        }
    }
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("failed to read or write a recording file: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Recording(#[from] RecordingError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    WindowsBackend(#[from] WindowsBackendError),
    #[error(transparent)]
    HotkeyParse(#[from] HotkeyParseError),
    #[error("failed to install Ctrl+C handler: {0}")]
    CtrlC(#[from] ctrlc::Error),
    #[error("duration_seconds must be finite and greater than zero, found {0}")]
    InvalidDuration(f64),
    #[error("control recording requires both --record-output and --start-record-hotkey")]
    InvalidControlRecordingConfiguration,
    #[error("control playback requires both --play-input and --start-play-hotkey")]
    InvalidControlPlaybackConfiguration,
    #[error("doctor reported one or more failing checks")]
    DoctorCheckFailed,
}

pub fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Sample { output, title } => write_sample_recording(&output, title.as_deref()),
        Command::Validate { input } => validate_recording(&input),
        Command::Inspect { input } => inspect_recording(&input),
        Command::Play {
            input,
            speed,
            start_hotkey,
            stop_hotkey,
        } => play_recording_file(&input, speed, start_hotkey, stop_hotkey),
        Command::Record {
            output,
            title,
            duration_seconds,
            strategy,
            start_hotkey,
            stop_hotkey,
        } => record_recording_file(
            &output,
            title,
            duration_seconds,
            strategy,
            start_hotkey,
            stop_hotkey,
        ),
        Command::Control {
            record_output,
            record_title,
            record_duration_seconds,
            record_strategy,
            start_record_hotkey,
            play_input,
            play_speed,
            start_play_hotkey,
            stop_hotkey,
        } => run_control_command(ControlCommandConfiguration {
            record_output,
            record_title,
            record_duration_seconds,
            record_strategy,
            start_record_hotkey,
            play_input,
            play_speed,
            start_play_hotkey,
            stop_hotkey,
        }),
        Command::Doctor { json, command } => run_doctor_command(command, json),
        Command::WatchHotkey {
            modifiers,
            key,
            timeout_seconds,
            allow_auto_repeat,
        } => watch_hotkey(&modifiers, key, timeout_seconds, allow_auto_repeat),
    }
}

fn watch_hotkey(
    modifiers: &[HotkeyModifierArgument],
    key: HotkeyKeyArgument,
    timeout_seconds: Option<f64>,
    allow_auto_repeat: bool,
) -> Result<(), CliError> {
    let timeout_seconds = validate_optional_duration(timeout_seconds)?;
    let stop_requested = Arc::new(AtomicBool::new(false));
    install_shutdown_handler(&stop_requested, None)?;

    arm_optional_duration_stop(timeout_seconds, &stop_requested);

    let hotkey_binding = hotkey_binding_from_arguments(modifiers, key)?;
    let hotkey_registration =
        HotkeyRegistration::from_binding(HOTKEY_SMOKE_IDENTIFIER, &hotkey_binding);
    let hotkey_registration = if allow_auto_repeat {
        hotkey_registration.with_auto_repeat_enabled()
    } else {
        hotkey_registration
    };
    let hotkey_label = hotkey_binding.to_string();

    if let Some(seconds) = timeout_seconds {
        println!(
            "watching for global hotkey {} for up to {:.3} seconds; press Ctrl+C to stop",
            hotkey_label, seconds
        );
    } else {
        println!(
            "watching for global hotkey {}; press Ctrl+C to stop",
            hotkey_label
        );
    }

    match wait_for_hotkey_activation(&[hotkey_registration], stop_requested.as_ref())? {
        Some(activation) => {
            println!(
                "hotkey triggered: {} (id={})",
                hotkey_label,
                activation.identifier()
            );
        }
        None => {
            println!("hotkey watch stopped before any registered hotkey fired");
        }
    }

    Ok(())
}

fn record_recording_file(
    output: &Path,
    title: Option<String>,
    duration_seconds: Option<f64>,
    strategy: RecordingStrategyArgument,
    start_hotkey: Option<HotkeyBinding>,
    stop_hotkey: Option<HotkeyBinding>,
) -> Result<(), CliError> {
    let duration_seconds = validate_optional_duration(duration_seconds)?;
    let resolved_output_path = resolve_session_output_path(output);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let recording_strategy = strategy.into_backend_strategy();
    let control_controller = ControlController;

    install_shutdown_handler(&stop_requested, None)?;

    if !wait_for_optional_start_hotkey("recording", start_hotkey.as_ref(), &stop_requested)? {
        return Ok(());
    }

    arm_optional_duration_stop(duration_seconds, &stop_requested);
    let stop_controls = format_stop_controls(stop_hotkey.as_ref());

    if let Some(seconds) = duration_seconds {
        println!(
            "recording with {} for up to {:.3} seconds; press {} to stop early",
            recording_strategy, seconds, stop_controls
        );
    } else {
        println!(
            "recording with {}; press {} to stop",
            recording_strategy, stop_controls
        );
    }

    let recording_outcome = control_controller.run_recording_action(
        recording_strategy,
        title,
        stop_hotkey.as_ref(),
        &stop_requested,
    )?;
    let recording = recording_outcome.recording;

    write_recording(&resolved_output_path, &recording)?;

    println!(
        "recorded {} events over {} us to {}",
        recording.event_count(),
        recording.duration().as_micros(),
        resolved_output_path.display()
    );
    print_recording_finalization_summary(recording_outcome.finalization_report);
    print_recording_metrics(&recording, &resolved_output_path)?;

    Ok(())
}

fn play_recording_file(
    path: &Path,
    speed: f64,
    start_hotkey: Option<HotkeyBinding>,
    stop_hotkey: Option<HotkeyBinding>,
) -> Result<(), CliError> {
    let resolved_input_path = resolve_session_input_path(path);
    let recording = load_recording(&resolved_input_path)?;
    let speed = SpeedMultiplier::new(speed)?;
    let stop_requested = Arc::new(AtomicBool::new(false));
    let control_controller = ControlController;

    install_shutdown_handler(&stop_requested, None)?;

    if !wait_for_optional_start_hotkey("playback", start_hotkey.as_ref(), &stop_requested)? {
        return Ok(());
    }

    let stop_controls = format_stop_controls(stop_hotkey.as_ref());

    println!(
        "playing {} events from {} at {}x speed; press {} to stop",
        recording.event_count(),
        resolved_input_path.display(),
        speed.get(),
        stop_controls
    );

    let playback_outcome = control_controller.run_playback_action(
        &recording,
        speed,
        stop_hotkey.as_ref(),
        &stop_requested,
    )?;
    let report = playback_outcome.playback_report;

    print_playback_preparation_summary(
        playback_outcome.preparation_report,
        playback_outcome.prepared_event_count,
    );

    if report.interrupted {
        println!(
            "playback interrupted after {} events and {:.3} ms",
            report.dispatched_events,
            report.elapsed.as_secs_f64() * 1_000.0
        );
    } else {
        println!(
            "playback completed: {} events in {:.3} ms",
            report.dispatched_events,
            report.elapsed.as_secs_f64() * 1_000.0
        );
    }

    Ok(())
}

fn run_control_command(configuration: ControlCommandConfiguration) -> Result<(), CliError> {
    let ControlCommandConfiguration {
        record_output,
        record_title,
        record_duration_seconds,
        record_strategy,
        start_record_hotkey,
        play_input,
        play_speed,
        start_play_hotkey,
        stop_hotkey,
    } = configuration;

    let record_duration_seconds = validate_optional_duration(record_duration_seconds)?;
    let recording_action = match (record_output, start_record_hotkey) {
        (Some(output), Some(hotkey)) => Some((resolve_session_output_path(&output), hotkey)),
        (None, None) => None,
        _ => return Err(CliError::InvalidControlRecordingConfiguration),
    };
    let playback_action = match (play_input, start_play_hotkey) {
        (Some(input), Some(hotkey)) => Some((
            resolve_session_input_path(&input),
            hotkey,
            SpeedMultiplier::new(play_speed)?,
        )),
        (None, None) => None,
        _ => return Err(CliError::InvalidControlPlaybackConfiguration),
    };
    let control_bindings = ControlBindings::new(
        recording_action.as_ref().map(|(_, hotkey)| hotkey.clone()),
        playback_action
            .as_ref()
            .map(|(_, hotkey, _)| hotkey.clone()),
    )?;
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let current_action_stop_target = Arc::new(Mutex::new(None));
    let control_controller = ControlController;

    install_shutdown_handler(&shutdown_requested, Some(&current_action_stop_target))?;

    if let Some((output, hotkey)) = &recording_action {
        println!(
            "control recording armed: {} -> {}",
            hotkey,
            output.display()
        );
    }

    if let Some((input, hotkey, speed)) = &playback_action {
        println!(
            "control playback armed: {} -> {} at {}x",
            hotkey,
            input.display(),
            speed.get()
        );
    }

    println!("control stop hotkey armed: {}", stop_hotkey);
    println!("press Ctrl+C to exit the control loop");

    while let Some(control_action) =
        control_controller.wait_for_next_action(&control_bindings, shutdown_requested.as_ref())?
    {
        match control_action {
            ControlAction::Record => {
                let Some((output, _)) = &recording_action else {
                    continue;
                };

                if let Err(error) = run_control_recording_action(
                    control_controller,
                    output,
                    record_title.clone(),
                    record_duration_seconds,
                    record_strategy.into_backend_strategy(),
                    &stop_hotkey,
                    &current_action_stop_target,
                ) {
                    eprintln!("control recording action failed: {error}");
                }
            }
            ControlAction::Play => {
                let Some((input, _, speed)) = &playback_action else {
                    continue;
                };

                if let Err(error) = run_control_playback_action(
                    control_controller,
                    input,
                    *speed,
                    &stop_hotkey,
                    &current_action_stop_target,
                ) {
                    eprintln!("control playback action failed: {error}");
                }
            }
        }

        if shutdown_requested.load(Ordering::SeqCst) {
            break;
        }
    }

    println!("control loop stopped");
    Ok(())
}

fn run_doctor_command(command: DoctorCommand, emit_json: bool) -> Result<(), CliError> {
    match command {
        DoctorCommand::Environment => {
            let report = build_environment_doctor_report()?;
            if emit_json {
                print_doctor_json("environment", &report)?;
            } else {
                print_environment_doctor_report(&report);
            }

            finish_doctor_command(report.summary)
        }
        DoctorCommand::Recording { input } => {
            let report = build_recording_doctor_output(&input);
            if emit_json {
                print_doctor_json("recording", &report)?;
            } else {
                print_recording_doctor_output(&report);
            }

            finish_doctor_command(report.summary)
        }
        DoctorCommand::Playback { input, speed } => {
            let report = build_playback_doctor_output(&input, speed);
            if emit_json {
                print_doctor_json("playback", &report)?;
            } else {
                print_playback_doctor_output(&report);
            }

            finish_doctor_command(report.summary)
        }
        DoctorCommand::Control {
            record_output,
            start_record_hotkey,
            play_input,
            play_speed,
            start_play_hotkey,
            stop_hotkey,
        } => {
            let report = build_control_doctor_output(DoctorControlConfiguration {
                record_output,
                start_record_hotkey,
                play_input,
                play_speed,
                start_play_hotkey,
                stop_hotkey,
            });
            if emit_json {
                print_doctor_json("control", &report)?;
            } else {
                print_control_doctor_output(&report);
            }

            finish_doctor_command(report.summary)
        }
    }
}

fn build_environment_doctor_report() -> Result<EnvironmentDoctorReport, CliError> {
    let hotkey_probe = HotkeyBinding::new(
        vec![
            HotkeyModifier::Control,
            HotkeyModifier::Alt,
            HotkeyModifier::Shift,
        ],
        HotkeyKey::F12,
    )?;
    let backend_report = diagnose_windows_environment(hotkey_probe);

    Ok(augment_environment_doctor_report_with_session_directory(
        backend_report,
        &default_session_directory(),
    ))
}

fn augment_environment_doctor_report_with_session_directory(
    environment_report: EnvironmentDoctorReport,
    session_directory: &Path,
) -> EnvironmentDoctorReport {
    let EnvironmentDoctorReport {
        target_os,
        target_arch,
        recording_backends,
        playback_backend,
        hotkey_backend,
        hotkey_probe,
        checks,
        ..
    } = environment_report;
    let mut checks = checks;
    checks.push(check_session_directory(session_directory));

    EnvironmentDoctorReport::new(
        target_os,
        target_arch,
        recording_backends,
        playback_backend,
        hotkey_backend,
        hotkey_probe,
        checks,
    )
}

fn build_recording_doctor_output(path: &Path) -> DoctorRecordingOutput {
    let input_path = resolve_session_input_path(path);
    let (mut checks, recording) = load_recording_for_doctor(&input_path);
    let recording_report = recording.as_ref().map(diagnose_recording);

    if let Some(report) = &recording_report {
        checks.extend(report.checks.iter().cloned());
    }

    DoctorRecordingOutput {
        input_path,
        summary: DiagnosticSummary::from_checks(&checks),
        checks,
        recording: recording_report,
    }
}

fn build_playback_doctor_output(path: &Path, speed: f64) -> DoctorPlaybackOutput {
    let input_path = resolve_session_input_path(path);
    let mut checks = Vec::new();
    let speed_multiplier = match SpeedMultiplier::new(speed) {
        Ok(speed_multiplier) => Some(speed_multiplier),
        Err(error) => {
            checks.push(DiagnosticCheck::fail(
                "speed_multiplier",
                format!("requested playback speed is invalid: {error}"),
            ));
            None
        }
    };
    let (recording_checks, recording) = load_recording_for_doctor(&input_path);
    checks.extend(recording_checks);

    let recording_report = recording.as_ref().map(diagnose_recording);
    if let Some(report) = &recording_report {
        checks.extend(report.checks.iter().cloned());
    }

    let playback_report = match (recording.as_ref(), speed_multiplier) {
        (Some(recording), Some(speed_multiplier)) => {
            match diagnose_playback(recording, speed_multiplier) {
                Ok(report) => {
                    checks.extend(report.checks.iter().cloned());
                    Some(report)
                }
                Err(error) => {
                    checks.push(DiagnosticCheck::fail(
                        "prepared_playback_plan",
                        format!("playback preparation failed: {error}"),
                    ));
                    None
                }
            }
        }
        _ => None,
    };

    DoctorPlaybackOutput {
        input_path,
        requested_speed: speed,
        summary: DiagnosticSummary::from_checks(&checks),
        checks,
        recording: recording_report,
        playback: playback_report,
    }
}

fn build_control_doctor_output(configuration: DoctorControlConfiguration) -> DoctorControlOutput {
    let DoctorControlConfiguration {
        record_output,
        start_record_hotkey,
        play_input,
        play_speed,
        start_play_hotkey,
        stop_hotkey,
    } = configuration;
    let record_output_path = record_output
        .as_ref()
        .map(|path| resolve_session_output_path(path));
    let play_input_path = play_input
        .as_ref()
        .map(|path| resolve_session_input_path(path));

    let control = build_control_doctor_report(
        record_output_path.as_ref(),
        start_record_hotkey.clone(),
        play_input_path.as_ref(),
        start_play_hotkey.clone(),
        stop_hotkey,
        play_speed,
    );
    let mut checks = control.checks.clone();

    if let Some(record_output_path) = &record_output_path {
        checks.push(check_output_path_parent(record_output_path));
    }

    let speed_multiplier = match SpeedMultiplier::new(play_speed) {
        Ok(speed_multiplier) => Some(speed_multiplier),
        Err(error) => {
            checks.push(DiagnosticCheck::fail(
                "playback_speed",
                format!("requested playback speed is invalid: {error}"),
            ));
            None
        }
    };

    let playback = if let Some(play_input_path) = &play_input_path {
        let (playback_checks, recording) = load_recording_for_doctor(play_input_path);
        checks.extend(playback_checks);

        match (recording.as_ref(), speed_multiplier) {
            (Some(recording), Some(speed_multiplier)) => {
                match diagnose_playback(recording, speed_multiplier) {
                    Ok(report) => {
                        checks.extend(report.checks.iter().cloned());
                        Some(report)
                    }
                    Err(error) => {
                        checks.push(DiagnosticCheck::fail(
                            "control_playback_plan",
                            format!("control playback preparation failed: {error}"),
                        ));
                        None
                    }
                }
            }
            _ => None,
        }
    } else {
        None
    };

    DoctorControlOutput {
        record_output_path,
        play_input_path,
        requested_play_speed: play_speed,
        summary: DiagnosticSummary::from_checks(&checks),
        checks,
        control,
        playback,
    }
}

fn build_control_doctor_report(
    record_output_path: Option<&PathBuf>,
    start_record_hotkey: Option<HotkeyBinding>,
    play_input_path: Option<&PathBuf>,
    start_play_hotkey: Option<HotkeyBinding>,
    stop_hotkey: HotkeyBinding,
    play_speed: f64,
) -> ControlDoctorReport {
    let mut checks = Vec::with_capacity(4);

    checks.push(DiagnosticCheck::pass(
        "stop_hotkey",
        format!("control stop hotkey is {}", stop_hotkey),
    ));

    checks.push(match (record_output_path, start_record_hotkey.as_ref()) {
        (Some(record_output_path), Some(start_record_hotkey)) => DiagnosticCheck::pass(
            "recording_action",
            format!(
                "recording action is armed on {} -> {}",
                start_record_hotkey,
                record_output_path.display()
            ),
        ),
        (None, None) => DiagnosticCheck::warn(
            "recording_action",
            "recording action is not configured for this control loop",
        ),
        _ => DiagnosticCheck::fail(
            "recording_action",
            "control recording requires both --record-output and --start-record-hotkey",
        ),
    });

    checks.push(match (play_input_path, start_play_hotkey.as_ref()) {
        (Some(play_input_path), Some(start_play_hotkey)) => DiagnosticCheck::pass(
            "playback_action",
            format!(
                "playback action is armed on {} -> {} at {}x",
                start_play_hotkey,
                play_input_path.display(),
                play_speed,
            ),
        ),
        (None, None) => DiagnosticCheck::warn(
            "playback_action",
            "playback action is not configured for this control loop",
        ),
        _ => DiagnosticCheck::fail(
            "playback_action",
            "control playback requires both --play-input and --start-play-hotkey",
        ),
    });

    checks.push(
        match ControlBindings::new(start_record_hotkey.clone(), start_play_hotkey.clone()) {
            Ok(_) => DiagnosticCheck::pass(
                "control_bindings",
                "idle control-loop start hotkeys are internally consistent",
            ),
            Err(error) => DiagnosticCheck::fail(
                "control_bindings",
                format!("control-loop binding validation failed: {error}"),
            ),
        },
    );

    ControlDoctorReport::new(start_record_hotkey, start_play_hotkey, stop_hotkey, checks)
}

fn load_recording_for_doctor(path: &Path) -> (Vec<DiagnosticCheck>, Option<Recording>) {
    let mut checks = Vec::with_capacity(2);

    match fs::read_to_string(path) {
        Ok(input) => {
            let input_file_summary = match fs::metadata(path) {
                Ok(metadata) => format!(
                    "recording file is readable at {} ({} bytes)",
                    path.display(),
                    metadata.len()
                ),
                Err(_) => format!("recording file is readable at {}", path.display()),
            };
            checks.push(DiagnosticCheck::pass("input_file", input_file_summary));

            match Recording::from_json_str(&input) {
                Ok(recording) => {
                    checks.push(DiagnosticCheck::pass(
                        "recording_parse",
                        "recording payload parsed and validated successfully",
                    ));
                    (checks, Some(recording))
                }
                Err(error) => {
                    checks.push(DiagnosticCheck::fail(
                        "recording_parse",
                        format!("recording payload is invalid: {error}"),
                    ));
                    (checks, None)
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            checks.push(DiagnosticCheck::fail(
                "input_file",
                format!("recording file was not found at {}", path.display()),
            ));
            (checks, None)
        }
        Err(error) => {
            checks.push(DiagnosticCheck::fail(
                "input_file",
                format!("failed to read {}: {error}", path.display()),
            ));
            (checks, None)
        }
    }
}

fn check_session_directory(path: &Path) -> DiagnosticCheck {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => DiagnosticCheck::pass(
            "session_directory",
            format!("session directory is available at {}", path.display()),
        ),
        Ok(_) => DiagnosticCheck::fail(
            "session_directory",
            format!(
                "session path exists but is not a directory: {}",
                path.display()
            ),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DiagnosticCheck::warn(
            "session_directory",
            format!(
                "session directory {} does not exist yet; it will be created on first write",
                path.display()
            ),
        ),
        Err(error) => DiagnosticCheck::fail(
            "session_directory",
            format!(
                "failed to inspect session directory {}: {error}",
                path.display()
            ),
        ),
    }
}

fn check_output_path_parent(path: &Path) -> DiagnosticCheck {
    let Some(parent) = path.parent() else {
        return DiagnosticCheck::pass(
            "record_output_parent",
            format!(
                "recording output path {} has no parent directory to verify",
                path.display()
            ),
        );
    };

    if parent.as_os_str().is_empty() {
        return DiagnosticCheck::pass(
            "record_output_parent",
            format!(
                "recording output path {} uses the current directory",
                path.display()
            ),
        );
    }

    match fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => DiagnosticCheck::pass(
            "record_output_parent",
            format!(
                "recording output directory is available at {}",
                parent.display()
            ),
        ),
        Ok(_) => DiagnosticCheck::fail(
            "record_output_parent",
            format!(
                "recording output parent exists but is not a directory: {}",
                parent.display()
            ),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DiagnosticCheck::warn(
            "record_output_parent",
            format!(
                "recording output directory {} does not exist yet; it will be created on first write",
                parent.display()
            ),
        ),
        Err(error) => DiagnosticCheck::fail(
            "record_output_parent",
            format!(
                "failed to inspect recording output directory {}: {error}",
                parent.display()
            ),
        ),
    }
}

fn finish_doctor_command(summary: DiagnosticSummary) -> Result<(), CliError> {
    if summary.has_failures() {
        Err(CliError::DoctorCheckFailed)
    } else {
        Ok(())
    }
}

fn print_doctor_json<T>(target: &str, report: &T) -> Result<(), CliError>
where
    T: Serialize,
{
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "target": target,
            "report": report,
        }))?
    );

    Ok(())
}

fn print_environment_doctor_report(report: &EnvironmentDoctorReport) {
    println!("doctor target: environment");
    println!("target_os: {}", report.target_os);
    println!("target_arch: {}", report.target_arch);
    println!(
        "recording_backends: {}",
        report.recording_backends.join(", ")
    );
    println!("playback_backend: {}", report.playback_backend);
    println!("hotkey_backend: {}", report.hotkey_backend);
    println!("hotkey_probe: {}", report.hotkey_probe);
    print_diagnostic_summary(report.summary);
    print_diagnostic_checks(&report.checks);
}

fn print_recording_doctor_output(report: &DoctorRecordingOutput) {
    println!("doctor target: recording");
    println!("input_path: {}", report.input_path.display());

    if let Some(recording) = &report.recording {
        println!(
            "title: {}",
            recording.title.as_deref().unwrap_or("<untitled>")
        );
        println!("schema: {}", recording.schema_version);
        println!("events: {}", recording.event_count);
        println!("duration_us: {}", recording.duration_micros);
        println!(
            "average_events_per_second: {:.2}",
            recording.average_events_per_second
        );
    }

    print_diagnostic_summary(report.summary);
    print_diagnostic_checks(&report.checks);
}

fn print_playback_doctor_output(report: &DoctorPlaybackOutput) {
    println!("doctor target: playback");
    println!("input_path: {}", report.input_path.display());
    println!("requested_speed: {}", report.requested_speed);

    if let Some(playback) = &report.playback {
        println!("canonical_events: {}", playback.canonical_event_count);
        println!("prepared_events: {}", playback.prepared_event_count);
        println!("prepared_speed: {}", playback.speed_multiplier.get());
    }

    print_diagnostic_summary(report.summary);
    print_diagnostic_checks(&report.checks);
}

fn print_control_doctor_output(report: &DoctorControlOutput) {
    println!("doctor target: control");

    if let Some(record_output_path) = &report.record_output_path {
        println!("record_output_path: {}", record_output_path.display());
    }

    if let Some(play_input_path) = &report.play_input_path {
        println!("play_input_path: {}", play_input_path.display());
    }

    println!("requested_play_speed: {}", report.requested_play_speed);
    println!("stop_hotkey: {}", report.control.stop_hotkey);

    if let Some(playback) = &report.playback {
        println!("control_prepared_events: {}", playback.prepared_event_count);
    }

    print_diagnostic_summary(report.summary);
    print_diagnostic_checks(&report.checks);
}

fn print_diagnostic_summary(summary: DiagnosticSummary) {
    println!(
        "overall_status: {}",
        format_diagnostic_status(summary.overall_status())
    );
    println!(
        "check_counts: pass={} warn={} fail={}",
        summary.pass_count, summary.warn_count, summary.fail_count
    );
}

fn print_diagnostic_checks(checks: &[DiagnosticCheck]) {
    for check in checks {
        println!(
            "[{}] {}: {}",
            format_diagnostic_status(check.status),
            check.name,
            check.summary
        );
    }
}

fn format_diagnostic_status(status: DiagnosticStatus) -> &'static str {
    match status {
        DiagnosticStatus::Pass => "pass",
        DiagnosticStatus::Warn => "warn",
        DiagnosticStatus::Fail => "fail",
    }
}

fn write_sample_recording(path: &Path, title: Option<&str>) -> Result<(), CliError> {
    let resolved_output_path = resolve_session_output_path(path);
    let recording = sample_recording(title);
    write_recording(&resolved_output_path, &recording)?;

    println!(
        "wrote sample recording to {} ({} events, {} us)",
        resolved_output_path.display(),
        recording.event_count(),
        recording.duration().as_micros()
    );

    Ok(())
}

fn validate_recording(path: &Path) -> Result<(), CliError> {
    let recording = load_recording(&resolve_session_input_path(path))?;

    println!(
        "recording is valid: {} events over {} us",
        recording.event_count(),
        recording.duration().as_micros()
    );

    Ok(())
}

fn inspect_recording(path: &Path) -> Result<(), CliError> {
    let recording = load_recording(&resolve_session_input_path(path))?;
    let metadata = recording.metadata();
    let title = metadata.title.as_deref().unwrap_or("<untitled>");
    let duration_micros = recording.duration().as_micros();
    let duration_millis = duration_micros as f64 / 1_000.0;

    println!("title: {title}");
    println!("schema: {}", metadata.schema_version.get());
    println!("events: {}", recording.event_count());
    println!("duration_us: {duration_micros}");
    println!("duration_ms: {duration_millis:.3}");
    println!(
        "virtual_display: origin=({}, {}) size={}x{}",
        metadata.display.virtual_origin.x,
        metadata.display.virtual_origin.y,
        metadata.display.virtual_size.width,
        metadata.display.virtual_size.height
    );
    println!("monitors: {}", metadata.display.monitors.len());

    if let Some(first_event) = recording.events().first() {
        println!("first_sequence: {}", first_event.sequence);
    }

    if let Some(last_event) = recording.events().last() {
        println!("last_sequence: {}", last_event.sequence);
    }

    print_recording_metrics(&recording, &resolve_session_input_path(path))?;

    Ok(())
}

fn load_recording(path: &Path) -> Result<Recording, CliError> {
    let input = fs::read_to_string(path)?;
    Ok(Recording::from_json_str(&input)?)
}

fn write_recording(path: &Path, recording: &Recording) -> Result<(), CliError> {
    ensure_parent_directory_exists(path)?;

    let output_file = File::create(path)?;
    let mut writer = BufWriter::new(output_file);
    recording.write_json_pretty(&mut writer)?;
    writer.flush()?;

    Ok(())
}

fn print_recording_metrics(recording: &Recording, path: &Path) -> Result<(), CliError> {
    let metrics = recording.metrics();
    let file_size_bytes = fs::metadata(path)?.len();
    let input_event_size_bytes = size_of::<InputEvent>() as u64;
    let estimated_event_buffer_bytes = input_event_size_bytes * metrics.total_events as u64;
    let average_events_per_second = average_events_per_second(&metrics);

    println!("event_size_bytes: {input_event_size_bytes}");
    println!(
        "estimated_event_buffer_bytes: {} ({})",
        estimated_event_buffer_bytes,
        format_binary_size(estimated_event_buffer_bytes)
    );
    println!(
        "file_size_bytes: {file_size_bytes} ({})",
        format_binary_size(file_size_bytes)
    );
    println!("average_events_per_second: {average_events_per_second:.2}");
    println!("peak_events_per_second: {}", metrics.peak_events_per_second);
    println!(
        "key_pressed_events: {}",
        metrics.action_counts.key_pressed_events
    );
    println!(
        "key_released_events: {}",
        metrics.action_counts.key_released_events
    );
    println!(
        "pointer_moved_events: {}",
        metrics.action_counts.pointer_moved_events
    );
    println!(
        "mouse_button_pressed_events: {}",
        metrics.action_counts.mouse_button_pressed_events
    );
    println!(
        "mouse_button_released_events: {}",
        metrics.action_counts.mouse_button_released_events
    );
    println!(
        "mouse_wheel_scrolled_events: {}",
        metrics.action_counts.mouse_wheel_scrolled_events
    );

    Ok(())
}

fn average_events_per_second(metrics: &RecordingMetrics) -> f64 {
    if metrics.total_events == 0 {
        return 0.0;
    }

    let duration_micros = metrics.duration_micros.max(1);
    metrics.total_events as f64 * 1_000_000.0 / duration_micros as f64
}

fn validate_optional_duration(duration_seconds: Option<f64>) -> Result<Option<f64>, CliError> {
    match duration_seconds {
        Some(seconds) if !seconds.is_finite() || seconds <= 0.0 => {
            Err(CliError::InvalidDuration(seconds))
        }
        _ => Ok(duration_seconds),
    }
}

fn arm_optional_duration_stop(duration_seconds: Option<f64>, stop_requested: &Arc<AtomicBool>) {
    if let Some(seconds) = duration_seconds {
        let timed_stop = Arc::clone(stop_requested);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
            timed_stop.store(true, Ordering::SeqCst);
        });
    }
}

fn install_shutdown_handler(
    stop_requested: &Arc<AtomicBool>,
    current_action_stop_target: Option<&Arc<Mutex<Option<Weak<AtomicBool>>>>>,
) -> Result<(), CliError> {
    let stop_requested = Arc::clone(stop_requested);
    let current_action_stop_target = current_action_stop_target.cloned();

    ctrlc::set_handler(move || {
        stop_requested.store(true, Ordering::SeqCst);

        let Some(current_action_stop_target) = &current_action_stop_target else {
            return;
        };

        let Ok(current_action_stop_target) = current_action_stop_target.lock() else {
            return;
        };

        let Some(current_action_stop_requested) =
            current_action_stop_target.as_ref().and_then(Weak::upgrade)
        else {
            return;
        };

        current_action_stop_requested.store(true, Ordering::SeqCst);
    })?;

    Ok(())
}

fn set_current_action_stop_target(
    current_action_stop_target: &Arc<Mutex<Option<Weak<AtomicBool>>>>,
    stop_requested: Option<&Arc<AtomicBool>>,
) {
    if let Ok(mut current_action_stop_target) = current_action_stop_target.lock() {
        *current_action_stop_target = stop_requested.map(Arc::downgrade);
    }
}

fn run_control_recording_action(
    control_controller: ControlController,
    output: &Path,
    title: Option<String>,
    duration_seconds: Option<f64>,
    strategy: RecordingStrategy,
    stop_hotkey: &HotkeyBinding,
    current_action_stop_target: &Arc<Mutex<Option<Weak<AtomicBool>>>>,
) -> Result<(), CliError> {
    let stop_requested = Arc::new(AtomicBool::new(false));

    set_current_action_stop_target(current_action_stop_target, Some(&stop_requested));
    arm_optional_duration_stop(duration_seconds, &stop_requested);

    println!("control recording started; press {} to stop", stop_hotkey);

    let recording_result = control_controller.run_recording_action(
        strategy,
        title,
        Some(stop_hotkey),
        &stop_requested,
    );

    set_current_action_stop_target(current_action_stop_target, None);

    let recording_outcome = recording_result?;
    write_recording(output, &recording_outcome.recording)?;

    println!(
        "control recording saved {} events over {} us to {}",
        recording_outcome.recording.event_count(),
        recording_outcome.recording.duration().as_micros(),
        output.display()
    );
    print_recording_finalization_summary(recording_outcome.finalization_report);
    print_recording_metrics(&recording_outcome.recording, output)?;

    Ok(())
}

fn run_control_playback_action(
    control_controller: ControlController,
    input: &Path,
    speed: SpeedMultiplier,
    stop_hotkey: &HotkeyBinding,
    current_action_stop_target: &Arc<Mutex<Option<Weak<AtomicBool>>>>,
) -> Result<(), CliError> {
    let recording = load_recording(input)?;
    let stop_requested = Arc::new(AtomicBool::new(false));

    set_current_action_stop_target(current_action_stop_target, Some(&stop_requested));

    println!(
        "control playback started from {} at {}x speed; press {} to stop",
        input.display(),
        speed.get(),
        stop_hotkey
    );

    let playback_result = control_controller.run_playback_action(
        &recording,
        speed,
        Some(stop_hotkey),
        &stop_requested,
    );

    set_current_action_stop_target(current_action_stop_target, None);

    let playback_outcome = playback_result?;
    print_playback_preparation_summary(
        playback_outcome.preparation_report,
        playback_outcome.prepared_event_count,
    );

    if playback_outcome.playback_report.interrupted {
        println!(
            "control playback interrupted after {} events and {:.3} ms",
            playback_outcome.playback_report.dispatched_events,
            playback_outcome.playback_report.elapsed.as_secs_f64() * 1_000.0
        );
    } else {
        println!(
            "control playback completed: {} events in {:.3} ms",
            playback_outcome.playback_report.dispatched_events,
            playback_outcome.playback_report.elapsed.as_secs_f64() * 1_000.0
        );
    }

    Ok(())
}

fn wait_for_optional_start_hotkey(
    action_name: &str,
    start_hotkey: Option<&HotkeyBinding>,
    stop_requested: &Arc<AtomicBool>,
) -> Result<bool, CliError> {
    let Some(start_hotkey) = start_hotkey else {
        return Ok(true);
    };

    println!(
        "waiting for {} start hotkey {}; press Ctrl+C to cancel",
        action_name, start_hotkey
    );

    match wait_for_hotkey_press_and_release(
        start_hotkey,
        START_ACTION_HOTKEY_IDENTIFIER,
        stop_requested.as_ref(),
    )? {
        HotkeyWaitOutcome::Activated => {
            println!("{} start hotkey triggered: {}", action_name, start_hotkey);
            Ok(true)
        }
        HotkeyWaitOutcome::Cancelled => {
            println!(
                "{} start was cancelled before the hotkey fired",
                action_name
            );
            Ok(false)
        }
    }
}

fn hotkey_binding_from_arguments(
    modifiers: &[HotkeyModifierArgument],
    key: HotkeyKeyArgument,
) -> Result<HotkeyBinding, CliError> {
    HotkeyBinding::new(
        modifiers
            .iter()
            .copied()
            .map(HotkeyModifierArgument::into_hotkey_modifier)
            .collect(),
        key.into_hotkey_key(),
    )
    .map_err(CliError::from)
}

fn format_stop_controls(stop_hotkey: Option<&HotkeyBinding>) -> String {
    match stop_hotkey {
        Some(stop_hotkey) => format!("{} or Ctrl+C", stop_hotkey),
        None => String::from("Ctrl+C"),
    }
}

fn print_recording_finalization_summary(
    finalization_report: regxorder_core::RecordingFinalizationReport,
) {
    if finalization_report.appended_release_events > 0 {
        println!(
            "recording_finalization_appended_release_events: {}",
            finalization_report.appended_release_events
        );
    }
}

fn print_playback_preparation_summary(
    preparation_report: regxorder_core::PlaybackPreparationReport,
    prepared_event_count: usize,
) {
    println!("prepared_playback_events: {}", prepared_event_count);

    if preparation_report.skipped_unmatched_release_events > 0
        || preparation_report.appended_release_events > 0
    {
        println!(
            "prepared_playback_cleanup: skipped_unmatched_release_events={} appended_release_events={}",
            preparation_report.skipped_unmatched_release_events,
            preparation_report.appended_release_events
        );
    }
}

fn format_binary_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

    let mut value = bytes as f64;
    let mut unit_index = 0_usize;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    format!("{value:.2} {}", UNITS[unit_index])
}

fn sample_recording(title: Option<&str>) -> Recording {
    Recording::new(
        RecordingMetadata {
            schema_version: SchemaVersion::new(1),
            title: title.map(ToOwned::to_owned),
            display: DisplayMetadata {
                virtual_origin: AbsoluteScreenPoint { x: 0, y: 0 },
                virtual_size: ScreenSize {
                    width: 1920,
                    height: 1080,
                },
                monitors: Vec::new(),
            },
        },
        vec![
            InputEvent {
                sequence: 0,
                elapsed_time: ElapsedTime::from_micros(0),
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
                elapsed_time: ElapsedTime::from_micros(40_000),
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
    .expect("built-in sample recording should always be valid")
}

fn resolve_session_output_path(requested_path: &Path) -> PathBuf {
    if requested_path.is_absolute() || requested_path.components().count() > 1 {
        requested_path.to_path_buf()
    } else {
        default_session_directory().join(requested_path)
    }
}

fn resolve_session_input_path(requested_path: &Path) -> PathBuf {
    if requested_path.exists()
        || requested_path.is_absolute()
        || requested_path.components().count() > 1
    {
        requested_path.to_path_buf()
    } else {
        default_session_directory().join(requested_path)
    }
}

fn default_session_directory() -> PathBuf {
    PathBuf::from(SESSION_DIRECTORY_NAME)
}

fn ensure_parent_directory_exists(path: &Path) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use clap::Parser;

    use super::{
        Cli, CliError, Command, ControlCommandConfiguration, DoctorCommand,
        DoctorControlConfiguration, HotkeyKeyArgument, HotkeyModifierArgument,
        RecordingStrategyArgument, augment_environment_doctor_report_with_session_directory,
        average_events_per_second, build_control_doctor_output, build_playback_doctor_output,
        build_recording_doctor_output, format_binary_size, format_stop_controls,
        hotkey_binding_from_arguments, resolve_session_input_path, resolve_session_output_path,
        run_control_command, sample_recording, write_recording,
    };
    use regxorder_core::{
        AbsoluteScreenPoint, DiagnosticCheck, DiagnosticStatus, DisplayMetadata, ElapsedTime,
        HotkeyBinding, InputAction, InputEvent, KeyDescriptor, Recording, RecordingActionCounts,
        RecordingMetadata, RecordingMetrics, ScanCode, SchemaVersion, ScreenSize,
    };
    use regxorder_win32::WindowsBackendError;

    #[test]
    fn built_in_sample_recording_is_valid_and_ordered() {
        let recording = sample_recording(Some("CLI sample"));

        assert_eq!(recording.event_count(), 2);
        assert_eq!(recording.events()[0].sequence, 0);
        assert_eq!(recording.events()[1].sequence, 1);
        assert_eq!(recording.duration().as_micros(), 40_000);
        assert_eq!(recording.metadata().title.as_deref(), Some("CLI sample"));
    }

    #[test]
    fn bare_output_filenames_are_written_under_the_sessions_directory() {
        let resolved = resolve_session_output_path(Path::new("demo.json"));

        assert_eq!(resolved, PathBuf::from("sessions").join("demo.json"));
    }

    #[test]
    fn nested_relative_output_paths_are_respected() {
        let resolved = resolve_session_output_path(Path::new("fixtures/demo.json"));

        assert_eq!(resolved, PathBuf::from("fixtures").join("demo.json"));
    }

    #[test]
    fn missing_bare_input_filenames_fall_back_to_the_sessions_directory() {
        let resolved = resolve_session_input_path(Path::new("does-not-exist.json"));

        assert_eq!(
            resolved,
            PathBuf::from("sessions").join("does-not-exist.json")
        );
    }

    #[test]
    fn average_events_per_second_uses_recording_duration() {
        let metrics = RecordingMetrics {
            total_events: 120,
            duration_micros: 2_000_000,
            peak_events_per_second: 80,
            action_counts: RecordingActionCounts::default(),
        };

        assert_eq!(average_events_per_second(&metrics), 60.0);
    }

    #[test]
    fn binary_sizes_are_rendered_in_human_readable_units() {
        assert_eq!(format_binary_size(56), "56.00 B");
        assert_eq!(format_binary_size(1_536), "1.50 KiB");
        assert_eq!(format_binary_size(2_097_152), "2.00 MiB");
    }

    #[test]
    fn watch_hotkey_arguments_build_shared_hotkey_bindings() {
        let binding = hotkey_binding_from_arguments(
            &[
                HotkeyModifierArgument::Control,
                HotkeyModifierArgument::Shift,
            ],
            HotkeyKeyArgument::F9,
        )
        .expect("watch hotkey arguments should build");

        assert_eq!(binding.to_string(), "Ctrl+Shift+F9");
    }

    #[test]
    fn stop_controls_include_optional_hotkey_labels() {
        let stop_hotkey = "ctrl+shift+f10"
            .parse::<HotkeyBinding>()
            .expect("stop hotkeys should parse");

        assert_eq!(
            format_stop_controls(Some(&stop_hotkey)),
            "Ctrl+Shift+F10 or Ctrl+C"
        );
        assert_eq!(format_stop_controls(None), "Ctrl+C");
    }

    #[test]
    fn control_command_parser_accepts_record_and_play_hotkeys() {
        let cli = Cli::try_parse_from([
            "regxorder-cli",
            "control",
            "--record-output",
            "demo.json",
            "--start-record-hotkey",
            "ctrl+shift+f11",
            "--play-input",
            "demo.json",
            "--start-play-hotkey",
            "ctrl+shift+f10",
            "--stop-hotkey",
            "ctrl+shift+f12",
        ])
        .expect("control command should parse");

        match cli.command {
            Command::Control {
                record_output,
                start_record_hotkey,
                play_input,
                start_play_hotkey,
                stop_hotkey,
                ..
            } => {
                assert_eq!(record_output, Some(PathBuf::from("demo.json")));
                assert_eq!(play_input, Some(PathBuf::from("demo.json")));
                assert_eq!(
                    start_record_hotkey.map(|binding| binding.to_string()),
                    Some(String::from("Ctrl+Shift+F11"))
                );
                assert_eq!(
                    start_play_hotkey.map(|binding| binding.to_string()),
                    Some(String::from("Ctrl+Shift+F10"))
                );
                assert_eq!(stop_hotkey.to_string(), "Ctrl+Shift+F12");
            }
            _ => panic!("expected the control command variant"),
        }
    }

    #[test]
    fn control_command_rejects_record_output_without_a_start_hotkey() {
        let error = run_control_command(ControlCommandConfiguration {
            record_output: Some(PathBuf::from("demo.json")),
            record_title: None,
            record_duration_seconds: None,
            record_strategy: RecordingStrategyArgument::RawInput,
            start_record_hotkey: None,
            play_input: None,
            play_speed: 1.0,
            start_play_hotkey: None,
            stop_hotkey: "ctrl+shift+f12"
                .parse::<HotkeyBinding>()
                .expect("stop hotkey should parse"),
        })
        .expect_err("control command should reject incomplete recording configuration");

        assert!(matches!(
            error,
            CliError::InvalidControlRecordingConfiguration
        ));
    }

    #[test]
    fn control_command_rejects_play_input_without_a_start_hotkey() {
        let error = run_control_command(ControlCommandConfiguration {
            record_output: None,
            record_title: None,
            record_duration_seconds: None,
            record_strategy: RecordingStrategyArgument::RawInput,
            start_record_hotkey: None,
            play_input: Some(PathBuf::from("demo.json")),
            play_speed: 1.0,
            start_play_hotkey: None,
            stop_hotkey: "ctrl+shift+f12"
                .parse::<HotkeyBinding>()
                .expect("stop hotkey should parse"),
        })
        .expect_err("control command should reject incomplete playback configuration");

        assert!(matches!(
            error,
            CliError::InvalidControlPlaybackConfiguration
        ));
    }

    #[test]
    fn control_command_requires_at_least_one_start_action_hotkey() {
        let error = run_control_command(ControlCommandConfiguration {
            record_output: None,
            record_title: None,
            record_duration_seconds: None,
            record_strategy: RecordingStrategyArgument::RawInput,
            start_record_hotkey: None,
            play_input: None,
            play_speed: 1.0,
            start_play_hotkey: None,
            stop_hotkey: "ctrl+shift+f12"
                .parse::<HotkeyBinding>()
                .expect("stop hotkey should parse"),
        })
        .expect_err("control command should require at least one control action");

        assert!(matches!(
            error,
            CliError::WindowsBackend(WindowsBackendError::NoControlActionHotkeys)
        ));
    }

    #[test]
    fn doctor_playback_parser_accepts_json_output_and_speed() {
        let cli = Cli::try_parse_from([
            "regxorder-cli",
            "doctor",
            "--json",
            "playback",
            "--input",
            "demo.json",
            "--speed",
            "2.5",
        ])
        .expect("doctor playback command should parse");

        match cli.command {
            Command::Doctor {
                json,
                command: DoctorCommand::Playback { input, speed },
            } => {
                assert!(json);
                assert_eq!(input, PathBuf::from("demo.json"));
                assert_eq!(speed, 2.5);
            }
            _ => panic!("expected the doctor playback command variant"),
        }
    }

    #[test]
    fn recording_doctor_reports_missing_input_files_as_failures() {
        let report = build_recording_doctor_output(Path::new("missing-doctor-input.json"));

        assert!(report.summary.has_failures());
        assert!(report.recording.is_none());
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.name == "input_file" && check.status == DiagnosticStatus::Fail)
        );
    }

    #[test]
    fn playback_doctor_reports_preparation_cleanup_warnings() {
        let recording_path = unique_temp_path("doctor-playback-warning.json");
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
            ],
        )
        .expect("sample recording should be valid");
        write_recording(&recording_path, &recording)
            .expect("doctor test recording should be written");

        let report = build_playback_doctor_output(&recording_path, 1.0);

        assert!(report.summary.warn_count > 0);
        assert_eq!(
            report
                .playback
                .as_ref()
                .expect("playback report should be present")
                .preparation_report
                .skipped_unmatched_release_events,
            1
        );
        assert_eq!(
            report
                .playback
                .as_ref()
                .expect("playback report should be present")
                .preparation_report
                .appended_release_events,
            1
        );

        let _ = fs::remove_file(&recording_path);
    }

    #[test]
    fn control_doctor_reports_incomplete_recording_configuration_as_failures() {
        let report = build_control_doctor_output(DoctorControlConfiguration {
            record_output: Some(PathBuf::from("demo.json")),
            start_record_hotkey: None,
            play_input: None,
            play_speed: 1.0,
            start_play_hotkey: None,
            stop_hotkey: "ctrl+shift+f12"
                .parse::<HotkeyBinding>()
                .expect("stop hotkey should parse"),
        });

        assert!(report.summary.has_failures());
        assert!(report.checks.iter().any(|check| {
            check.name == "recording_action" && check.status == DiagnosticStatus::Fail
        }));
    }

    #[test]
    fn environment_doctor_warns_when_session_directory_is_missing() {
        let session_directory = unique_temp_path("doctor-session-directory");
        let base_report = regxorder_core::EnvironmentDoctorReport::new(
            String::from("windows"),
            String::from("x86_64"),
            vec![String::from("raw_input")],
            String::from("send_input"),
            String::from("register_hotkey"),
            "ctrl+alt+shift+f12"
                .parse::<HotkeyBinding>()
                .expect("doctor probe hotkey should parse"),
            vec![DiagnosticCheck::pass("hotkey_probe", "probe succeeded")],
        );

        let report = augment_environment_doctor_report_with_session_directory(
            base_report,
            &session_directory,
        );

        assert_eq!(report.summary.warn_count, 1);
        assert!(report.checks.iter().any(|check| {
            check.name == "session_directory" && check.status == DiagnosticStatus::Warn
        }));
    }

    fn sample_metadata() -> RecordingMetadata {
        RecordingMetadata {
            schema_version: SchemaVersion::new(1),
            title: Some(String::from("CLI doctor sample")),
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

    fn unique_temp_path(file_name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after the unix epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("regxorder-{timestamp}-{file_name}"))
    }
}
