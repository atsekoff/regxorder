use std::{
    mem,
    sync::{Arc, Mutex, OnceLock, atomic::AtomicBool, mpsc},
};

use regxorder_core::{
    AbsoluteScreenPoint, DisplayMetadata, InputAction, InputEvent, KeyDescriptor, MouseButton,
    Recording, ScanCode,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::{
            GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RAWKEYBOARD,
            RAWMOUSE, RID_INPUT, RIDEV_INPUTSINK, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
            RegisterRawInputDevices,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, HWND_MESSAGE,
            RI_KEY_BREAK, RI_KEY_E0, RI_KEY_E1, RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP,
            RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP, RI_MOUSE_HWHEEL,
            RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_MIDDLE_BUTTON_DOWN,
            RI_MOUSE_MIDDLE_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP,
            RI_MOUSE_WHEEL, RegisterClassW, WM_INPUT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
            WM_SYSKEYUP, WNDCLASSW,
        },
    },
};

use crate::WindowsBackendError;

use super::{RecorderState, record_with_worker, thread_message_loop::run_thread_message_loop};

const RAW_INPUT_WINDOW_CLASS_NAME: &str = "regxorder-raw-input-window";
const WINDOW_CLASS_ALREADY_EXISTS: i32 = 1410;
const GENERIC_DESKTOP_USAGE_PAGE: u16 = 0x01;
const GENERIC_MOUSE_USAGE: u16 = 0x02;
const GENERIC_KEYBOARD_USAGE: u16 = 0x06;
const INITIAL_RAW_INPUT_BUFFER_CAPACITY: usize = 256;

static RAW_INPUT_STATE: OnceLock<Mutex<Option<Arc<RawInputRecordingState>>>> = OnceLock::new();

struct RawInputRecordingState {
    recorder_state: RecorderState,
    raw_input_buffer: Mutex<Vec<u8>>,
}

impl RawInputRecordingState {
    fn new(display: DisplayMetadata) -> Self {
        Self {
            recorder_state: RecorderState::new(display),
            raw_input_buffer: Mutex::new(Vec::with_capacity(INITIAL_RAW_INPUT_BUFFER_CAPACITY)),
        }
    }

    fn display(&self) -> &DisplayMetadata {
        &self.recorder_state.display
    }

    fn record_action(&self, action: InputAction) {
        self.recorder_state.record_action(action);
    }

    fn record_actions<I>(&self, actions: I)
    where
        I: IntoIterator<Item = InputAction>,
    {
        self.recorder_state.record_actions(actions);
    }

    fn take_events(&self) -> Result<Vec<InputEvent>, WindowsBackendError> {
        self.recorder_state.take_events()
    }
}

pub fn record_with_raw_input(
    stop_requested: &AtomicBool,
    title: Option<String>,
) -> Result<Recording, WindowsBackendError> {
    record_with_worker(stop_requested, title, run_raw_input_thread)
}

fn run_raw_input_thread(
    display: DisplayMetadata,
    thread_id_tx: mpsc::SyncSender<u32>,
    startup_tx: mpsc::SyncSender<Result<(), WindowsBackendError>>,
) -> Result<Vec<InputEvent>, WindowsBackendError> {
    let thread_id = unsafe { GetCurrentThreadId() };
    thread_id_tx
        .send(thread_id)
        .map_err(|_| WindowsBackendError::Internal("failed to send recorder thread id"))?;

    let shared_state = Arc::new(RawInputRecordingState::new(display));
    set_raw_input_state(Some(Arc::clone(&shared_state)))?;

    let mut window_handle: HWND = std::ptr::null_mut();
    let result = (|| {
        let module_handle = unsafe { GetModuleHandleW(std::ptr::null()) };
        if module_handle.is_null() {
            return Err(WindowsBackendError::last_os_error("GetModuleHandleW"));
        }

        let class_name = wide_null(RAW_INPUT_WINDOW_CLASS_NAME);
        register_raw_input_window_class(module_handle, &class_name)?;

        window_handle = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                module_handle,
                std::ptr::null_mut(),
            )
        };
        if window_handle.is_null() {
            return Err(WindowsBackendError::last_os_error("CreateWindowExW"));
        }

        register_raw_input_devices(window_handle)?;
        startup_tx
            .send(Ok(()))
            .map_err(|_| WindowsBackendError::Internal("failed to signal recorder startup"))?;

        run_thread_message_loop()?;
        shared_state.take_events()
    })();

    if !window_handle.is_null() {
        unsafe {
            DestroyWindow(window_handle);
        }
    }
    set_raw_input_state(None)?;

    if result.is_err() {
        let _ = startup_tx.send(Err(WindowsBackendError::Internal(
            "raw input recorder thread exited before reporting success",
        )));
    }

    result
}

fn register_raw_input_window_class(
    module_handle: windows_sys::Win32::Foundation::HINSTANCE,
    class_name: &[u16],
) -> Result<(), WindowsBackendError> {
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(raw_input_window_proc),
        hInstance: module_handle,
        lpszClassName: class_name.as_ptr(),
        ..unsafe { mem::zeroed() }
    };

    let class_atom = unsafe { RegisterClassW(&window_class) };
    if class_atom == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(WINDOW_CLASS_ALREADY_EXISTS) {
            return Err(WindowsBackendError::Os {
                context: "RegisterClassW",
                source: error,
            });
        }
    }

    Ok(())
}

fn register_raw_input_devices(window_handle: HWND) -> Result<(), WindowsBackendError> {
    let devices = [
        RAWINPUTDEVICE {
            usUsagePage: GENERIC_DESKTOP_USAGE_PAGE,
            usUsage: GENERIC_MOUSE_USAGE,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: window_handle,
        },
        RAWINPUTDEVICE {
            usUsagePage: GENERIC_DESKTOP_USAGE_PAGE,
            usUsage: GENERIC_KEYBOARD_USAGE,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: window_handle,
        },
    ];

    let registered = unsafe {
        RegisterRawInputDevices(
            devices.as_ptr(),
            devices.len() as u32,
            mem::size_of::<RAWINPUTDEVICE>() as u32,
        )
    };

    if registered == 0 {
        Err(WindowsBackendError::last_os_error(
            "RegisterRawInputDevices",
        ))
    } else {
        Ok(())
    }
}

unsafe extern "system" fn raw_input_window_proc(
    window_handle: HWND,
    message: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match message {
        WM_INPUT => {
            if let Some(state) = active_raw_input_state() {
                record_raw_input_packet(l_param as HRAWINPUT, &state);
            }

            0
        }
        _ => unsafe { DefWindowProcW(window_handle, message, w_param, l_param) },
    }
}

fn record_raw_input_packet(raw_input_handle: HRAWINPUT, state: &RawInputRecordingState) {
    let mut raw_input_size = 0_u32;
    let header_size = mem::size_of::<RAWINPUTHEADER>() as u32;
    let query_result = unsafe {
        GetRawInputData(
            raw_input_handle,
            RID_INPUT,
            std::ptr::null_mut(),
            &mut raw_input_size,
            header_size,
        )
    };
    if query_result == u32::MAX || raw_input_size == 0 {
        return;
    }

    let mut raw_input_buffer = match state.raw_input_buffer.lock() {
        Ok(raw_input_buffer) => raw_input_buffer,
        Err(_) => return,
    };
    raw_input_buffer.resize(raw_input_size as usize, 0);

    let read_result = unsafe {
        GetRawInputData(
            raw_input_handle,
            RID_INPUT,
            raw_input_buffer.as_mut_ptr().cast(),
            &mut raw_input_size,
            header_size,
        )
    };
    if read_result == u32::MAX {
        return;
    }

    let raw_input = unsafe { &*(raw_input_buffer.as_ptr() as *const RAWINPUT) };
    match raw_input.header.dwType {
        value if value == RIM_TYPEKEYBOARD => unsafe {
            if let Some(action) = translate_raw_keyboard(&raw_input.data.keyboard) {
                state.record_action(action);
            }
        },
        value if value == RIM_TYPEMOUSE => unsafe {
            record_raw_mouse(&raw_input.data.mouse, state);
        },
        _ => {}
    }
}

fn translate_raw_keyboard(raw_keyboard: &RAWKEYBOARD) -> Option<InputAction> {
    if raw_keyboard.VKey == 255 {
        return None;
    }

    let key_flags = u32::from(raw_keyboard.Flags);
    let key = KeyDescriptor {
        scan_code: ScanCode::new(raw_keyboard.MakeCode),
        logical_name: None,
        extended: key_flags & (RI_KEY_E0 | RI_KEY_E1) != 0,
    };

    if key_flags & RI_KEY_BREAK != 0 {
        return Some(InputAction::KeyReleased { key });
    }

    match raw_keyboard.Message {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(InputAction::KeyPressed { key }),
        WM_KEYUP | WM_SYSKEYUP => Some(InputAction::KeyReleased { key }),
        _ => Some(InputAction::KeyPressed { key }),
    }
}

fn record_raw_mouse(raw_mouse: &RAWMOUSE, state: &RawInputRecordingState) {
    let pointer_move_action = if raw_mouse.lLastX != 0 || raw_mouse.lLastY != 0 {
        current_pointer_position(state.display()).map(|pointer_position| {
            InputAction::PointerMoved {
                position: pointer_position,
            }
        })
    } else {
        None
    };

    let raw_button_state = unsafe { raw_mouse.Anonymous.Anonymous };
    state.record_actions(
        pointer_move_action
            .into_iter()
            .chain(raw_mouse_button_actions(
                raw_button_state.usButtonFlags,
                raw_button_state.usButtonData,
            )),
    );
}

fn current_pointer_position(display: &DisplayMetadata) -> Option<regxorder_core::PointerPosition> {
    let mut point = POINT { x: 0, y: 0 };
    let cursor_query_succeeded = unsafe { GetCursorPos(&mut point) };
    if cursor_query_succeeded == 0 {
        return None;
    }

    display
        .pointer_position_for_absolute(AbsoluteScreenPoint {
            x: point.x,
            y: point.y,
        })
        .ok()
}

fn raw_mouse_button_actions(
    button_flags: u16,
    button_data: u16,
) -> impl Iterator<Item = InputAction> {
    let button_flags = u32::from(button_flags);
    let wheel_delta = i32::from(button_data as i16);

    [
        (button_flags & RI_MOUSE_LEFT_BUTTON_DOWN != 0).then_some(
            InputAction::MouseButtonPressed {
                button: MouseButton::Left,
            },
        ),
        (button_flags & RI_MOUSE_LEFT_BUTTON_UP != 0).then_some(InputAction::MouseButtonReleased {
            button: MouseButton::Left,
        }),
        (button_flags & RI_MOUSE_RIGHT_BUTTON_DOWN != 0).then_some(
            InputAction::MouseButtonPressed {
                button: MouseButton::Right,
            },
        ),
        (button_flags & RI_MOUSE_RIGHT_BUTTON_UP != 0).then_some(
            InputAction::MouseButtonReleased {
                button: MouseButton::Right,
            },
        ),
        (button_flags & RI_MOUSE_MIDDLE_BUTTON_DOWN != 0).then_some(
            InputAction::MouseButtonPressed {
                button: MouseButton::Middle,
            },
        ),
        (button_flags & RI_MOUSE_MIDDLE_BUTTON_UP != 0).then_some(
            InputAction::MouseButtonReleased {
                button: MouseButton::Middle,
            },
        ),
        (button_flags & RI_MOUSE_BUTTON_4_DOWN != 0).then_some(InputAction::MouseButtonPressed {
            button: MouseButton::X1,
        }),
        (button_flags & RI_MOUSE_BUTTON_4_UP != 0).then_some(InputAction::MouseButtonReleased {
            button: MouseButton::X1,
        }),
        (button_flags & RI_MOUSE_BUTTON_5_DOWN != 0).then_some(InputAction::MouseButtonPressed {
            button: MouseButton::X2,
        }),
        (button_flags & RI_MOUSE_BUTTON_5_UP != 0).then_some(InputAction::MouseButtonReleased {
            button: MouseButton::X2,
        }),
        (button_flags & RI_MOUSE_WHEEL != 0).then_some(InputAction::MouseWheelScrolled {
            axis: regxorder_core::ScrollAxis::Vertical,
            delta: wheel_delta,
        }),
        (button_flags & RI_MOUSE_HWHEEL != 0).then_some(InputAction::MouseWheelScrolled {
            axis: regxorder_core::ScrollAxis::Horizontal,
            delta: wheel_delta,
        }),
    ]
    .into_iter()
    .flatten()
}

fn raw_input_state_slot() -> &'static Mutex<Option<Arc<RawInputRecordingState>>> {
    RAW_INPUT_STATE.get_or_init(|| Mutex::new(None))
}

fn set_raw_input_state(
    state: Option<Arc<RawInputRecordingState>>,
) -> Result<(), WindowsBackendError> {
    let mut slot = raw_input_state_slot()
        .lock()
        .map_err(|_| WindowsBackendError::Internal("raw input state mutex was poisoned"))?;
    *slot = state;
    Ok(())
}

fn active_raw_input_state() -> Option<Arc<RawInputRecordingState>> {
    raw_input_state_slot().lock().ok()?.as_ref().map(Arc::clone)
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use regxorder_core::{InputAction, MouseButton};
    use windows_sys::Win32::UI::{
        Input::{RAWKEYBOARD, RAWMOUSE},
        WindowsAndMessaging::{
            RI_KEY_BREAK, RI_KEY_E0, RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_LEFT_BUTTON_DOWN,
            RI_MOUSE_WHEEL, WM_KEYDOWN,
        },
    };

    use super::{raw_mouse_button_actions, translate_raw_keyboard};

    #[test]
    fn raw_keyboard_translation_uses_break_and_extended_flags() {
        let raw_key_down = RAWKEYBOARD {
            MakeCode: 30,
            Flags: RI_KEY_E0 as u16,
            Reserved: 0,
            VKey: 0,
            Message: WM_KEYDOWN,
            ExtraInformation: 0,
        };
        let raw_key_up = RAWKEYBOARD {
            Flags: (RI_KEY_E0 | RI_KEY_BREAK) as u16,
            ..raw_key_down
        };

        let pressed_action =
            translate_raw_keyboard(&raw_key_down).expect("raw key-down input should translate");
        let released_action =
            translate_raw_keyboard(&raw_key_up).expect("raw key-up input should translate");

        match pressed_action {
            InputAction::KeyPressed { key } => {
                assert_eq!(key.scan_code.get(), 30);
                assert!(key.extended);
            }
            other => panic!("unexpected action: {other:?}"),
        }

        assert!(matches!(released_action, InputAction::KeyReleased { .. }));
    }

    #[test]
    fn raw_mouse_button_decoder_handles_buttons_and_wheels() {
        let actions: Vec<_> = raw_mouse_button_actions(
            (RI_MOUSE_LEFT_BUTTON_DOWN | RI_MOUSE_WHEEL | RI_MOUSE_BUTTON_4_DOWN) as u16,
            120_i16 as u16,
        )
        .collect();

        assert!(matches!(
            actions[0],
            InputAction::MouseButtonPressed {
                button: MouseButton::Left
            }
        ));
        assert!(matches!(
            actions[1],
            InputAction::MouseButtonPressed {
                button: MouseButton::X1
            }
        ));
        assert!(matches!(
            actions[2],
            InputAction::MouseWheelScrolled {
                axis: regxorder_core::ScrollAxis::Vertical,
                delta: 120
            }
        ));
    }

    #[test]
    fn raw_mouse_button_decoder_ignores_empty_flag_sets() {
        let actions: Vec<_> = raw_mouse_button_actions(0, 0).collect();

        assert!(actions.is_empty());
    }

    #[test]
    fn raw_mouse_type_is_constructible_for_strategy_code_paths() {
        let raw_mouse = RAWMOUSE::default();

        assert_eq!(raw_mouse.lLastX, 0);
        assert_eq!(raw_mouse.lLastY, 0);
    }
}
