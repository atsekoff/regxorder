use std::{
    hint::spin_loop,
    mem::size_of,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use regxorder_core::{
    AbsoluteScreenPoint, DisplayMetadata, InputAction, KeyDescriptor, MouseButton, Recording,
    SpeedMultiplier,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
};

use crate::{ProcessElevationStatus, WindowsBackendError, current_process_elevation_status};

const SPIN_THRESHOLD: Duration = Duration::from_millis(2);
const XBUTTON1_DATA: u32 = 0x0001;
const XBUTTON2_DATA: u32 = 0x0002;

/// A summary of a completed playback run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackReport {
    pub dispatched_events: usize,
    pub interrupted: bool,
    pub elapsed: Duration,
    pub speed: SpeedMultiplier,
}

#[derive(Debug, Default)]
struct PressedState {
    keys: Vec<KeyDescriptor>,
    buttons: Vec<MouseButton>,
}

impl PressedState {
    fn note_action(&mut self, action: &InputAction) {
        match action {
            InputAction::KeyPressed { key } => {
                if !self.keys.iter().any(|pressed| pressed == key) {
                    self.keys.push(key.clone());
                }
            }
            InputAction::KeyReleased { key } => {
                self.keys.retain(|pressed| pressed != key);
            }
            InputAction::MouseButtonPressed { button } => {
                if !self.buttons.iter().any(|pressed| pressed == button) {
                    self.buttons.push(*button);
                }
            }
            InputAction::MouseButtonReleased { button } => {
                self.buttons.retain(|pressed| pressed != button);
            }
            InputAction::PointerMoved { .. } | InputAction::MouseWheelScrolled { .. } => {}
        }
    }

    fn cleanup_inputs(&self) -> Vec<INPUT> {
        let mut cleanup = Vec::with_capacity(self.keys.len() + self.buttons.len());

        for button in self.buttons.iter().rev() {
            cleanup.push(mouse_button_input(*button, false));
        }

        for key in self.keys.iter().rev() {
            cleanup.push(key_input(key, false));
        }

        cleanup
    }
}

/// Replays a validated recording using `SendInput` and a speed-aware scheduler.
pub fn play_recording(
    recording: &Recording,
    speed: SpeedMultiplier,
    stop_requested: &AtomicBool,
) -> Result<PlaybackReport, WindowsBackendError> {
    let started = Instant::now();
    let mut dispatched_events = 0_usize;
    let mut pressed_state = PressedState::default();
    let display = &recording.metadata().display;

    for event in recording.events() {
        wait_until(
            started,
            scheduled_elapsed_time(event.elapsed_time.as_micros(), speed),
        );

        if stop_requested.load(Ordering::SeqCst) {
            cleanup_pressed_inputs(&pressed_state)?;
            return Ok(PlaybackReport {
                dispatched_events,
                interrupted: true,
                elapsed: started.elapsed(),
                speed,
            });
        }

        let input = input_for_action(&event.action, display);

        if let Err(error) = send_inputs(&[input]) {
            let cleanup_result = cleanup_pressed_inputs(&pressed_state);
            if cleanup_result.is_err() {
                return cleanup_result.map(|_| unreachable!());
            }

            return Err(error);
        }

        pressed_state.note_action(&event.action);
        dispatched_events += 1;
    }

    Ok(PlaybackReport {
        dispatched_events,
        interrupted: false,
        elapsed: started.elapsed(),
        speed,
    })
}

fn cleanup_pressed_inputs(pressed_state: &PressedState) -> Result<(), WindowsBackendError> {
    let cleanup_inputs = pressed_state.cleanup_inputs();
    if cleanup_inputs.is_empty() {
        return Ok(());
    }

    send_inputs(&cleanup_inputs)
}

fn scheduled_elapsed_time(elapsed_time_micros: u64, speed: SpeedMultiplier) -> Duration {
    let adjusted = (elapsed_time_micros as f64 / speed.get()).round();
    Duration::from_micros(adjusted.max(0.0) as u64)
}

fn wait_until(started: Instant, due_at: Duration) {
    let deadline = started + due_at;

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }

        let remaining = deadline.duration_since(now);
        if remaining > SPIN_THRESHOLD {
            thread::sleep(remaining - SPIN_THRESHOLD);
        } else {
            spin_loop();
        }
    }
}

fn send_inputs(inputs: &[INPUT]) -> Result<(), WindowsBackendError> {
    let requested = inputs.len() as u32;
    let submitted = unsafe { SendInput(requested, inputs.as_ptr(), size_of::<INPUT>() as i32) };

    if submitted == requested {
        Ok(())
    } else if submitted == 0 {
        match current_process_elevation_status() {
            Ok(ProcessElevationStatus::NotElevated) => {
                Err(WindowsBackendError::ElevationRequired {
                    operation: "playback",
                })
            }
            Ok(ProcessElevationStatus::Elevated) | Err(_) => {
                Err(WindowsBackendError::last_os_error("SendInput"))
            }
        }
    } else {
        Err(WindowsBackendError::PartialSend {
            requested,
            submitted,
        })
    }
}

fn input_for_action(action: &InputAction, display: &DisplayMetadata) -> INPUT {
    match action {
        InputAction::KeyPressed { key } => key_input(key, true),
        InputAction::KeyReleased { key } => key_input(key, false),
        InputAction::PointerMoved { position } => mouse_move_input(position.absolute, display),
        InputAction::MouseButtonPressed { button } => mouse_button_input(*button, true),
        InputAction::MouseButtonReleased { button } => mouse_button_input(*button, false),
        InputAction::MouseWheelScrolled { axis, delta } => mouse_wheel_input(*axis, *delta),
    }
}

fn key_input(key: &KeyDescriptor, pressed: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if key.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !pressed {
        flags |= KEYEVENTF_KEYUP;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: key.scan_code.get(),
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_move_input(position: AbsoluteScreenPoint, display: &DisplayMetadata) -> INPUT {
    let (dx, dy) = to_absolute_mouse_coordinates(position, display);

    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_button_input(button: MouseButton, pressed: bool) -> INPUT {
    let (dw_flags, mouse_data) = match (button, pressed) {
        (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
        (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
        (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
        (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
        (MouseButton::X1, true) => (MOUSEEVENTF_XDOWN, XBUTTON1_DATA),
        (MouseButton::X1, false) => (MOUSEEVENTF_XUP, XBUTTON1_DATA),
        (MouseButton::X2, true) => (MOUSEEVENTF_XDOWN, XBUTTON2_DATA),
        (MouseButton::X2, false) => (MOUSEEVENTF_XUP, XBUTTON2_DATA),
    };

    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data,
                dwFlags: dw_flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_wheel_input(axis: regxorder_core::ScrollAxis, delta: i32) -> INPUT {
    let dw_flags = match axis {
        regxorder_core::ScrollAxis::Vertical => MOUSEEVENTF_WHEEL,
        regxorder_core::ScrollAxis::Horizontal => MOUSEEVENTF_HWHEEL,
    };

    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: delta as u32,
                dwFlags: dw_flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn to_absolute_mouse_coordinates(
    position: AbsoluteScreenPoint,
    display: &DisplayMetadata,
) -> (i32, i32) {
    fn scale_axis(value: i32, origin: i32, span: u32) -> i32 {
        let relative = i64::from(value) - i64::from(origin);
        let denominator = i64::from(span.saturating_sub(1).max(1));
        let scaled = (relative * 65_535) / denominator;
        scaled.clamp(0, 65_535) as i32
    }

    (
        scale_axis(
            position.x,
            display.virtual_origin.x,
            display.virtual_size.width,
        ),
        scale_axis(
            position.y,
            display.virtual_origin.y,
            display.virtual_size.height,
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::AtomicBool, time::Duration};

    use regxorder_core::{
        AbsoluteScreenPoint, DisplayMetadata, InputAction, InputEvent, KeyDescriptor, Recording,
        RecordingMetadata, SchemaVersion, ScreenSize,
    };

    use super::{
        PlaybackReport, PressedState, key_input, scheduled_elapsed_time,
        to_absolute_mouse_coordinates,
    };

    fn sample_display() -> DisplayMetadata {
        DisplayMetadata {
            virtual_origin: AbsoluteScreenPoint { x: -1920, y: 0 },
            virtual_size: ScreenSize {
                width: 3840,
                height: 1080,
            },
            monitors: Vec::new(),
        }
    }

    #[test]
    fn scheduled_elapsed_time_scales_with_speed_multiplier() {
        let fast = scheduled_elapsed_time(
            50_000,
            regxorder_core::SpeedMultiplier::new(2.0).expect("speed is valid"),
        );
        let slow = scheduled_elapsed_time(
            50_000,
            regxorder_core::SpeedMultiplier::new(0.5).expect("speed is valid"),
        );

        assert_eq!(fast, Duration::from_micros(25_000));
        assert_eq!(slow, Duration::from_micros(100_000));
    }

    #[test]
    fn absolute_mouse_coordinates_respect_virtual_display_origin() {
        let display = sample_display();
        let point = AbsoluteScreenPoint { x: 0, y: 540 };

        let (dx, dy) = to_absolute_mouse_coordinates(point, &display);

        assert!(dx > 32_000 && dx < 33_600, "unexpected dx: {dx}");
        assert!(dy > 32_700 && dy < 32_900, "unexpected dy: {dy}");
    }

    #[test]
    fn pressed_state_releases_tracked_inputs_in_reverse_order() {
        let key = KeyDescriptor {
            scan_code: regxorder_core::ScanCode::new(30),
            logical_name: Some(String::from("A")),
            extended: false,
        };

        let mut state = PressedState::default();
        state.note_action(&InputAction::MouseButtonPressed {
            button: regxorder_core::MouseButton::Left,
        });
        state.note_action(&InputAction::KeyPressed { key: key.clone() });

        let cleanup = state.cleanup_inputs();
        assert_eq!(cleanup.len(), 2);

        let keyup = key_input(&key, false);
        unsafe {
            assert_eq!(cleanup[0].Anonymous.mi.dwFlags, super::MOUSEEVENTF_LEFTUP);
            assert_eq!(cleanup[1].Anonymous.ki.dwFlags, keyup.Anonymous.ki.dwFlags);
        }
    }

    #[test]
    fn playback_report_can_be_constructed_for_completed_runs() {
        let report = PlaybackReport {
            dispatched_events: 2,
            interrupted: false,
            elapsed: Duration::from_millis(25),
            speed: regxorder_core::SpeedMultiplier::new(1.0).expect("speed is valid"),
        };

        assert_eq!(report.dispatched_events, 2);
        assert!(!report.interrupted);
    }

    #[test]
    fn empty_recordings_complete_without_interruption() {
        let recording = Recording::new(
            RecordingMetadata {
                schema_version: SchemaVersion::new(1),
                title: Some(String::from("empty")),
                display: sample_display(),
            },
            Vec::<InputEvent>::new(),
        )
        .expect("empty recording is valid");

        let stop = AtomicBool::new(false);
        let report = super::play_recording(
            &recording,
            regxorder_core::SpeedMultiplier::new(1.0).expect("speed is valid"),
            &stop,
        )
        .expect("empty playback should succeed");

        assert_eq!(report.dispatched_events, 0);
        assert!(!report.interrupted);
    }
}
