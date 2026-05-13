//! Slint-based desktop user interface for regxorder.

slint::include_modules!();

use regxorder_core::CURRENT_SCHEMA_VERSION;
use slint::{ComponentHandle, SharedString};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopShellState {
    status_text: SharedString,
    library_summary: SharedString,
    selected_recording: SharedString,
    playback_summary: SharedString,
    diagnostics_summary: SharedString,
}

impl DesktopShellState {
    fn initial() -> Self {
        let schema_version = CURRENT_SCHEMA_VERSION.get();

        Self {
            status_text: SharedString::from(
                "Desktop shell ready. Recording, playback, and environment checks still route through the SDK and CLI while the UI surface is being wired.",
            ),
            library_summary: SharedString::from(
                "No recording library is loaded yet. The next slice will enumerate saved sessions and recent recordings from disk.",
            ),
            selected_recording: SharedString::from("Selected recording: none"),
            playback_summary: SharedString::from(format!(
                "Playback stays library-first. This shell will call the same prepared playback services used by the CLI. Current schema: v{schema_version}."
            )),
            diagnostics_summary: SharedString::from(
                "Environment diagnostics remain CLI-backed in this slice. Use the refresh action below to restate the current workflow boundary inside the desktop shell.",
            ),
        }
    }

    fn sample_overview() -> Self {
        Self {
            status_text: SharedString::from(
                "Loaded the desktop shell sample state. The next slice will swap this placeholder data for real session discovery.",
            ),
            library_summary: SharedString::from(
                "Sample library view loaded. Expect the production version to group saved recordings under the shared sessions directory and surface their metadata here.",
            ),
            selected_recording: SharedString::from(
                "Selected recording: demo-session.json (42 events, 2.150 ms)",
            ),
            playback_summary: SharedString::from(
                "Playback controls remain placeholders in this slice, but they are positioned to drive the same speed, start-hotkey, and stop-hotkey flows as the CLI.",
            ),
            diagnostics_summary: SharedString::from(
                "The diagnostics pane will eventually summarize the same environment, recording, and playback checks exposed by the CLI doctor and diagnostics commands.",
            ),
        }
    }

    fn diagnostics_refresh_note() -> Self {
        Self {
            status_text: SharedString::from(
                "Diagnostics remain CLI-backed for now. The UI shell is warning about the current integration boundary rather than pretending to execute live checks.",
            ),
            library_summary: SharedString::from(
                "Use the sample action to preview how a selected recording will surface inside the desktop shell.",
            ),
            selected_recording: SharedString::from("Selected recording: none"),
            playback_summary: SharedString::from(
                "Playback wiring is intentionally deferred until the UI can call the shared SDK flows without duplicating CLI-only orchestration.",
            ),
            diagnostics_summary: SharedString::from(
                "Run `regxorder diagnostics environment` or `regxorder doctor environment` from the CLI for live checks today. The UI pane will adopt those same reports in a later slice.",
            ),
        }
    }

    fn apply_to(&self, window: &AppWindow) {
        window.set_status_text(self.status_text.clone());
        window.set_library_summary(self.library_summary.clone());
        window.set_selected_recording(self.selected_recording.clone());
        window.set_playback_summary(self.playback_summary.clone());
        window.set_diagnostics_summary(self.diagnostics_summary.clone());
    }
}

/// Runs the desktop shell for regxorder.
pub fn run_desktop_ui() -> Result<(), slint::PlatformError> {
    let app_window = AppWindow::new()?;
    DesktopShellState::initial().apply_to(&app_window);

    let app_window_weak = app_window.as_weak();
    app_window.on_load_sample_overview_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        DesktopShellState::sample_overview().apply_to(&app_window);
    });

    let app_window_weak = app_window.as_weak();
    app_window.on_refresh_diagnostics_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        DesktopShellState::diagnostics_refresh_note().apply_to(&app_window);
    });

    app_window.run()
}

#[cfg(test)]
mod tests {
    use super::DesktopShellState;

    #[test]
    fn initial_shell_state_mentions_the_cli_boundary() {
        let state = DesktopShellState::initial();

        assert!(state.status_text.to_string().contains("CLI"));
        assert!(state.playback_summary.to_string().contains("schema: v1"));
    }

    #[test]
    fn sample_shell_state_selects_a_demo_recording() {
        let state = DesktopShellState::sample_overview();

        assert_eq!(
            state.selected_recording.to_string(),
            "Selected recording: demo-session.json (42 events, 2.150 ms)"
        );
        assert!(
            state
                .library_summary
                .to_string()
                .contains("Sample library view loaded")
        );
    }
}
