use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use regxorder_core::{HotkeyBinding, HotkeyKey, HotkeyModifier};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

use crate::{HotkeyRegistration, WindowsBackendError, wait_for_hotkey_activation};

const HOTKEY_RELEASE_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// The result of waiting for a hotkey-controlled action boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyWaitOutcome {
    Activated,
    Cancelled,
}

/// Waits for a single shared hotkey binding to fire or for shutdown to be requested.
pub fn wait_for_hotkey_binding(
    binding: &HotkeyBinding,
    identifier: i32,
    stop_requested: &AtomicBool,
) -> Result<HotkeyWaitOutcome, WindowsBackendError> {
    if stop_requested.load(Ordering::SeqCst) {
        return Ok(HotkeyWaitOutcome::Cancelled);
    }

    match wait_for_hotkey_activation(
        &[HotkeyRegistration::from_binding(identifier, binding)],
        stop_requested,
    )? {
        Some(_) => Ok(HotkeyWaitOutcome::Activated),
        None => Ok(HotkeyWaitOutcome::Cancelled),
    }
}

/// Waits for a hotkey press and then for every key in the chord to be released.
pub fn wait_for_hotkey_press_and_release(
    binding: &HotkeyBinding,
    identifier: i32,
    stop_requested: &AtomicBool,
) -> Result<HotkeyWaitOutcome, WindowsBackendError> {
    match wait_for_hotkey_binding(binding, identifier, stop_requested)? {
        HotkeyWaitOutcome::Activated => Ok(wait_for_hotkey_release(binding, stop_requested)),
        HotkeyWaitOutcome::Cancelled => Ok(HotkeyWaitOutcome::Cancelled),
    }
}

/// Waits for every key in an already-activated hotkey chord to be released.
pub fn wait_for_hotkey_release(
    binding: &HotkeyBinding,
    stop_requested: &AtomicBool,
) -> HotkeyWaitOutcome {
    wait_for_hotkey_release_with(binding, stop_requested, is_virtual_key_pressed, || {
        thread::sleep(HOTKEY_RELEASE_POLL_INTERVAL)
    })
}

fn wait_for_hotkey_release_with<F, G>(
    binding: &HotkeyBinding,
    stop_requested: &AtomicBool,
    mut is_virtual_key_pressed: F,
    mut wait: G,
) -> HotkeyWaitOutcome
where
    F: FnMut(i32) -> bool,
    G: FnMut(),
{
    let binding_virtual_keys = binding_virtual_keys(binding);

    loop {
        if stop_requested.load(Ordering::SeqCst) {
            return HotkeyWaitOutcome::Cancelled;
        }

        if !binding_virtual_keys
            .iter()
            .copied()
            .any(&mut is_virtual_key_pressed)
        {
            return HotkeyWaitOutcome::Activated;
        }

        wait();
    }
}

fn binding_virtual_keys(binding: &HotkeyBinding) -> Vec<i32> {
    let mut binding_virtual_keys = Vec::with_capacity(binding.modifiers().len() + 2);

    for modifier in binding.modifiers() {
        match modifier {
            HotkeyModifier::Control => binding_virtual_keys.push(VK_CONTROL.into()),
            HotkeyModifier::Alt => binding_virtual_keys.push(VK_MENU.into()),
            HotkeyModifier::Shift => binding_virtual_keys.push(VK_SHIFT.into()),
            HotkeyModifier::Win => {
                binding_virtual_keys.push(VK_LWIN.into());
                binding_virtual_keys.push(VK_RWIN.into());
            }
        }
    }

    binding_virtual_keys.push(hotkey_virtual_key_code(binding.key()) as i32);
    binding_virtual_keys
}

fn is_virtual_key_pressed(virtual_key_code: i32) -> bool {
    unsafe { GetAsyncKeyState(virtual_key_code) < 0 }
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

/// A shared stop-hotkey monitor that flips an atomic stop flag when the binding fires.
#[derive(Debug)]
pub struct StopHotkeyMonitor {
    join_handle: thread::JoinHandle<Result<(), WindowsBackendError>>,
}

impl StopHotkeyMonitor {
    /// Starts a background listener for a stop hotkey.
    pub fn spawn(
        binding: &HotkeyBinding,
        identifier: i32,
        stop_requested: &Arc<AtomicBool>,
    ) -> Self {
        let binding = binding.clone();
        let stop_requested = Arc::clone(stop_requested);

        Self {
            join_handle: thread::spawn(move || {
                if wait_for_hotkey_binding(&binding, identifier, stop_requested.as_ref())?
                    == HotkeyWaitOutcome::Activated
                {
                    stop_requested.store(true, Ordering::SeqCst);
                }

                Ok(())
            }),
        }
    }

    /// Signals shutdown and waits for the monitor thread to exit.
    pub fn finish(self, stop_requested: &Arc<AtomicBool>) -> Result<(), WindowsBackendError> {
        stop_requested.store(true, Ordering::SeqCst);

        match self.join_handle.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(WindowsBackendError::ThreadPanic),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        sync::{Arc, atomic::AtomicBool},
    };

    use regxorder_core::HotkeyBinding;

    use super::{
        HotkeyWaitOutcome, StopHotkeyMonitor, wait_for_hotkey_binding, wait_for_hotkey_release_with,
    };

    #[test]
    fn wait_for_hotkey_binding_cancels_when_shutdown_is_requested() {
        let stop_requested = AtomicBool::new(true);
        let binding = "ctrl+shift+f9"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse");

        let outcome = wait_for_hotkey_binding(&binding, 1, &stop_requested)
            .expect("hotkey waiting should complete");

        assert_eq!(outcome, HotkeyWaitOutcome::Cancelled);
    }

    #[test]
    fn stop_hotkey_monitor_finishes_cleanly_after_shutdown() {
        let stop_requested = Arc::new(AtomicBool::new(true));
        let binding = "ctrl+shift+f10"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse");
        let monitor = StopHotkeyMonitor::spawn(&binding, 2, &stop_requested);

        monitor
            .finish(&stop_requested)
            .expect("stop hotkey monitors should finish cleanly");
    }

    #[test]
    fn hotkey_release_barrier_waits_until_the_chord_is_released() {
        let stop_requested = AtomicBool::new(false);
        let binding = "ctrl+f9"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse");
        let release_phase = Cell::new(0_u8);
        let wait_count = Cell::new(0_u8);

        let outcome = wait_for_hotkey_release_with(
            &binding,
            &stop_requested,
            |virtual_key_code| virtual_key_code == 0x11 && release_phase.get() == 0,
            || {
                wait_count.set(wait_count.get() + 1);
                release_phase.set(1);
            },
        );

        assert_eq!(outcome, HotkeyWaitOutcome::Activated);
        assert_eq!(wait_count.get(), 1);
    }

    #[test]
    fn hotkey_release_barrier_cancels_when_shutdown_is_requested() {
        let stop_requested = AtomicBool::new(true);
        let binding = "ctrl+f9"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse");

        let outcome = wait_for_hotkey_release_with(&binding, &stop_requested, |_| true, || {});

        assert_eq!(outcome, HotkeyWaitOutcome::Cancelled);
    }
}
