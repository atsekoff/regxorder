use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use regxorder_core::HotkeyBinding;

use crate::{HotkeyRegistration, WindowsBackendError, wait_for_hotkey_activation};

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
    match wait_for_hotkey_activation(
        &[HotkeyRegistration::from_binding(identifier, binding)],
        stop_requested,
    )? {
        Some(_) => Ok(HotkeyWaitOutcome::Activated),
        None => Ok(HotkeyWaitOutcome::Cancelled),
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
    use std::sync::{Arc, atomic::AtomicBool};

    use regxorder_core::HotkeyBinding;

    use super::{HotkeyWaitOutcome, StopHotkeyMonitor, wait_for_hotkey_binding};

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
}
