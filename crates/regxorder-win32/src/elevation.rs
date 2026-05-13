use std::{
    ffi::{OsStr, OsString},
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::Path,
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, WAIT_FAILED, WAIT_OBJECT_0},
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
    },
    UI::{
        Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

use crate::WindowsBackendError;

/// The elevation state of the current regxorder process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessElevationStatus {
    Elevated,
    NotElevated,
}

/// Returns whether the current regxorder process is elevated.
pub fn current_process_elevation_status() -> Result<ProcessElevationStatus, WindowsBackendError> {
    let mut token_handle = std::ptr::null_mut();
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) };
    if opened == 0 {
        return Err(WindowsBackendError::last_os_error("OpenProcessToken"));
    }

    let mut token_elevation: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
    let mut returned_length = 0_u32;
    let queried = unsafe {
        GetTokenInformation(
            token_handle,
            TokenElevation,
            (&mut token_elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_length,
        )
    };
    let query_result = if queried == 0 {
        Err(WindowsBackendError::last_os_error("GetTokenInformation"))
    } else {
        Ok(if token_elevation.TokenIsElevated == 0 {
            ProcessElevationStatus::NotElevated
        } else {
            ProcessElevationStatus::Elevated
        })
    };

    unsafe {
        CloseHandle(token_handle);
    }

    query_result
}

/// Returns an explicit elevation-required error when the current process is not elevated.
pub fn ensure_current_process_is_elevated(
    operation: &'static str,
) -> Result<(), WindowsBackendError> {
    match current_process_elevation_status()? {
        ProcessElevationStatus::Elevated => Ok(()),
        ProcessElevationStatus::NotElevated => {
            Err(WindowsBackendError::ElevationRequired { operation })
        }
    }
}

/// Relaunches the current regxorder executable with elevation and waits for it to finish.
pub fn relaunch_process_elevated_and_wait(
    executable_path: &Path,
    arguments: &[OsString],
) -> Result<u32, WindowsBackendError> {
    let verb = encode_wide_with_nul(OsStr::new("runas"));
    let executable = encode_wide_with_nul(executable_path.as_os_str());
    let parameters = build_windows_command_line(arguments);
    let parameters_wide = encode_wide_with_nul(parameters.as_os_str());

    let mut execute_info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: if arguments.is_empty() {
            std::ptr::null()
        } else {
            parameters_wide.as_ptr()
        },
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };

    let launched = unsafe { ShellExecuteExW(&mut execute_info) };
    if launched == 0 {
        return Err(WindowsBackendError::last_os_error("ShellExecuteExW"));
    }

    if execute_info.hProcess.is_null() {
        return Err(WindowsBackendError::Internal(
            "ShellExecuteExW returned without a process handle",
        ));
    }

    let wait_result = unsafe { WaitForSingleObject(execute_info.hProcess, INFINITE) };
    if wait_result == WAIT_FAILED {
        let error = WindowsBackendError::last_os_error("WaitForSingleObject");
        unsafe {
            CloseHandle(execute_info.hProcess);
        }
        return Err(error);
    }

    if wait_result != WAIT_OBJECT_0 {
        unsafe {
            CloseHandle(execute_info.hProcess);
        }
        return Err(WindowsBackendError::Internal(
            "unexpected wait result for elevated child process",
        ));
    }

    let mut exit_code = 0_u32;
    let exit_code_result = unsafe { GetExitCodeProcess(execute_info.hProcess, &mut exit_code) };
    let result = if exit_code_result == 0 {
        Err(WindowsBackendError::last_os_error("GetExitCodeProcess"))
    } else {
        Ok(exit_code)
    };

    unsafe {
        CloseHandle(execute_info.hProcess);
    }

    result
}

fn encode_wide_with_nul(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn build_windows_command_line(arguments: &[OsString]) -> OsString {
    let mut command_line = Vec::new();

    for (index, argument) in arguments.iter().enumerate() {
        if index > 0 {
            command_line.push(u16::from(b' '));
        }

        append_windows_command_argument(&mut command_line, argument);
    }

    OsString::from_wide(&command_line)
}

fn append_windows_command_argument(command_line: &mut Vec<u16>, argument: &OsStr) {
    const BACKSLASH: u16 = b'\\' as u16;
    const DOUBLE_QUOTE: u16 = b'"' as u16;

    let argument_wide: Vec<u16> = argument.encode_wide().collect();
    let needs_quotes = argument_wide.is_empty()
        || argument_wide
            .iter()
            .any(|unit| matches!(*unit, 9 | 10 | 11 | 12 | 13 | 32))
        || argument_wide.contains(&DOUBLE_QUOTE);

    if !needs_quotes {
        command_line.extend(argument_wide);
        return;
    }

    command_line.push(DOUBLE_QUOTE);
    let mut trailing_backslashes = 0_usize;

    for unit in argument_wide {
        if unit == BACKSLASH {
            trailing_backslashes += 1;
            continue;
        }

        if unit == DOUBLE_QUOTE {
            command_line.extend(std::iter::repeat_n(BACKSLASH, trailing_backslashes * 2 + 1));
            command_line.push(DOUBLE_QUOTE);
            trailing_backslashes = 0;
            continue;
        }

        command_line.extend(std::iter::repeat_n(BACKSLASH, trailing_backslashes));
        command_line.push(unit);
        trailing_backslashes = 0;
    }

    command_line.extend(std::iter::repeat_n(BACKSLASH, trailing_backslashes * 2));
    command_line.push(DOUBLE_QUOTE);
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};

    use super::build_windows_command_line;

    #[test]
    fn windows_command_line_quotes_arguments_with_spaces() {
        let command_line = build_windows_command_line(&[
            OsString::from("play"),
            OsString::from("--input"),
            OsString::from(r"C:\Program Files\regxorder\demo file.json"),
        ]);

        assert_eq!(
            command_line,
            OsString::from(r#"play --input "C:\Program Files\regxorder\demo file.json""#)
        );
    }

    #[test]
    fn windows_command_line_escapes_embedded_quotes() {
        let command_line = build_windows_command_line(&[
            OsString::from("play"),
            OsString::from("--label"),
            OsString::from(r#"quoted "demo" value"#),
        ]);

        assert_eq!(
            command_line,
            OsString::from(r#"play --label "quoted \"demo\" value""#)
        );
    }

    #[test]
    fn windows_command_line_preserves_non_utf8_arguments() {
        let command_line = build_windows_command_line(&[
            OsString::from_wide(&[0x0061, 0x20AC]),
            OsString::from("sample"),
        ]);

        assert_eq!(
            command_line,
            OsString::from_wide(&[
                0x0061, 0x20AC, 32, 0x0073, 0x0061, 0x006D, 0x0070, 0x006C, 0x0065
            ])
        );
    }
}
