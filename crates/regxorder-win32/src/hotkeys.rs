use std::{
    ops::{BitOr, BitOrAssign},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

use regxorder_core::{HotkeyBinding, HotkeyKey, HotkeyModifier};
use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{
            MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
            UnregisterHotKey,
        },
        WindowsAndMessaging::{
            GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, WM_APP, WM_HOTKEY, WM_QUIT,
        },
    },
};

use crate::WindowsBackendError;

const HOTKEY_STOP_THREAD_MESSAGE: u32 = WM_APP + 2;
const HOTKEY_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Bitflags for Windows global hotkey modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyModifiers(u32);

impl HotkeyModifiers {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn alt() -> Self {
        Self(MOD_ALT)
    }

    pub const fn control() -> Self {
        Self(MOD_CONTROL)
    }

    pub const fn shift() -> Self {
        Self(MOD_SHIFT)
    }

    pub const fn win() -> Self {
        Self(MOD_WIN)
    }

    const fn bits(self) -> u32 {
        self.0
    }
}

impl Default for HotkeyModifiers {
    fn default() -> Self {
        Self::empty()
    }
}

impl BitOr for HotkeyModifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for HotkeyModifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A single global hotkey registration handled by the Win32 backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyRegistration {
    identifier: i32,
    modifiers: HotkeyModifiers,
    virtual_key_code: u32,
    suppress_auto_repeat: bool,
}

impl HotkeyRegistration {
    /// Creates a new global hotkey registration.
    pub const fn new(identifier: i32, modifiers: HotkeyModifiers, virtual_key_code: u32) -> Self {
        Self {
            identifier,
            modifiers,
            virtual_key_code,
            suppress_auto_repeat: true,
        }
    }

    /// Allows a held key to retrigger the hotkey.
    pub const fn with_auto_repeat_enabled(mut self) -> Self {
        self.suppress_auto_repeat = false;
        self
    }

    /// Returns the application-defined identifier for this registration.
    pub const fn identifier(self) -> i32 {
        self.identifier
    }

    /// Returns the modifier set used to register this hotkey.
    pub const fn modifiers(self) -> HotkeyModifiers {
        self.modifiers
    }

    /// Returns the Windows virtual-key code for this hotkey.
    pub const fn virtual_key_code(self) -> u32 {
        self.virtual_key_code
    }

    /// Reports whether repeated keydown messages are suppressed.
    pub const fn suppresses_auto_repeat(self) -> bool {
        self.suppress_auto_repeat
    }

    /// Creates a Windows hotkey registration from the shared hotkey binding model.
    pub fn from_binding(identifier: i32, binding: &HotkeyBinding) -> Self {
        Self::new(
            identifier,
            binding
                .modifiers()
                .iter()
                .copied()
                .fold(HotkeyModifiers::empty(), |combined, modifier| {
                    combined | hotkey_modifier_flags(modifier)
                }),
            hotkey_virtual_key_code(binding.key()),
        )
    }

    fn registration_flags(self) -> u32 {
        self.modifiers.bits()
            | if self.suppress_auto_repeat {
                MOD_NOREPEAT
            } else {
                0
            }
    }

    fn description(self) -> String {
        format!(
            "modifiers=0x{:X}, vk=0x{:X}, suppress_auto_repeat={}",
            self.modifiers.bits(),
            self.virtual_key_code,
            self.suppress_auto_repeat
        )
    }
}

const fn hotkey_modifier_flags(modifier: HotkeyModifier) -> HotkeyModifiers {
    match modifier {
        HotkeyModifier::Control => HotkeyModifiers::control(),
        HotkeyModifier::Alt => HotkeyModifiers::alt(),
        HotkeyModifier::Shift => HotkeyModifiers::shift(),
        HotkeyModifier::Win => HotkeyModifiers::win(),
    }
}

const fn hotkey_virtual_key_code(key: HotkeyKey) -> u32 {
    match key {
        HotkeyKey::Escape => 0x1B,
        HotkeyKey::F1 => 0x70,
        HotkeyKey::F2 => 0x71,
        HotkeyKey::F3 => 0x72,
        HotkeyKey::F4 => 0x73,
        HotkeyKey::F5 => 0x74,
        HotkeyKey::F6 => 0x75,
        HotkeyKey::F7 => 0x76,
        HotkeyKey::F8 => 0x77,
        HotkeyKey::F9 => 0x78,
        HotkeyKey::F10 => 0x79,
        HotkeyKey::F11 => 0x7A,
        HotkeyKey::F12 => 0x7B,
    }
}

/// A hotkey activation reported by the Win32 backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyActivation {
    identifier: i32,
}

impl HotkeyActivation {
    /// Returns the identifier of the hotkey that fired.
    pub const fn identifier(self) -> i32 {
        self.identifier
    }
}

/// Waits until one of the registered global hotkeys fires or shutdown is requested.
pub fn wait_for_hotkey_activation(
    registrations: &[HotkeyRegistration],
    stop_requested: &AtomicBool,
) -> Result<Option<HotkeyActivation>, WindowsBackendError> {
    validate_hotkey_registrations(registrations)?;

    let registrations = registrations.to_vec();
    let (thread_id_tx, thread_id_rx) = mpsc::sync_channel(1);
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let (activation_tx, activation_rx) = mpsc::sync_channel(1);

    let listener_thread = thread::spawn(move || {
        run_hotkey_listener_thread(registrations, thread_id_tx, startup_tx, activation_tx)
    });

    let thread_id = thread_id_rx
        .recv()
        .map_err(|_| WindowsBackendError::Internal("failed to receive hotkey thread id"))?;
    let startup_result = startup_rx.recv().map_err(|_| {
        WindowsBackendError::Internal("failed to receive hotkey listener startup result")
    })?;

    if let Err(error) = startup_result {
        let _ = listener_thread.join();
        return Err(error);
    }

    let activation_result = loop {
        match activation_rx.recv_timeout(HOTKEY_POLL_INTERVAL) {
            Ok(result) => break result,
            Err(RecvTimeoutError::Timeout) => {
                if stop_requested.load(Ordering::SeqCst) {
                    post_hotkey_stop_message(thread_id)?;
                    break activation_rx.recv().map_err(|_| {
                        WindowsBackendError::Internal(
                            "hotkey listener exited before reporting activation state",
                        )
                    })?;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return match listener_thread.join() {
                    Ok(Ok(())) => Err(WindowsBackendError::Internal(
                        "hotkey listener exited before reporting activation state",
                    )),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err(WindowsBackendError::ThreadPanic),
                };
            }
        }
    };

    match listener_thread.join() {
        Ok(Ok(())) => activation_result,
        Ok(Err(error)) => Err(error),
        Err(_) => Err(WindowsBackendError::ThreadPanic),
    }
}

pub(crate) fn probe_hotkey_registration(
    registration: HotkeyRegistration,
) -> Result<(), WindowsBackendError> {
    let registrations = [registration];

    validate_hotkey_registrations(&registrations)?;
    ensure_hotkey_message_queue()?;
    register_hotkeys(&registrations)?;
    unregister_hotkeys(&registrations);

    Ok(())
}

fn validate_hotkey_registrations(
    registrations: &[HotkeyRegistration],
) -> Result<(), WindowsBackendError> {
    if registrations.is_empty() {
        return Err(WindowsBackendError::NoHotkeyRegistrations);
    }

    let mut identifiers = std::collections::BTreeSet::new();
    for registration in registrations {
        if !identifiers.insert(registration.identifier()) {
            return Err(WindowsBackendError::DuplicateHotkeyIdentifier {
                identifier: registration.identifier(),
            });
        }
    }

    Ok(())
}

fn run_hotkey_listener_thread(
    registrations: Vec<HotkeyRegistration>,
    thread_id_tx: mpsc::SyncSender<u32>,
    startup_tx: mpsc::SyncSender<Result<(), WindowsBackendError>>,
    activation_tx: mpsc::SyncSender<Result<Option<HotkeyActivation>, WindowsBackendError>>,
) -> Result<(), WindowsBackendError> {
    let thread_id = unsafe { GetCurrentThreadId() };
    thread_id_tx
        .send(thread_id)
        .map_err(|_| WindowsBackendError::Internal("failed to send hotkey thread id"))?;

    ensure_hotkey_message_queue()?;

    if let Err(error) = register_hotkeys(&registrations) {
        let _ = startup_tx.send(Err(error));
        return Ok(());
    }

    startup_tx
        .send(Ok(()))
        .map_err(|_| WindowsBackendError::Internal("failed to signal hotkey listener startup"))?;

    let activation_result = wait_for_hotkey_message();
    let _ = activation_tx.send(activation_result);

    unregister_hotkeys(&registrations);
    Ok(())
}

fn ensure_hotkey_message_queue() -> Result<(), WindowsBackendError> {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    let peeked = unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    if peeked == -1 {
        Err(WindowsBackendError::last_os_error("PeekMessageW"))
    } else {
        Ok(())
    }
}

fn register_hotkeys(registrations: &[HotkeyRegistration]) -> Result<(), WindowsBackendError> {
    for (registered_count, registration) in registrations.iter().enumerate() {
        let registered = unsafe {
            RegisterHotKey(
                std::ptr::null_mut::<std::ffi::c_void>() as HWND,
                registration.identifier(),
                registration.registration_flags(),
                registration.virtual_key_code(),
            )
        };

        if registered == 0 {
            unregister_hotkeys(&registrations[..registered_count]);
            return Err(WindowsBackendError::HotkeyRegistrationFailed {
                identifier: registration.identifier(),
                description: registration.description(),
                source: std::io::Error::last_os_error(),
            });
        }
    }

    Ok(())
}

fn unregister_hotkeys(registrations: &[HotkeyRegistration]) {
    for registration in registrations {
        unsafe {
            UnregisterHotKey(
                std::ptr::null_mut::<std::ffi::c_void>() as HWND,
                registration.identifier(),
            );
        }
    }
}

fn wait_for_hotkey_message() -> Result<Option<HotkeyActivation>, WindowsBackendError> {
    let mut message: MSG = unsafe { std::mem::zeroed() };

    loop {
        let status = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if status == -1 {
            return Err(WindowsBackendError::last_os_error("GetMessageW"));
        }

        if status == 0
            || message.message == HOTKEY_STOP_THREAD_MESSAGE
            || message.message == WM_QUIT
        {
            return Ok(None);
        }

        if message.message == WM_HOTKEY {
            return Ok(Some(HotkeyActivation {
                identifier: message.wParam as i32,
            }));
        }
    }
}

fn post_hotkey_stop_message(thread_id: u32) -> Result<(), WindowsBackendError> {
    let posted = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
            thread_id,
            HOTKEY_STOP_THREAD_MESSAGE,
            0,
            0,
        )
    };
    if posted == 0 {
        Err(WindowsBackendError::last_os_error("PostThreadMessageW"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use regxorder_core::HotkeyBinding;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT};

    use super::{HotkeyModifiers, HotkeyRegistration, wait_for_hotkey_activation};
    use crate::WindowsBackendError;

    #[test]
    fn hotkey_modifiers_combine_expected_windows_flags() {
        let modifiers =
            HotkeyModifiers::control() | HotkeyModifiers::shift() | HotkeyModifiers::alt();

        assert_eq!(modifiers.bits(), MOD_CONTROL | MOD_SHIFT | MOD_ALT);
    }

    #[test]
    fn hotkey_listener_rejects_empty_registration_lists() {
        let stop_requested = AtomicBool::new(false);
        let error = wait_for_hotkey_activation(&[], &stop_requested)
            .expect_err("empty hotkey lists should be rejected");

        assert!(matches!(error, WindowsBackendError::NoHotkeyRegistrations));
    }

    #[test]
    fn hotkey_listener_rejects_duplicate_identifiers() {
        let stop_requested = AtomicBool::new(false);
        let registrations = [
            HotkeyRegistration::new(1, HotkeyModifiers::control(), 0x78),
            HotkeyRegistration::new(1, HotkeyModifiers::shift(), 0x79),
        ];

        let error = wait_for_hotkey_activation(&registrations, &stop_requested)
            .expect_err("duplicate hotkey identifiers should be rejected");

        assert!(matches!(
            error,
            WindowsBackendError::DuplicateHotkeyIdentifier { identifier: 1 }
        ));
    }

    #[test]
    fn hotkey_registration_maps_shared_bindings_into_windows_values() {
        let binding = "ctrl+shift+f9"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse");
        let registration = HotkeyRegistration::from_binding(7, &binding);

        assert_eq!(registration.identifier(), 7);
        assert_eq!(
            registration.modifiers(),
            HotkeyModifiers::control() | HotkeyModifiers::shift()
        );
        assert_eq!(registration.virtual_key_code(), 0x78);
        assert!(registration.suppresses_auto_repeat());
    }
}
