mod low_level_hooks;
mod raw_input;

use std::{
    fmt, mem,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use regxorder_core::{
    AbsoluteScreenPoint, DisplayMetadata, ElapsedTime, InputAction, InputEvent, Recording,
    RecordingMetadata, SchemaVersion, ScreenSize,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetSystemMetrics, MSG, PostThreadMessageW, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, TranslateMessage, WM_APP, WM_QUIT,
};

use crate::WindowsBackendError;

pub use low_level_hooks::record_with_low_level_hooks;
pub use raw_input::record_with_raw_input;

pub(super) const RECORD_STOP_MESSAGE: u32 = WM_APP + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingStrategy {
    RawInput,
    LowLevelHooks,
}

impl RecordingStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RawInput => "raw-input",
            Self::LowLevelHooks => "low-level-hooks",
        }
    }
}

impl fmt::Display for RecordingStrategy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub(super) struct RecorderState {
    started_at: Instant,
    display: DisplayMetadata,
    next_sequence: AtomicU64,
    events: Mutex<Vec<InputEvent>>,
}

impl RecorderState {
    pub(super) fn new(display: DisplayMetadata) -> Self {
        Self {
            started_at: Instant::now(),
            display,
            next_sequence: AtomicU64::new(0),
            events: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn record_action(&self, action: InputAction) {
        let elapsed = self.started_at.elapsed().as_micros();
        let elapsed_time_micros = u64::try_from(elapsed).unwrap_or(u64::MAX);
        let event = InputEvent {
            sequence: self.next_sequence.fetch_add(1, Ordering::SeqCst),
            elapsed_time: ElapsedTime::from_micros(elapsed_time_micros),
            action,
        };

        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    pub(super) fn record_actions<I>(&self, actions: I)
    where
        I: IntoIterator<Item = InputAction>,
    {
        for action in actions {
            self.record_action(action);
        }
    }

    pub(super) fn take_events(&self) -> Result<Vec<InputEvent>, WindowsBackendError> {
        let mut events = self
            .events
            .lock()
            .map_err(|_| WindowsBackendError::Internal("recorded event buffer was poisoned"))?;
        Ok(mem::take(&mut *events))
    }
}

pub fn record_with_strategy(
    strategy: RecordingStrategy,
    stop_requested: &AtomicBool,
    title: Option<String>,
) -> Result<Recording, WindowsBackendError> {
    match strategy {
        RecordingStrategy::RawInput => record_with_raw_input(stop_requested, title),
        RecordingStrategy::LowLevelHooks => record_with_low_level_hooks(stop_requested, title),
    }
}

pub(super) fn record_with_worker<F>(
    stop_requested: &AtomicBool,
    title: Option<String>,
    worker: F,
) -> Result<Recording, WindowsBackendError>
where
    F: FnOnce(
            DisplayMetadata,
            mpsc::SyncSender<u32>,
            mpsc::SyncSender<Result<(), WindowsBackendError>>,
        ) -> Result<Vec<InputEvent>, WindowsBackendError>
        + Send
        + 'static,
{
    let display = capture_display_metadata();
    let metadata = RecordingMetadata {
        schema_version: SchemaVersion::new(1),
        title,
        display: display.clone(),
    };

    let (thread_id_tx, thread_id_rx) = mpsc::sync_channel(1);
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);

    let thread_display = display.clone();
    let recorder_thread = thread::spawn(move || worker(thread_display, thread_id_tx, startup_tx));

    let thread_id = thread_id_rx
        .recv()
        .map_err(|_| WindowsBackendError::Internal("failed to receive recorder thread id"))?;
    let startup_result = startup_rx
        .recv()
        .map_err(|_| WindowsBackendError::Internal("failed to receive recorder startup result"))?;

    if let Err(error) = startup_result {
        let _ = recorder_thread.join();
        return Err(error);
    }

    while !stop_requested.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10));
    }

    post_stop_message(thread_id)?;

    let events = match recorder_thread.join() {
        Ok(result) => result?,
        Err(_) => return Err(WindowsBackendError::ThreadPanic),
    };

    Recording::new(metadata, events).map_err(WindowsBackendError::from)
}

pub(super) fn message_loop() -> Result<(), WindowsBackendError> {
    let mut message: MSG = unsafe { mem::zeroed() };

    loop {
        let status = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if status == -1 {
            return Err(WindowsBackendError::last_os_error("GetMessageW"));
        }
        if status == 0 || message.message == RECORD_STOP_MESSAGE || message.message == WM_QUIT {
            break;
        }

        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    Ok(())
}

fn capture_display_metadata() -> DisplayMetadata {
    DisplayMetadata {
        virtual_origin: AbsoluteScreenPoint {
            x: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
            y: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
        },
        virtual_size: ScreenSize {
            width: unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32 },
            height: unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32 },
        },
        monitors: Vec::new(),
    }
}

pub(super) fn post_stop_message(thread_id: u32) -> Result<(), WindowsBackendError> {
    let posted = unsafe { PostThreadMessageW(thread_id, RECORD_STOP_MESSAGE, 0, 0) };
    if posted == 0 {
        Err(WindowsBackendError::last_os_error("PostThreadMessageW"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{RecordingStrategy, capture_display_metadata};

    #[test]
    fn display_capture_returns_non_zero_virtual_dimensions() {
        let display = capture_display_metadata();

        assert!(display.virtual_size.width > 0);
        assert!(display.virtual_size.height > 0);
    }

    #[test]
    fn recording_strategy_names_are_stable_for_cli_and_docs() {
        assert_eq!(RecordingStrategy::RawInput.as_str(), "raw-input");
        assert_eq!(RecordingStrategy::LowLevelHooks.as_str(), "low-level-hooks");
    }
}
