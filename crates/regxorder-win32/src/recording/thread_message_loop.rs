use std::mem;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP, WM_QUIT,
};

use crate::WindowsBackendError;

const RECORD_STOP_THREAD_MESSAGE: u32 = WM_APP + 1;

pub(super) fn run_thread_message_loop() -> Result<(), WindowsBackendError> {
    let mut message: MSG = unsafe { mem::zeroed() };

    loop {
        let status = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if status == -1 {
            return Err(WindowsBackendError::last_os_error("GetMessageW"));
        }
        if status == 0
            || message.message == RECORD_STOP_THREAD_MESSAGE
            || message.message == WM_QUIT
        {
            break;
        }

        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    Ok(())
}

pub(super) fn post_stop_message(thread_id: u32) -> Result<(), WindowsBackendError> {
    let posted = unsafe { PostThreadMessageW(thread_id, RECORD_STOP_THREAD_MESSAGE, 0, 0) };
    if posted == 0 {
        Err(WindowsBackendError::last_os_error("PostThreadMessageW"))
    } else {
        Ok(())
    }
}
