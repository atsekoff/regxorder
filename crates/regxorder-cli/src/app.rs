use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    mem::size_of,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use regxorder_core::{
    AbsoluteScreenPoint, DisplayMetadata, ElapsedTime, InputAction, InputEvent, KeyDescriptor,
    Recording, RecordingError, RecordingMetadata, RecordingMetrics, ScanCode, SchemaVersion,
    ScreenSize, SpeedMultiplier, ValidationError,
};
use regxorder_win32::{
    RecordingStrategy, WindowsBackendError, play_recording, record_with_strategy,
};
use thiserror::Error;

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
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RecordingStrategyArgument {
    RawInput,
    LowLevelHooks,
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
    #[error("failed to install Ctrl+C handler: {0}")]
    CtrlC(#[from] ctrlc::Error),
    #[error("duration_seconds must be finite and greater than zero, found {0}")]
    InvalidDuration(f64),
}

pub fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Sample { output, title } => write_sample_recording(&output, title.as_deref()),
        Command::Validate { input } => validate_recording(&input),
        Command::Inspect { input } => inspect_recording(&input),
        Command::Play { input, speed } => play_recording_file(&input, speed),
        Command::Record {
            output,
            title,
            duration_seconds,
            strategy,
        } => record_recording_file(&output, title, duration_seconds, strategy),
    }
}

fn record_recording_file(
    output: &Path,
    title: Option<String>,
    duration_seconds: Option<f64>,
    strategy: RecordingStrategyArgument,
) -> Result<(), CliError> {
    let resolved_output_path = resolve_session_output_path(output);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let stop_handler = Arc::clone(&stop_requested);
    let recording_strategy = strategy.into_backend_strategy();

    ctrlc::set_handler(move || {
        stop_handler.store(true, Ordering::SeqCst);
    })?;

    if let Some(seconds) = duration_seconds {
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(CliError::InvalidDuration(seconds));
        }

        let timed_stop = Arc::clone(&stop_requested);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
            timed_stop.store(true, Ordering::SeqCst);
        });

        println!(
            "recording with {} for up to {:.3} seconds; press Ctrl+C to stop early",
            recording_strategy, seconds
        );
    } else {
        println!(
            "recording with {}; press Ctrl+C to stop",
            recording_strategy
        );
    }

    let recording = record_with_strategy(recording_strategy, stop_requested.as_ref(), title)?;
    write_recording(&resolved_output_path, &recording)?;

    println!(
        "recorded {} events over {} us to {}",
        recording.event_count(),
        recording.duration().as_micros(),
        resolved_output_path.display()
    );
    print_recording_metrics(&recording, &resolved_output_path)?;

    Ok(())
}

fn play_recording_file(path: &Path, speed: f64) -> Result<(), CliError> {
    let resolved_input_path = resolve_session_input_path(path);
    let recording = load_recording(&resolved_input_path)?;
    let speed = SpeedMultiplier::new(speed)?;
    let stop_requested = Arc::new(AtomicBool::new(false));
    let stop_handler = Arc::clone(&stop_requested);

    ctrlc::set_handler(move || {
        stop_handler.store(true, Ordering::SeqCst);
    })?;

    println!(
        "playing {} events from {} at {}x speed; press Ctrl+C to stop",
        recording.event_count(),
        resolved_input_path.display(),
        speed.get()
    );

    let report = play_recording(&recording, speed, stop_requested.as_ref())?;

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
        average_events_per_second, format_binary_size, resolve_session_input_path,
        resolve_session_output_path, sample_recording,
    };
    use regxorder_core::{RecordingActionCounts, RecordingMetrics};

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
}
