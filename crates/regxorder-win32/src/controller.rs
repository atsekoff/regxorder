use std::sync::{Arc, atomic::AtomicBool};

use regxorder_core::{
    HotkeyBinding, PlaybackPreparationReport, Recording, RecordingFinalizationReport,
    SpeedMultiplier, finalize_recording_session, prepare_playback_plan,
};

use crate::{
    HotkeyRegistration, PlaybackReport, RecordingStrategy, StopHotkeyMonitor, WindowsBackendError,
    play_recording, record_with_strategy, wait_for_hotkey_activation, wait_for_hotkey_release,
};

const CONTROL_RECORD_IDENTIFIER: i32 = 1;
const CONTROL_PLAY_IDENTIFIER: i32 = 2;
const CONTROL_STOP_IDENTIFIER: i32 = 3;

/// A hotkey-triggered control action that can be run while the controller is idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Record,
    Play,
}

/// The set of idle hotkeys that can trigger control actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlBindings {
    start_recording: Option<HotkeyBinding>,
    start_playback: Option<HotkeyBinding>,
}

impl ControlBindings {
    /// Creates validated control bindings for the idle controller loop.
    pub fn new(
        start_recording: Option<HotkeyBinding>,
        start_playback: Option<HotkeyBinding>,
    ) -> Result<Self, WindowsBackendError> {
        if start_recording.is_none() && start_playback.is_none() {
            return Err(WindowsBackendError::NoControlActionHotkeys);
        }

        if start_recording.is_some() && start_recording == start_playback {
            return Err(WindowsBackendError::DuplicateControlActionHotkey {
                binding: start_recording
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            });
        }

        Ok(Self {
            start_recording,
            start_playback,
        })
    }

    fn registrations(&self) -> Vec<HotkeyRegistration> {
        let mut registrations = Vec::with_capacity(2);

        if let Some(start_recording) = &self.start_recording {
            registrations.push(HotkeyRegistration::from_binding(
                CONTROL_RECORD_IDENTIFIER,
                start_recording,
            ));
        }

        if let Some(start_playback) = &self.start_playback {
            registrations.push(HotkeyRegistration::from_binding(
                CONTROL_PLAY_IDENTIFIER,
                start_playback,
            ));
        }

        registrations
    }

    fn binding_for_action(&self, action: ControlAction) -> Option<&HotkeyBinding> {
        match action {
            ControlAction::Record => self.start_recording.as_ref(),
            ControlAction::Play => self.start_playback.as_ref(),
        }
    }
}

/// A finalized recording result produced by the shared controller.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingActionOutcome {
    pub recording: Recording,
    pub finalization_report: RecordingFinalizationReport,
}

/// A playback result produced by the shared controller after preparation and dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackActionOutcome {
    pub playback_report: PlaybackReport,
    pub preparation_report: PlaybackPreparationReport,
    pub prepared_event_count: usize,
}

/// Shared orchestration over hotkeys, recording finalization, and playback preparation.
#[derive(Debug, Default, Clone, Copy)]
pub struct ControlController;

impl ControlController {
    /// Waits for the next idle control action to fire and clears the start chord before returning.
    pub fn wait_for_next_action(
        self,
        bindings: &ControlBindings,
        stop_requested: &AtomicBool,
    ) -> Result<Option<ControlAction>, WindowsBackendError> {
        let Some(activation) =
            wait_for_hotkey_activation(&bindings.registrations(), stop_requested)?
        else {
            return Ok(None);
        };

        let action = match activation.identifier() {
            CONTROL_RECORD_IDENTIFIER => ControlAction::Record,
            CONTROL_PLAY_IDENTIFIER => ControlAction::Play,
            _ => {
                return Err(WindowsBackendError::Internal(
                    "unknown control action hotkey",
                ));
            }
        };

        let Some(binding) = bindings.binding_for_action(action) else {
            return Err(WindowsBackendError::Internal(
                "missing binding for activated control action",
            ));
        };

        match wait_for_hotkey_release(binding, stop_requested) {
            crate::HotkeyWaitOutcome::Activated => Ok(Some(action)),
            crate::HotkeyWaitOutcome::Cancelled => Ok(None),
        }
    }

    /// Runs a recording action, applies session-boundary cleanup, and returns the finalized recording.
    pub fn run_recording_action(
        self,
        strategy: RecordingStrategy,
        title: Option<String>,
        stop_hotkey: Option<&HotkeyBinding>,
        stop_requested: &Arc<AtomicBool>,
    ) -> Result<RecordingActionOutcome, WindowsBackendError> {
        let stop_hotkey_monitor = spawn_optional_action_stop_hotkey(stop_hotkey, stop_requested);
        let recording_result = record_with_strategy(strategy, stop_requested.as_ref(), title);
        let stop_hotkey_result =
            finish_optional_action_stop_hotkey(stop_requested, stop_hotkey_monitor);
        let recording = recording_result?;
        stop_hotkey_result?;

        let (recording, finalization_report) = finalize_recording_session(recording)?;

        Ok(RecordingActionOutcome {
            recording,
            finalization_report,
        })
    }

    /// Builds a prepared playback plan, runs playback, and reports the cleanup that was applied.
    pub fn run_playback_action(
        self,
        recording: &Recording,
        speed: SpeedMultiplier,
        stop_hotkey: Option<&HotkeyBinding>,
        stop_requested: &Arc<AtomicBool>,
    ) -> Result<PlaybackActionOutcome, WindowsBackendError> {
        let prepared_playback_plan = prepare_playback_plan(recording)?;
        let prepared_event_count = prepared_playback_plan.event_count();
        let preparation_report = prepared_playback_plan.report();
        let stop_hotkey_monitor = spawn_optional_action_stop_hotkey(stop_hotkey, stop_requested);
        let playback_result = play_recording(
            prepared_playback_plan.recording(),
            speed,
            stop_requested.as_ref(),
        );
        let stop_hotkey_result =
            finish_optional_action_stop_hotkey(stop_requested, stop_hotkey_monitor);
        let playback_report = playback_result?;
        stop_hotkey_result?;

        Ok(PlaybackActionOutcome {
            playback_report,
            preparation_report,
            prepared_event_count,
        })
    }
}

fn spawn_optional_action_stop_hotkey(
    stop_hotkey: Option<&HotkeyBinding>,
    stop_requested: &Arc<AtomicBool>,
) -> Option<StopHotkeyMonitor> {
    let stop_hotkey = stop_hotkey?;
    Some(StopHotkeyMonitor::spawn(
        stop_hotkey,
        CONTROL_STOP_IDENTIFIER,
        stop_requested,
    ))
}

fn finish_optional_action_stop_hotkey(
    stop_requested: &Arc<AtomicBool>,
    stop_hotkey_monitor: Option<StopHotkeyMonitor>,
) -> Result<(), WindowsBackendError> {
    let Some(stop_hotkey_monitor) = stop_hotkey_monitor else {
        return Ok(());
    };

    stop_hotkey_monitor.finish(stop_requested)
}

#[cfg(test)]
mod tests {
    use regxorder_core::HotkeyBinding;

    use super::ControlBindings;
    use crate::WindowsBackendError;

    #[test]
    fn control_bindings_require_at_least_one_start_hotkey() {
        let error = ControlBindings::new(None, None)
            .expect_err("control bindings should require at least one action");

        assert!(matches!(error, WindowsBackendError::NoControlActionHotkeys));
    }

    #[test]
    fn control_bindings_reject_duplicate_start_hotkeys() {
        let record_binding = "ctrl+shift+f11"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse");
        let play_binding = "ctrl+shift+f11"
            .parse::<HotkeyBinding>()
            .expect("hotkey bindings should parse");
        let error = ControlBindings::new(Some(record_binding), Some(play_binding))
            .expect_err("duplicate action hotkeys should be rejected");

        assert!(matches!(
            error,
            WindowsBackendError::DuplicateControlActionHotkey { .. }
        ));
    }
}
