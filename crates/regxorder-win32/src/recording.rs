use std::{
    mem,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use regxorder_core::{
    AbsoluteScreenPoint, DisplayMetadata, ElapsedTime, InputAction, InputEvent, KeyDescriptor,
    MouseButton, Recording, RecordingMetadata, ScanCode, SchemaVersion, ScreenSize,
};
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, GetSystemMetrics, HC_ACTION, HHOOK,
        KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
        PostThreadMessageW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
        WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
        XBUTTON1, XBUTTON2,
    },
};

use crate::WindowsBackendError;

const RECORD_STOP_MESSAGE: u32 = WM_APP + 1;

static HOOK_STATE: OnceLock<Mutex<Option<Arc<HookSharedState>>>> = OnceLock::new();

struct HookSharedState {
    started_at: Instant,
    display: DisplayMetadata,
    next_sequence: AtomicU64,
    events: Mutex<Vec<InputEvent>>,
}

impl HookSharedState {
    fn new(display: DisplayMetadata) -> Self {
        Self {
            started_at: Instant::now(),
            display,
            next_sequence: AtomicU64::new(0),
            events: Mutex::new(Vec::new()),
        }
    }

    fn record_action(&self, action: InputAction) {
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

    fn take_events(&self) -> Result<Vec<InputEvent>, WindowsBackendError> {
        let mut events = self
            .events
            .lock()
            .map_err(|_| WindowsBackendError::Internal("recorded event buffer was poisoned"))?;
        Ok(mem::take(&mut *events))
    }
}

/// Records input using low-level keyboard and mouse hooks until `stop_requested` becomes true.
pub fn record_with_low_level_hooks(
    stop_requested: &AtomicBool,
    title: Option<String>,
) -> Result<Recording, WindowsBackendError> {
    let display = capture_display_metadata();
    let metadata = RecordingMetadata {
        schema_version: SchemaVersion::new(1),
        title,
        display: display.clone(),
    };

    let (thread_id_tx, thread_id_rx) = mpsc::sync_channel(1);
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);

    let thread_display = display.clone();
    let recorder_thread =
        thread::spawn(move || run_hook_thread(thread_display, thread_id_tx, startup_tx));

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

fn run_hook_thread(
    display: DisplayMetadata,
    thread_id_tx: mpsc::SyncSender<u32>,
    startup_tx: mpsc::SyncSender<Result<(), WindowsBackendError>>,
) -> Result<Vec<InputEvent>, WindowsBackendError> {
    let thread_id = unsafe { GetCurrentThreadId() };
    thread_id_tx
        .send(thread_id)
        .map_err(|_| WindowsBackendError::Internal("failed to send recorder thread id"))?;

    let shared_state = Arc::new(HookSharedState::new(display));
    set_hook_state(Some(Arc::clone(&shared_state)))?;

    let mut keyboard_hook: HHOOK = std::ptr::null_mut();
    let mut mouse_hook: HHOOK = std::ptr::null_mut();

    let result = (|| {
        let module_handle = unsafe { GetModuleHandleW(std::ptr::null()) };
        if module_handle.is_null() {
            return Err(WindowsBackendError::last_os_error("GetModuleHandleW"));
        }

        keyboard_hook = unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), module_handle, 0)
        };
        if keyboard_hook.is_null() {
            return Err(WindowsBackendError::last_os_error(
                "SetWindowsHookExW(WH_KEYBOARD_LL)",
            ));
        }

        mouse_hook =
            unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), module_handle, 0) };
        if mouse_hook.is_null() {
            return Err(WindowsBackendError::last_os_error(
                "SetWindowsHookExW(WH_MOUSE_LL)",
            ));
        }

        startup_tx
            .send(Ok(()))
            .map_err(|_| WindowsBackendError::Internal("failed to signal recorder startup"))?;

        message_loop()?;
        shared_state.take_events()
    })();

    if !keyboard_hook.is_null() {
        unsafe {
            UnhookWindowsHookEx(keyboard_hook);
        }
    }
    if !mouse_hook.is_null() {
        unsafe {
            UnhookWindowsHookEx(mouse_hook);
        }
    }
    set_hook_state(None)?;

    if result.is_err() {
        let _ = startup_tx.send(Err(WindowsBackendError::Internal(
            "recorder thread exited before reporting success",
        )));
    }

    result
}

fn message_loop() -> Result<(), WindowsBackendError> {
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

fn post_stop_message(thread_id: u32) -> Result<(), WindowsBackendError> {
    let posted = unsafe { PostThreadMessageW(thread_id, RECORD_STOP_MESSAGE, 0, 0) };
    if posted == 0 {
        Err(WindowsBackendError::last_os_error("PostThreadMessageW"))
    } else {
        Ok(())
    }
}

fn hook_state_slot() -> &'static Mutex<Option<Arc<HookSharedState>>> {
    HOOK_STATE.get_or_init(|| Mutex::new(None))
}

fn set_hook_state(state: Option<Arc<HookSharedState>>) -> Result<(), WindowsBackendError> {
    let mut slot = hook_state_slot()
        .lock()
        .map_err(|_| WindowsBackendError::Internal("hook state mutex was poisoned"))?;
    *slot = state;
    Ok(())
}

fn active_hook_state() -> Option<Arc<HookSharedState>> {
    hook_state_slot().lock().ok()?.as_ref().map(Arc::clone)
}

unsafe extern "system" fn keyboard_hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code == HC_ACTION as i32 {
        if let Some(state) = active_hook_state() {
            let info = unsafe { &*(l_param as *const KBDLLHOOKSTRUCT) };
            if let Some(action) = translate_keyboard_message(w_param as u32, info) {
                state.record_action(action);
            }
        }
    }

    unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) }
}

unsafe extern "system" fn mouse_hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code == HC_ACTION as i32 {
        if let Some(state) = active_hook_state() {
            let info = unsafe { &*(l_param as *const MSLLHOOKSTRUCT) };
            if let Some(action) = translate_mouse_message(w_param as u32, info, &state.display) {
                state.record_action(action);
            }
        }
    }

    unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) }
}

fn translate_keyboard_message(message: u32, info: &KBDLLHOOKSTRUCT) -> Option<InputAction> {
    if info.flags & LLKHF_INJECTED != 0 {
        return None;
    }

    let key = KeyDescriptor {
        scan_code: ScanCode::new(info.scanCode as u16),
        logical_name: None,
        extended: info.flags & LLKHF_EXTENDED != 0,
    };

    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(InputAction::KeyPressed { key }),
        WM_KEYUP | WM_SYSKEYUP => Some(InputAction::KeyReleased { key }),
        _ => None,
    }
}

fn translate_mouse_message(
    message: u32,
    info: &MSLLHOOKSTRUCT,
    display: &DisplayMetadata,
) -> Option<InputAction> {
    if info.flags & LLMHF_INJECTED != 0 {
        return None;
    }

    match message {
        WM_MOUSEMOVE => display
            .pointer_position_for_absolute(AbsoluteScreenPoint {
                x: info.pt.x,
                y: info.pt.y,
            })
            .ok()
            .map(|position| InputAction::PointerMoved { position }),
        WM_LBUTTONDOWN => Some(InputAction::MouseButtonPressed {
            button: MouseButton::Left,
        }),
        WM_LBUTTONUP => Some(InputAction::MouseButtonReleased {
            button: MouseButton::Left,
        }),
        WM_RBUTTONDOWN => Some(InputAction::MouseButtonPressed {
            button: MouseButton::Right,
        }),
        WM_RBUTTONUP => Some(InputAction::MouseButtonReleased {
            button: MouseButton::Right,
        }),
        WM_MBUTTONDOWN => Some(InputAction::MouseButtonPressed {
            button: MouseButton::Middle,
        }),
        WM_MBUTTONUP => Some(InputAction::MouseButtonReleased {
            button: MouseButton::Middle,
        }),
        WM_XBUTTONDOWN => x_button_from_mouse_data(info.mouseData)
            .map(|button| InputAction::MouseButtonPressed { button }),
        WM_XBUTTONUP => x_button_from_mouse_data(info.mouseData)
            .map(|button| InputAction::MouseButtonReleased { button }),
        WM_MOUSEWHEEL => Some(InputAction::MouseWheelScrolled {
            axis: regxorder_core::ScrollAxis::Vertical,
            delta: wheel_delta(info.mouseData),
        }),
        WM_MOUSEHWHEEL => Some(InputAction::MouseWheelScrolled {
            axis: regxorder_core::ScrollAxis::Horizontal,
            delta: wheel_delta(info.mouseData),
        }),
        _ => None,
    }
}

fn wheel_delta(mouse_data: u32) -> i32 {
    ((mouse_data >> 16) as i16) as i32
}

fn x_button_from_mouse_data(mouse_data: u32) -> Option<MouseButton> {
    match mouse_data >> 16 {
        value if value == u32::from(XBUTTON1) => Some(MouseButton::X1),
        value if value == u32::from(XBUTTON2) => Some(MouseButton::X2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use regxorder_core::{AbsoluteScreenPoint, DisplayMetadata, MouseButton, ScreenSize};
    use windows_sys::Win32::{
        Foundation::POINT,
        UI::WindowsAndMessaging::{
            KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, MSLLHOOKSTRUCT, WM_KEYDOWN, WM_KEYUP,
            WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_XBUTTONDOWN, XBUTTON1,
        },
    };

    use regxorder_core::InputAction;

    use super::{
        capture_display_metadata, record_with_low_level_hooks, translate_keyboard_message,
        translate_mouse_message,
    };

    fn sample_display() -> DisplayMetadata {
        DisplayMetadata {
            virtual_origin: AbsoluteScreenPoint { x: 0, y: 0 },
            virtual_size: ScreenSize {
                width: 1920,
                height: 1080,
            },
            monitors: Vec::new(),
        }
    }

    #[test]
    fn keyboard_translation_maps_key_messages_into_actions() {
        let down = KBDLLHOOKSTRUCT {
            vkCode: 0,
            scanCode: 30,
            flags: LLKHF_EXTENDED,
            time: 0,
            dwExtraInfo: 0,
        };

        let down_action =
            translate_keyboard_message(WM_KEYDOWN, &down).expect("key down should translate");
        let up_action =
            translate_keyboard_message(WM_KEYUP, &down).expect("key up should translate");

        match down_action {
            InputAction::KeyPressed { key } => {
                assert_eq!(key.scan_code.get(), 30);
                assert!(key.extended);
            }
            other => panic!("unexpected action: {other:?}"),
        }

        assert!(matches!(up_action, InputAction::KeyReleased { .. }));
        assert!(
            translate_keyboard_message(
                WM_KEYDOWN,
                &KBDLLHOOKSTRUCT {
                    flags: LLKHF_INJECTED,
                    ..down
                },
            )
            .is_none()
        );
    }

    #[test]
    fn mouse_translation_maps_move_wheel_and_xbutton_messages() {
        let display = sample_display();
        let mouse = MSLLHOOKSTRUCT {
            pt: POINT { x: 960, y: 540 },
            mouseData: (120_i32 as u32) << 16,
            flags: 0,
            time: 0,
            dwExtraInfo: 0,
        };

        let move_action = translate_mouse_message(WM_MOUSEMOVE, &mouse, &display)
            .expect("mouse move should translate");
        let wheel_action = translate_mouse_message(WM_MOUSEWHEEL, &mouse, &display)
            .expect("mouse wheel should translate");
        let xbutton_action = translate_mouse_message(
            WM_XBUTTONDOWN,
            &MSLLHOOKSTRUCT {
                mouseData: u32::from(XBUTTON1) << 16,
                ..mouse
            },
            &display,
        )
        .expect("xbutton should translate");

        match move_action {
            InputAction::PointerMoved { position } => {
                assert_eq!(position.absolute, AbsoluteScreenPoint { x: 960, y: 540 });
            }
            other => panic!("unexpected action: {other:?}"),
        }

        assert!(matches!(
            wheel_action,
            InputAction::MouseWheelScrolled {
                axis: regxorder_core::ScrollAxis::Vertical,
                delta: 120
            }
        ));
        assert!(matches!(
            xbutton_action,
            InputAction::MouseButtonPressed {
                button: MouseButton::X1
            }
        ));
    }

    #[test]
    fn display_capture_returns_non_zero_virtual_dimensions() {
        let display = capture_display_metadata();

        assert!(display.virtual_size.width > 0);
        assert!(display.virtual_size.height > 0);
    }

    #[test]
    fn recorder_returns_empty_recording_when_stopped_immediately() {
        let stop_requested = AtomicBool::new(true);
        let recording = record_with_low_level_hooks(&stop_requested, Some(String::from("empty")))
            .expect("immediate stop should still produce a recording");

        assert_eq!(recording.event_count(), 0);
        assert_eq!(recording.metadata().title.as_deref(), Some("empty"));
    }
}
