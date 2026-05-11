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
    AbsoluteScreenPoint, DisplayMetadata, ElapsedTime, HotkeyBinding, HotkeyKey, HotkeyModifier,
    HotkeyParseError, InputAction, InputEvent, KeyDescriptor, Recording, RecordingError,
    RecordingMetadata, RecordingMetrics, ScanCode, SchemaVersion, ScreenSize, SpeedMultiplier,
    ValidationError,
};
use regxorder_win32::{
    ControlAction, ControlBindings, ControlController, HotkeyRegistration, HotkeyWaitOutcome,
    RecordingStrategy, WindowsBackendError, wait_for_hotkey_activation,
    wait_for_hotkey_press_and_release,
};
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
    use std::path::{Path, PathBuf};

    use super::{
        HotkeyKeyArgument, HotkeyModifierArgument, average_events_per_second, format_binary_size,
        format_stop_controls, hotkey_binding_from_arguments, resolve_session_input_path,
        resolve_session_output_path, sample_recording,
    };
    use regxorder_core::{HotkeyBinding, RecordingActionCounts, RecordingMetrics};

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
}
