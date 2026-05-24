//! Slint-based desktop user interface for regxorder.

slint::include_modules!();

mod event_list;
mod recording_presentation;

use std::{
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use regxorder_core::{
    DiagnosticStatus, ElapsedTime, EnvironmentDoctorReport, HotkeyBinding, HotkeyKey,
    HotkeyModifier, InputAction, InputEvent, Recording, RecordingMetadata, RecordingMetrics,
    SpeedMultiplier,
};
use regxorder_win32::{ControlController, RecordingStrategy, diagnose_windows_environment};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::event_list::{
    EventListFilter, build_event_duration_items, build_event_filter_counts,
    build_event_index_items, build_event_parameter_items, build_event_type_items,
    build_visible_event_rows, visible_event_source_indices,
};
use crate::recording_presentation::{
    build_recording_title_items, build_selected_recording_sections, display_title_for_recording,
};

const SESSION_DIRECTORY_NAME: &str = "sessions";
const DEFAULT_PLAYBACK_LOOP_COUNT: u16 = 1;
const MIN_PLAYBACK_LOOP_COUNT: u16 = 1;
const MAX_PLAYBACK_LOOP_COUNT: u16 = 9_999;
const DEFAULT_PLAYBACK_SPEED_VALUE: f64 = 1.0;
const MIN_PLAYBACK_SPEED_VALUE: f64 = 0.1;
const MAX_PLAYBACK_SPEED_VALUE: f64 = 10.0;
const HOTKEY_KEY_OPTIONS: [HotkeyKey; 13] = [
    HotkeyKey::Escape,
    HotkeyKey::F1,
    HotkeyKey::F2,
    HotkeyKey::F3,
    HotkeyKey::F4,
    HotkeyKey::F5,
    HotkeyKey::F6,
    HotkeyKey::F7,
    HotkeyKey::F8,
    HotkeyKey::F9,
    HotkeyKey::F10,
    HotkeyKey::F11,
    HotkeyKey::F12,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveView {
    Editor,
    Diagnostics,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppConfiguration {
    recording_strategy: RecordingStrategy,
    stop_action_hotkey: HotkeyBinding,
}

impl Default for AppConfiguration {
    fn default() -> Self {
        Self {
            recording_strategy: RecordingStrategy::RawInput,
            stop_action_hotkey: HotkeyBinding::new(
                vec![HotkeyModifier::Control, HotkeyModifier::Shift],
                HotkeyKey::F12,
            )
            .expect("default stop hotkey should be valid"),
        }
    }
}

#[derive(Debug, Clone)]
struct DesktopShellState {
    show_editor_view: bool,
    show_diagnostics_view: bool,
    show_settings_view: bool,
    editor_tab_selected: bool,
    diagnostics_tab_selected: bool,
    settings_tab_selected: bool,
    session_directory: SharedString,
    has_recordings: bool,
    recording_titles: ModelRc<SharedString>,
    selected_recording_index: i32,
    selected_recording_title: SharedString,
    selected_recording_details: SharedString,
    playback_controls_summary: SharedString,
    playback_summary: SharedString,
    playback_checks: SharedString,
    playback_loop_input_text: SharedString,
    playback_speed_input_text: SharedString,
    event_index_labels: ModelRc<SharedString>,
    event_type_labels: ModelRc<SharedString>,
    event_parameter_labels: ModelRc<SharedString>,
    event_duration_labels: ModelRc<SharedString>,
    filter_key_events_selected: bool,
    filter_mouse_button_events_selected: bool,
    filter_mouse_wheel_events_selected: bool,
    filter_pointer_move_events_selected: bool,
    filter_key_event_count_label: SharedString,
    filter_mouse_button_event_count_label: SharedString,
    filter_mouse_wheel_event_count_label: SharedString,
    filter_pointer_move_event_count_label: SharedString,
    stop_hotkey_summary: SharedString,
    stop_hotkey_control_selected: bool,
    stop_hotkey_alt_selected: bool,
    stop_hotkey_shift_selected: bool,
    stop_hotkey_win_selected: bool,
    stop_hotkey_key_index: i32,
    stop_hotkey_key_label: SharedString,
    can_start_recording: bool,
    can_play_selected_recording: bool,
    can_stop_active_action: bool,
    can_save_editor_changes: bool,
    can_revert_editor_changes: bool,
    can_delete_selected_recording: bool,
    recording_strategy_raw_selected: bool,
    recording_strategy_low_level_selected: bool,
    diagnostics_summary: SharedString,
    diagnostics_checks: SharedString,
    settings_summary: SharedString,
}

#[derive(Debug, Clone)]
struct DesktopShellModel {
    active_view: ActiveView,
    recording_library: RecordingLibraryState,
    editing_session: Option<EditingSession>,
    configuration: AppConfiguration,
    event_list_filter: EventListFilter,
    environment_report: EnvironmentDoctorReport,
    selected_playback_loop_count: u16,
    selected_playback_speed: SpeedMultiplier,
    playback_loop_input_text: String,
    playback_speed_input_text: String,
    playback_speed_validation_message: Option<String>,
    action_execution_state: ActionExecutionState,
    current_action_stop_requested: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, PartialEq)]
enum ActionExecutionState {
    Idle,
    Notice(String),
    RecordingRunning {
        recording_title: String,
        strategy: RecordingStrategy,
    },
    RecordingCompleted {
        recording_title: String,
        output_path: PathBuf,
        event_count: usize,
        duration_micros: u64,
    },
    RecordingFailed {
        recording_title: String,
        reason: String,
    },
    PlaybackRunning {
        recording_title: String,
        speed: SpeedMultiplier,
    },
    PlaybackCompleted {
        recording_title: String,
        speed: SpeedMultiplier,
        dispatched_events: usize,
        interrupted: bool,
        elapsed_millis: f64,
    },
    PlaybackFailed {
        recording_title: String,
        speed: SpeedMultiplier,
        reason: String,
    },
}

#[derive(Debug, Clone)]
struct RecordingRequest {
    output_path: PathBuf,
    recording_title: String,
    strategy: RecordingStrategy,
    stop_hotkey: Option<HotkeyBinding>,
    stop_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
struct PlaybackRequest {
    recording: Recording,
    recording_title: String,
    loop_count: u16,
    speed: SpeedMultiplier,
    stop_hotkey: Option<HotkeyBinding>,
    stop_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq)]
struct EditingSession {
    source_path: PathBuf,
    original_recording: Recording,
    working_recording: Recording,
    selected_event_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct RecordingLibraryState {
    session_directory: PathBuf,
    recordings: Vec<RecordingLibraryEntry>,
    invalid_recordings: Vec<InvalidRecordingEntry>,
    selected_recording_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct RecordingLibraryEntry {
    path: PathBuf,
    recording: Recording,
    metrics: RecordingMetrics,
    file_size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InvalidRecordingEntry {
    path: PathBuf,
    reason: String,
}

impl DesktopShellState {
    fn from_model(model: &DesktopShellModel) -> Self {
        let recording_count = model.recording_library.recordings.len();
        let selected_loop_count = model.selected_playback_loop_count;
        let selected_speed = model.selected_playback_speed.get();
        let action_running = model.is_action_running();
        let editor_is_dirty = model.editor_has_unsaved_changes();
        let (
            selected_recording_title,
            selected_recording_details,
            playback_summary,
            playback_checks,
        ) = build_selected_recording_sections(model.editing_session.as_ref());
        let visible_event_rows =
            build_visible_event_rows(model.editing_session.as_ref(), model.event_list_filter);
        let event_filter_counts = build_event_filter_counts(model.editing_session.as_ref());
        let diagnostics_checks = build_diagnostic_checks_text(
            &model.environment_report,
            &playback_summary,
            &playback_checks,
        );
        let stop_hotkey = &model.configuration.stop_action_hotkey;

        Self {
            show_editor_view: model.active_view == ActiveView::Editor,
            show_diagnostics_view: model.active_view == ActiveView::Diagnostics,
            show_settings_view: model.active_view == ActiveView::Settings,
            editor_tab_selected: model.active_view == ActiveView::Editor,
            diagnostics_tab_selected: model.active_view == ActiveView::Diagnostics,
            settings_tab_selected: model.active_view == ActiveView::Settings,
            session_directory: SharedString::from(format!(
                "Sessions directory: {}",
                model.recording_library.session_directory.display()
            )),
            has_recordings: recording_count > 0,
            recording_titles: Rc::new(VecModel::from(build_recording_title_items(
                &model.recording_library,
            )))
            .into(),
            selected_recording_index: model
                .recording_library
                .selected_recording_index
                .map(|index| index as i32)
                .unwrap_or(-1),
            selected_recording_title: SharedString::from(selected_recording_title),
            selected_recording_details: SharedString::from(selected_recording_details),
            playback_controls_summary: SharedString::from(format!(
                "Record {} · Speed {}x · Loop {} · {}",
                model.configuration.recording_strategy,
                selected_speed,
                selected_loop_count,
                compact_process_elevation_status(&model.environment_report)
            )),
            playback_summary: SharedString::from(playback_summary),
            playback_checks: SharedString::from(playback_checks),
            playback_loop_input_text: SharedString::from(model.playback_loop_input_text.clone()),
            playback_speed_input_text: SharedString::from(model.playback_speed_input_text.clone()),
            event_index_labels: Rc::new(VecModel::from(build_event_index_items(
                model.editing_session.as_ref(),
                &visible_event_rows,
            )))
            .into(),
            event_type_labels: Rc::new(VecModel::from(build_event_type_items(
                model.editing_session.as_ref(),
                &visible_event_rows,
            )))
            .into(),
            event_parameter_labels: Rc::new(VecModel::from(build_event_parameter_items(
                model.editing_session.as_ref(),
                &visible_event_rows,
            )))
            .into(),
            event_duration_labels: Rc::new(VecModel::from(build_event_duration_items(
                model.editing_session.as_ref(),
                &visible_event_rows,
            )))
            .into(),
            filter_key_events_selected: model.event_list_filter.show_key_events,
            filter_mouse_button_events_selected: model.event_list_filter.show_mouse_button_events,
            filter_mouse_wheel_events_selected: model.event_list_filter.show_mouse_wheel_events,
            filter_pointer_move_events_selected: model.event_list_filter.show_pointer_move_events,
            filter_key_event_count_label: SharedString::from(
                event_filter_counts.key_event_count.to_string(),
            ),
            filter_mouse_button_event_count_label: SharedString::from(
                event_filter_counts.mouse_button_event_count.to_string(),
            ),
            filter_mouse_wheel_event_count_label: SharedString::from(
                event_filter_counts.mouse_wheel_event_count.to_string(),
            ),
            filter_pointer_move_event_count_label: SharedString::from(
                event_filter_counts.pointer_move_event_count.to_string(),
            ),
            stop_hotkey_summary: SharedString::from(format!(
                "Stop recording or playback with {} using {}.",
                stop_hotkey, model.environment_report.hotkey_backend,
            )),
            stop_hotkey_control_selected: stop_hotkey
                .modifiers()
                .contains(&HotkeyModifier::Control),
            stop_hotkey_alt_selected: stop_hotkey.modifiers().contains(&HotkeyModifier::Alt),
            stop_hotkey_shift_selected: stop_hotkey.modifiers().contains(&HotkeyModifier::Shift),
            stop_hotkey_win_selected: stop_hotkey.modifiers().contains(&HotkeyModifier::Win),
            stop_hotkey_key_index: hotkey_key_index(stop_hotkey.key()),
            stop_hotkey_key_label: SharedString::from(stop_hotkey.key().display_name()),
            can_start_recording: !action_running && !editor_is_dirty,
            can_play_selected_recording: model.editable_recording().is_some()
                && !action_running
                && !editor_is_dirty,
            can_stop_active_action: action_running,
            can_save_editor_changes: editor_is_dirty && !action_running,
            can_revert_editor_changes: editor_is_dirty && !action_running,
            can_delete_selected_recording: model.recording_library.selected_recording().is_some()
                && !action_running
                && !editor_is_dirty,
            recording_strategy_raw_selected: model.configuration.recording_strategy
                == RecordingStrategy::RawInput,
            recording_strategy_low_level_selected: model.configuration.recording_strategy
                == RecordingStrategy::LowLevelHooks,
            diagnostics_summary: SharedString::from(build_diagnostics_summary(
                &model.environment_report,
                model.editing_session.as_ref(),
            )),
            diagnostics_checks: SharedString::from(diagnostics_checks),
            settings_summary: SharedString::from(build_settings_summary(model)),
        }
    }

    fn apply_to(&self, window: &AppWindow) {
        window.set_show_editor_view(self.show_editor_view);
        window.set_show_diagnostics_view(self.show_diagnostics_view);
        window.set_show_settings_view(self.show_settings_view);
        window.set_editor_tab_selected(self.editor_tab_selected);
        window.set_diagnostics_tab_selected(self.diagnostics_tab_selected);
        window.set_settings_tab_selected(self.settings_tab_selected);
        window.set_session_directory(self.session_directory.clone());
        window.set_has_recordings(self.has_recordings);
        window.set_recording_titles(self.recording_titles.clone());
        window.set_selected_recording_index(self.selected_recording_index);
        window.set_selected_recording_title(self.selected_recording_title.clone());
        window.set_selected_recording_details(self.selected_recording_details.clone());
        window.set_playback_controls_summary(self.playback_controls_summary.clone());
        window.set_playback_summary(self.playback_summary.clone());
        window.set_playback_checks(self.playback_checks.clone());
        window.set_playback_loop_input_text(self.playback_loop_input_text.clone());
        window.set_playback_speed_input_text(self.playback_speed_input_text.clone());
        window.set_event_index_labels(self.event_index_labels.clone());
        window.set_event_type_labels(self.event_type_labels.clone());
        window.set_event_parameter_labels(self.event_parameter_labels.clone());
        window.set_event_duration_labels(self.event_duration_labels.clone());
        window.set_filter_key_events_selected(self.filter_key_events_selected);
        window.set_filter_mouse_button_events_selected(self.filter_mouse_button_events_selected);
        window.set_filter_mouse_wheel_events_selected(self.filter_mouse_wheel_events_selected);
        window.set_filter_pointer_move_events_selected(self.filter_pointer_move_events_selected);
        window.set_filter_key_event_count_label(self.filter_key_event_count_label.clone());
        window.set_filter_mouse_button_event_count_label(
            self.filter_mouse_button_event_count_label.clone(),
        );
        window.set_filter_mouse_wheel_event_count_label(
            self.filter_mouse_wheel_event_count_label.clone(),
        );
        window.set_filter_pointer_move_event_count_label(
            self.filter_pointer_move_event_count_label.clone(),
        );
        window.set_stop_hotkey_summary(self.stop_hotkey_summary.clone());
        window.set_stop_hotkey_control_selected(self.stop_hotkey_control_selected);
        window.set_stop_hotkey_alt_selected(self.stop_hotkey_alt_selected);
        window.set_stop_hotkey_shift_selected(self.stop_hotkey_shift_selected);
        window.set_stop_hotkey_win_selected(self.stop_hotkey_win_selected);
        window.set_stop_hotkey_key_index(self.stop_hotkey_key_index);
        window.set_stop_hotkey_key_label(self.stop_hotkey_key_label.clone());
        window.set_can_start_recording(self.can_start_recording);
        window.set_can_play_selected_recording(self.can_play_selected_recording);
        window.set_can_stop_active_action(self.can_stop_active_action);
        window.set_can_save_editor_changes(self.can_save_editor_changes);
        window.set_can_revert_editor_changes(self.can_revert_editor_changes);
        window.set_can_delete_selected_recording(self.can_delete_selected_recording);
        window.set_recording_strategy_raw_selected(self.recording_strategy_raw_selected);
        window
            .set_recording_strategy_low_level_selected(self.recording_strategy_low_level_selected);
        window.set_diagnostics_summary(self.diagnostics_summary.clone());
        window.set_diagnostics_checks(self.diagnostics_checks.clone());
        window.set_settings_summary(self.settings_summary.clone());
    }
}

impl DesktopShellModel {
    fn load() -> Self {
        let recording_library = RecordingLibraryState::discover(None);

        let mut model = Self {
            active_view: ActiveView::Editor,
            editing_session: recording_library
                .selected_recording()
                .map(EditingSession::from_library_entry),
            configuration: AppConfiguration::default(),
            event_list_filter: EventListFilter::default(),
            recording_library,
            environment_report: load_environment_report(),
            selected_playback_loop_count: DEFAULT_PLAYBACK_LOOP_COUNT,
            selected_playback_speed: default_playback_speed(),
            playback_loop_input_text: format_playback_loop_input(DEFAULT_PLAYBACK_LOOP_COUNT),
            playback_speed_input_text: format_playback_speed_input(default_playback_speed()),
            playback_speed_validation_message: None,
            action_execution_state: ActionExecutionState::Idle,
            current_action_stop_requested: None,
        };
        model.sync_event_selection_to_filter();
        model
    }

    fn refresh_library(&mut self) {
        let selected_recording_path = self.current_selected_path();
        self.recording_library =
            RecordingLibraryState::discover(selected_recording_path.as_deref());

        if !self.editor_has_unsaved_changes() {
            self.sync_editor_with_selected_recording();
        }
    }

    fn select_recording(&mut self, index: usize) {
        if Some(index) == self.recording_library.selected_recording_index {
            return;
        }

        if self.editor_has_unsaved_changes() {
            self.note_action_notice(
                "Save or revert the current editor changes before switching sessions.",
            );
            return;
        }

        self.recording_library.select(index);
        self.sync_editor_with_selected_recording();
    }

    fn delete_selected_recording(&mut self) -> Result<(), String> {
        if self.is_action_running() {
            return Err(String::from(
                "Stop the active action before deleting a session.",
            ));
        }

        if self.editor_has_unsaved_changes() {
            return Err(String::from(
                "Save or revert the current editor changes before deleting a session.",
            ));
        }

        let Some(selected_recording) = self.recording_library.selected_recording() else {
            return Err(String::from("Select a session before deleting it."));
        };

        fs::remove_file(&selected_recording.path).map_err(|error| {
            format!(
                "Failed to delete {}: {error}",
                selected_recording.path.display()
            )
        })?;

        self.recording_library = RecordingLibraryState::discover(None);
        self.sync_editor_with_selected_recording();
        self.active_view = ActiveView::Editor;
        Ok(())
    }

    fn select_event(&mut self, index: usize) {
        let Some(source_event_index) =
            visible_event_source_indices(self.editing_session.as_ref(), self.event_list_filter)
                .get(index)
                .copied()
        else {
            return;
        };

        if let Some(editing_session) = &mut self.editing_session {
            editing_session.select_event(source_event_index);
        }
    }

    fn set_active_view(&mut self, active_view: ActiveView) {
        self.active_view = active_view;
    }

    fn set_playback_speed(&mut self, speed: SpeedMultiplier) {
        self.selected_playback_speed = speed;
        self.playback_speed_input_text = format_playback_speed_input(speed);
        self.playback_speed_validation_message = None;
    }

    fn set_playback_loop_count(&mut self, loop_count: u16) {
        self.selected_playback_loop_count =
            loop_count.clamp(MIN_PLAYBACK_LOOP_COUNT, MAX_PLAYBACK_LOOP_COUNT);
        self.playback_loop_input_text =
            format_playback_loop_input(self.selected_playback_loop_count);
    }

    fn set_playback_loop_input_text(&mut self, input: String) {
        if input.trim().is_empty() {
            self.playback_loop_input_text =
                format_playback_loop_input(self.selected_playback_loop_count);
            return;
        }

        self.playback_loop_input_text = input;

        if let Some(loop_count) = clamp_playback_loop_input(&self.playback_loop_input_text) {
            self.selected_playback_loop_count = loop_count;
        }
    }

    fn commit_playback_loop_input_text(&mut self) {
        let committed_loop_count = clamp_playback_loop_input(&self.playback_loop_input_text)
            .unwrap_or(self.selected_playback_loop_count);
        self.set_playback_loop_count(committed_loop_count);
    }

    fn set_playback_speed_input_text(&mut self, input: String) {
        if input.trim().is_empty() {
            self.playback_speed_input_text =
                format_playback_speed_input(self.selected_playback_speed);
            self.playback_speed_validation_message = None;
            return;
        }

        self.playback_speed_input_text = input;

        if let Some(speed) = clamp_playback_speed_input(&self.playback_speed_input_text) {
            self.selected_playback_speed = speed;
        }

        self.playback_speed_validation_message = None;
    }

    fn commit_playback_speed_input_text(&mut self) {
        let committed_speed = clamp_playback_speed_input(&self.playback_speed_input_text)
            .unwrap_or(self.selected_playback_speed);
        self.selected_playback_speed = committed_speed;
        self.playback_speed_input_text = format_playback_speed_input(committed_speed);
        self.playback_speed_validation_message = None;
    }

    fn set_show_key_events(&mut self, show_key_events: bool) {
        self.event_list_filter.show_key_events = show_key_events;
        self.sync_event_selection_to_filter();
    }

    fn set_show_mouse_button_events(&mut self, show_mouse_button_events: bool) {
        self.event_list_filter.show_mouse_button_events = show_mouse_button_events;
        self.sync_event_selection_to_filter();
    }

    fn set_show_mouse_wheel_events(&mut self, show_mouse_wheel_events: bool) {
        self.event_list_filter.show_mouse_wheel_events = show_mouse_wheel_events;
        self.sync_event_selection_to_filter();
    }

    fn set_show_pointer_move_events(&mut self, show_pointer_move_events: bool) {
        self.event_list_filter.show_pointer_move_events = show_pointer_move_events;
        self.sync_event_selection_to_filter();
    }

    fn set_stop_hotkey_modifier(
        &mut self,
        modifier: HotkeyModifier,
        enabled: bool,
    ) -> Result<(), String> {
        let mut modifiers = self.configuration.stop_action_hotkey.modifiers().to_vec();

        if enabled {
            if !modifiers.contains(&modifier) {
                modifiers.push(modifier);
            }
        } else {
            modifiers.retain(|candidate| *candidate != modifier);
        }

        self.configuration.stop_action_hotkey =
            HotkeyBinding::new(modifiers, self.configuration.stop_action_hotkey.key())
                .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn set_stop_hotkey_key(&mut self, key: HotkeyKey) {
        self.configuration.stop_action_hotkey = HotkeyBinding::new(
            self.configuration.stop_action_hotkey.modifiers().to_vec(),
            key,
        )
        .expect("configured stop hotkey modifiers should stay valid");
    }

    fn set_recording_strategy(&mut self, recording_strategy: RecordingStrategy) {
        self.configuration.recording_strategy = recording_strategy;
    }

    fn refresh_diagnostics(&mut self) {
        self.environment_report = load_environment_report();
    }

    fn editable_recording(&self) -> Option<&Recording> {
        self.editing_session
            .as_ref()
            .map(|editing_session| &editing_session.working_recording)
            .or_else(|| {
                self.recording_library
                    .selected_recording()
                    .map(|recording| &recording.recording)
            })
    }

    fn current_selected_path(&self) -> Option<PathBuf> {
        self.editing_session
            .as_ref()
            .map(|editing_session| editing_session.source_path.clone())
            .or_else(|| {
                self.recording_library
                    .selected_recording()
                    .map(|recording| recording.path.clone())
            })
    }

    fn sync_editor_with_selected_recording(&mut self) {
        self.editing_session = self
            .recording_library
            .selected_recording()
            .map(EditingSession::from_library_entry);
        self.sync_event_selection_to_filter();
    }

    fn editor_has_unsaved_changes(&self) -> bool {
        self.editing_session
            .as_ref()
            .is_some_and(EditingSession::is_dirty)
    }

    fn current_recording_title(&self) -> Option<String> {
        self.editing_session
            .as_ref()
            .map(EditingSession::display_title)
            .or_else(|| {
                self.recording_library
                    .selected_recording()
                    .map(RecordingLibraryEntry::display_title)
            })
    }

    fn is_action_running(&self) -> bool {
        self.current_action_stop_requested.is_some()
    }

    fn note_action_notice(&mut self, message: impl Into<String>) {
        self.action_execution_state = ActionExecutionState::Notice(message.into());
    }

    fn begin_recording_request(&mut self) -> Result<RecordingRequest, String> {
        if self.is_action_running() {
            return Err(String::from(
                "Another action is already running. Stop it before starting a new recording.",
            ));
        }

        if self.editor_has_unsaved_changes() {
            return Err(String::from(
                "Save or revert the current editor changes before starting a new recording.",
            ));
        }

        let (recording_title, output_path) = next_recording_destination(
            &self.recording_library.session_directory,
            &self.recording_library,
        );
        let stop_requested = Arc::new(AtomicBool::new(false));
        let request = RecordingRequest {
            output_path,
            recording_title: recording_title.clone(),
            strategy: self.configuration.recording_strategy,
            stop_hotkey: Some(self.configuration.stop_action_hotkey.clone()),
            stop_requested: Arc::clone(&stop_requested),
        };

        self.current_action_stop_requested = Some(stop_requested);
        self.action_execution_state = ActionExecutionState::RecordingRunning {
            recording_title,
            strategy: self.configuration.recording_strategy,
        };

        Ok(request)
    }

    fn begin_playback_request(&mut self) -> Result<PlaybackRequest, String> {
        if self.is_action_running() {
            return Err(String::from(
                "Another action is already running. Stop it before starting playback.",
            ));
        }

        self.commit_playback_loop_input_text();
        self.commit_playback_speed_input_text();

        let Some(selected_recording) = self.editable_recording().cloned() else {
            return Err(String::from(
                "Select a validated recording before starting playback.",
            ));
        };

        let stop_requested = Arc::new(AtomicBool::new(false));

        let request = PlaybackRequest {
            recording: selected_recording,
            recording_title: self
                .current_recording_title()
                .unwrap_or_else(|| String::from("Selected session")),
            loop_count: self.selected_playback_loop_count,
            speed: self.selected_playback_speed,
            stop_hotkey: Some(self.configuration.stop_action_hotkey.clone()),
            stop_requested: Arc::clone(&stop_requested),
        };

        self.current_action_stop_requested = Some(stop_requested);
        self.action_execution_state = ActionExecutionState::PlaybackRunning {
            recording_title: request.recording_title.clone(),
            speed: request.speed,
        };

        Ok(request)
    }

    fn request_stop_active_action(&mut self) {
        if let Some(stop_requested) = &self.current_action_stop_requested {
            stop_requested.store(true, Ordering::SeqCst);
        }
    }

    fn save_editor_changes(&mut self) -> Result<(), String> {
        let Some(editing_session) = &self.editing_session else {
            return Err(String::from("No session is loaded into the editor."));
        };

        if !editing_session.is_dirty() {
            return Ok(());
        }

        let selected_event_index = editing_session.selected_event_index;
        let source_path = editing_session.source_path.clone();
        write_recording_file(&source_path, &editing_session.working_recording)?;
        self.recording_library = RecordingLibraryState::discover(Some(&source_path));
        self.sync_editor_with_selected_recording();

        if let Some(editing_session) = &mut self.editing_session {
            editing_session.select_event_clamped(selected_event_index);
        }
        self.sync_event_selection_to_filter();

        self.note_action_notice(format!("Saved changes to {}.", source_path.display()));
        Ok(())
    }

    fn revert_editor_changes(&mut self) {
        if let Some(editing_session) = &mut self.editing_session {
            editing_session.revert();
        }
        self.sync_event_selection_to_filter();
    }

    fn delete_selected_event(&mut self) -> Result<(), String> {
        if self.is_action_running() {
            return Err(String::from(
                "Stop the active action before editing the event list.",
            ));
        }

        let Some(editing_session) = &mut self.editing_session else {
            return Err(String::from("No session is loaded into the editor."));
        };

        editing_session.delete_selected_event()?;
        self.sync_event_selection_to_filter();
        Ok(())
    }

    fn delete_visible_event(&mut self, visible_index: usize) -> Result<(), String> {
        if self.is_action_running() {
            return Err(String::from(
                "Stop the active action before editing the event list.",
            ));
        }

        let source_event_index =
            visible_event_source_indices(self.editing_session.as_ref(), self.event_list_filter)
                .get(visible_index)
                .copied()
                .ok_or_else(|| String::from("Select a visible event before deleting it."))?;

        let Some(editing_session) = &self.editing_session else {
            return Err(String::from("No session is loaded into the editor."));
        };

        let deleted_event_label = editing_session
            .working_recording
            .events()
            .get(source_event_index)
            .map(|event| format_event_action_label(&event.action))
            .unwrap_or_else(|| String::from("event"));

        let Some(editing_session) = &mut self.editing_session else {
            return Err(String::from("No session is loaded into the editor."));
        };

        editing_session.delete_event_at(source_event_index)?;
        self.sync_event_selection_to_filter();
        self.note_action_notice(format!(
            "Removed {deleted_event_label} from the working copy."
        ));
        Ok(())
    }

    fn duplicate_selected_event(&mut self) -> Result<(), String> {
        if self.is_action_running() {
            return Err(String::from(
                "Stop the active action before editing the event list.",
            ));
        }

        let Some(editing_session) = &mut self.editing_session else {
            return Err(String::from("No session is loaded into the editor."));
        };

        editing_session.duplicate_selected_event()?;
        self.sync_event_selection_to_filter();
        Ok(())
    }

    fn toggle_selected_event_state(&mut self) -> Result<(), String> {
        if self.is_action_running() {
            return Err(String::from(
                "Stop the active action before editing the event list.",
            ));
        }

        let Some(editing_session) = &mut self.editing_session else {
            return Err(String::from("No session is loaded into the editor."));
        };

        editing_session.toggle_selected_event_state()?;
        self.sync_event_selection_to_filter();
        Ok(())
    }

    fn nudge_selected_event(&mut self, delta_micros: i64) -> Result<(), String> {
        if self.is_action_running() {
            return Err(String::from(
                "Stop the active action before editing the event list.",
            ));
        }

        let Some(editing_session) = &mut self.editing_session else {
            return Err(String::from("No session is loaded into the editor."));
        };

        editing_session.nudge_selected_event(delta_micros)
    }

    fn finish_action(&mut self, action_execution_state: ActionExecutionState) {
        self.current_action_stop_requested = None;
        self.action_execution_state = action_execution_state;

        if let ActionExecutionState::RecordingCompleted { output_path, .. } =
            &self.action_execution_state
        {
            self.recording_library = RecordingLibraryState::discover(Some(output_path));
            self.sync_editor_with_selected_recording();
            self.active_view = ActiveView::Editor;
        }
    }

    fn sync_event_selection_to_filter(&mut self) {
        let visible_source_indices =
            visible_event_source_indices(self.editing_session.as_ref(), self.event_list_filter);

        let Some(editing_session) = &mut self.editing_session else {
            return;
        };

        if visible_source_indices.is_empty() {
            editing_session.selected_event_index = None;
            return;
        }

        if editing_session
            .selected_event_index
            .is_some_and(|index| visible_source_indices.contains(&index))
        {
            return;
        }

        editing_session.select_event(visible_source_indices[0]);
    }
}

impl EditingSession {
    fn from_library_entry(entry: &RecordingLibraryEntry) -> Self {
        Self {
            source_path: entry.path.clone(),
            original_recording: entry.recording.clone(),
            working_recording: entry.recording.clone(),
            selected_event_index: (!entry.recording.events().is_empty()).then_some(0),
        }
    }

    fn display_title(&self) -> String {
        display_title_for_recording(&self.source_path, &self.working_recording)
    }

    fn is_dirty(&self) -> bool {
        self.original_recording != self.working_recording
    }

    fn select_event(&mut self, index: usize) {
        if index < self.working_recording.events().len() {
            self.selected_event_index = Some(index);
        }
    }

    fn select_event_clamped(&mut self, index: Option<usize>) {
        if self.working_recording.events().is_empty() {
            self.selected_event_index = None;
            return;
        }

        let clamped_index = index
            .unwrap_or(0)
            .min(self.working_recording.events().len() - 1);
        self.selected_event_index = Some(clamped_index);
    }

    fn delete_selected_event(&mut self) -> Result<(), String> {
        let Some(selected_event_index) = self.selected_event_index else {
            return Err(String::from("Select an event before deleting it."));
        };

        self.delete_event_at(selected_event_index)
    }

    fn delete_event_at(&mut self, event_index: usize) -> Result<(), String> {
        if event_index >= self.working_recording.events().len() {
            return Err(String::from("Select an event before deleting it."));
        }

        let metadata = self.working_recording.metadata().clone();
        let mut events = self.working_recording.events().to_vec();
        events.remove(event_index);
        self.working_recording = rebuild_recording(metadata, events)?;
        self.select_event_clamped(Some(event_index));
        Ok(())
    }

    fn duplicate_selected_event(&mut self) -> Result<(), String> {
        let Some(selected_event_index) = self.selected_event_index else {
            return Err(String::from("Select an event before duplicating it."));
        };

        let metadata = self.working_recording.metadata().clone();
        let mut events = self.working_recording.events().to_vec();
        let duplicated_event = events[selected_event_index].clone();
        events.insert(selected_event_index + 1, duplicated_event);
        self.working_recording = rebuild_recording(metadata, events)?;
        self.select_event_clamped(Some(selected_event_index + 1));
        Ok(())
    }

    fn toggle_selected_event_state(&mut self) -> Result<(), String> {
        let Some(selected_event_index) = self.selected_event_index else {
            return Err(String::from("Select an event before changing it."));
        };

        let metadata = self.working_recording.metadata().clone();
        let mut events = self.working_recording.events().to_vec();
        events[selected_event_index].action = match &events[selected_event_index].action {
            InputAction::KeyPressed { key } => InputAction::KeyReleased { key: key.clone() },
            InputAction::KeyReleased { key } => InputAction::KeyPressed { key: key.clone() },
            InputAction::MouseButtonPressed { button } => {
                InputAction::MouseButtonReleased { button: *button }
            }
            InputAction::MouseButtonReleased { button } => {
                InputAction::MouseButtonPressed { button: *button }
            }
            _ => {
                return Err(String::from(
                    "Only key and mouse button events can toggle between pressed and released states.",
                ));
            }
        };
        self.working_recording = rebuild_recording(metadata, events)?;
        self.select_event_clamped(Some(selected_event_index));
        Ok(())
    }

    fn nudge_selected_event(&mut self, delta_micros: i64) -> Result<(), String> {
        let Some(selected_event_index) = self.selected_event_index else {
            return Err(String::from("Select an event before changing its timing."));
        };

        let metadata = self.working_recording.metadata().clone();
        let mut events = self.working_recording.events().to_vec();
        let minimum_micros = if selected_event_index == 0 {
            0
        } else {
            events[selected_event_index - 1].elapsed_time.as_micros()
        };
        let maximum_micros = events
            .get(selected_event_index + 1)
            .map(|event| event.elapsed_time.as_micros())
            .unwrap_or(u64::MAX);
        let current_micros = events[selected_event_index].elapsed_time.as_micros() as i128;
        let adjusted_micros = (current_micros + i128::from(delta_micros))
            .clamp(i128::from(minimum_micros), i128::from(maximum_micros))
            as u64;

        events[selected_event_index].elapsed_time = ElapsedTime::from_micros(adjusted_micros);
        self.working_recording = rebuild_recording(metadata, events)?;
        self.select_event_clamped(Some(selected_event_index));
        Ok(())
    }

    fn revert(&mut self) {
        let selected_event_index = self.selected_event_index;
        self.working_recording = self.original_recording.clone();
        self.select_event_clamped(selected_event_index);
    }
}

impl RecordingLibraryState {
    fn discover(selected_path: Option<&Path>) -> Self {
        let session_directory = default_session_directory();
        let mut recordings = Vec::new();
        let mut invalid_recordings = Vec::new();

        if let Ok(directory_entries) = fs::read_dir(&session_directory) {
            for directory_entry in directory_entries.flatten() {
                let path = directory_entry.path();

                if !path.is_file()
                    || path.extension().and_then(|extension| extension.to_str()) != Some("json")
                {
                    continue;
                }

                match load_recording_entry(&path) {
                    Ok(recording_entry) => recordings.push(recording_entry),
                    Err(reason) => invalid_recordings.push(InvalidRecordingEntry { path, reason }),
                }
            }
        }

        recordings.sort_by(|left, right| left.path.file_name().cmp(&right.path.file_name()));
        invalid_recordings
            .sort_by(|left, right| left.path.file_name().cmp(&right.path.file_name()));

        let selected_recording_index = if recordings.is_empty() {
            None
        } else {
            selected_path
                .and_then(|selected_path| {
                    recordings
                        .iter()
                        .position(|recording| recording.path == selected_path)
                })
                .or(Some(0))
        };

        Self {
            session_directory,
            recordings,
            invalid_recordings,
            selected_recording_index,
        }
    }

    fn selected_recording(&self) -> Option<&RecordingLibraryEntry> {
        self.selected_recording_index
            .and_then(|index| self.recordings.get(index))
    }

    fn select(&mut self, index: usize) {
        if index < self.recordings.len() {
            self.selected_recording_index = Some(index);
        }
    }
}

impl RecordingLibraryEntry {
    fn display_title(&self) -> String {
        display_title_for_recording(&self.path, &self.recording)
    }
}

/// Runs the desktop shell for regxorder.
pub fn run_desktop_ui() -> Result<(), slint::PlatformError> {
    let app_window = AppWindow::new()?;
    let model = Arc::new(Mutex::new(DesktopShellModel::load()));
    apply_model_to_window(&app_window, &model);

    let app_window_weak = app_window.as_weak();
    let model_for_refresh = Arc::clone(&model);
    app_window.on_refresh_library_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_refresh).refresh_library();
        apply_model_to_window(&app_window, &model_for_refresh);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_selection = Arc::clone(&model);
    app_window.on_select_recording_requested(move |index| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if index < 0 {
            return;
        }

        lock_model(&model_for_selection).select_recording(index as usize);
        apply_model_to_window(&app_window, &model_for_selection);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_recording_delete = Arc::clone(&model);
    app_window.on_delete_selected_recording_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_recording_delete).delete_selected_recording() {
            lock_model(&model_for_recording_delete).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_recording_delete);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_editor_view = Arc::clone(&model);
    app_window.on_show_editor_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_editor_view).set_active_view(ActiveView::Editor);
        apply_model_to_window(&app_window, &model_for_editor_view);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_diagnostics_view = Arc::clone(&model);
    app_window.on_show_diagnostics_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_diagnostics_view).set_active_view(ActiveView::Diagnostics);
        apply_model_to_window(&app_window, &model_for_diagnostics_view);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_settings_view = Arc::clone(&model);
    app_window.on_show_settings_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_settings_view).set_active_view(ActiveView::Settings);
        apply_model_to_window(&app_window, &model_for_settings_view);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_event_selection = Arc::clone(&model);
    app_window.on_select_event_requested(move |index| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if index < 0 {
            return;
        }

        lock_model(&model_for_event_selection).select_event(index as usize);
        apply_model_to_window(&app_window, &model_for_event_selection);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_key_filter = Arc::clone(&model);
    app_window.on_filter_key_events_requested(move |enabled| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_key_filter).set_show_key_events(enabled);
        apply_model_to_window(&app_window, &model_for_key_filter);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_button_filter = Arc::clone(&model);
    app_window.on_filter_mouse_button_events_requested(move |enabled| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_button_filter).set_show_mouse_button_events(enabled);
        apply_model_to_window(&app_window, &model_for_button_filter);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_wheel_filter = Arc::clone(&model);
    app_window.on_filter_mouse_wheel_events_requested(move |enabled| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_wheel_filter).set_show_mouse_wheel_events(enabled);
        apply_model_to_window(&app_window, &model_for_wheel_filter);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_pointer_filter = Arc::clone(&model);
    app_window.on_filter_pointer_move_events_requested(move |enabled| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_pointer_filter).set_show_pointer_move_events(enabled);
        apply_model_to_window(&app_window, &model_for_pointer_filter);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_delete_visible_event = Arc::clone(&model);
    app_window.on_delete_visible_event_requested(move |index| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if index < 0 {
            return;
        }

        if let Err(message) =
            lock_model(&model_for_delete_visible_event).delete_visible_event(index as usize)
        {
            lock_model(&model_for_delete_visible_event).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_delete_visible_event);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_control_modifier = Arc::clone(&model);
    app_window.on_toggle_stop_hotkey_control_requested(move |enabled| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_control_modifier)
            .set_stop_hotkey_modifier(HotkeyModifier::Control, enabled)
        {
            lock_model(&model_for_control_modifier).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_control_modifier);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_alt_modifier = Arc::clone(&model);
    app_window.on_toggle_stop_hotkey_alt_requested(move |enabled| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_alt_modifier)
            .set_stop_hotkey_modifier(HotkeyModifier::Alt, enabled)
        {
            lock_model(&model_for_alt_modifier).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_alt_modifier);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_shift_modifier = Arc::clone(&model);
    app_window.on_toggle_stop_hotkey_shift_requested(move |enabled| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_shift_modifier)
            .set_stop_hotkey_modifier(HotkeyModifier::Shift, enabled)
        {
            lock_model(&model_for_shift_modifier).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_shift_modifier);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_win_modifier = Arc::clone(&model);
    app_window.on_toggle_stop_hotkey_win_requested(move |enabled| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_win_modifier)
            .set_stop_hotkey_modifier(HotkeyModifier::Win, enabled)
        {
            lock_model(&model_for_win_modifier).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_win_modifier);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_hotkey_key = Arc::clone(&model);
    app_window.on_select_stop_hotkey_key_requested(move |index| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Some(hotkey_key) = hotkey_key_from_index(index) {
            lock_model(&model_for_hotkey_key).set_stop_hotkey_key(hotkey_key);
        }

        apply_model_to_window(&app_window, &model_for_hotkey_key);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_save = Arc::clone(&model);
    app_window.on_save_editor_changes_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_save).save_editor_changes() {
            lock_model(&model_for_save).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_save);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_revert = Arc::clone(&model);
    app_window.on_revert_editor_changes_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_revert).revert_editor_changes();
        apply_model_to_window(&app_window, &model_for_revert);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_delete = Arc::clone(&model);
    app_window.on_delete_selected_event_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_delete).delete_selected_event() {
            lock_model(&model_for_delete).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_delete);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_duplicate = Arc::clone(&model);
    app_window.on_duplicate_selected_event_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_duplicate).duplicate_selected_event() {
            lock_model(&model_for_duplicate).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_duplicate);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_toggle = Arc::clone(&model);
    app_window.on_toggle_selected_event_state_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_toggle).toggle_selected_event_state() {
            lock_model(&model_for_toggle).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_toggle);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_earlier = Arc::clone(&model);
    app_window.on_nudge_selected_event_earlier_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_earlier).nudge_selected_event(-1_000) {
            lock_model(&model_for_earlier).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_earlier);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_later = Arc::clone(&model);
    app_window.on_nudge_selected_event_later_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        if let Err(message) = lock_model(&model_for_later).nudge_selected_event(1_000) {
            lock_model(&model_for_later).note_action_notice(message);
        }

        apply_model_to_window(&app_window, &model_for_later);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_raw_input = Arc::clone(&model);
    app_window.on_choose_raw_input_recording_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_raw_input).set_recording_strategy(RecordingStrategy::RawInput);
        apply_model_to_window(&app_window, &model_for_raw_input);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_low_level = Arc::clone(&model);
    app_window.on_choose_low_level_hooks_recording_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_low_level).set_recording_strategy(RecordingStrategy::LowLevelHooks);
        apply_model_to_window(&app_window, &model_for_low_level);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_playback_loop = Arc::clone(&model);
    app_window.on_playback_loop_input_edited(move |value| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_playback_loop).set_playback_loop_input_text(value.to_string());
        apply_model_to_window(&app_window, &model_for_playback_loop);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_playback_loop_commit = Arc::clone(&model);
    app_window.on_playback_loop_input_committed(move |value| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        let mut model = lock_model(&model_for_playback_loop_commit);
        model.set_playback_loop_input_text(value.to_string());
        model.commit_playback_loop_input_text();
        drop(model);

        apply_model_to_window(&app_window, &model_for_playback_loop_commit);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_playback_speed = Arc::clone(&model);
    app_window.on_playback_speed_input_edited(move |value| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_playback_speed).set_playback_speed_input_text(value.to_string());
        apply_model_to_window(&app_window, &model_for_playback_speed);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_playback_speed_commit = Arc::clone(&model);
    app_window.on_playback_speed_input_committed(move |value| {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        let mut model = lock_model(&model_for_playback_speed_commit);
        model.set_playback_speed_input_text(value.to_string());
        model.commit_playback_speed_input_text();
        drop(model);

        apply_model_to_window(&app_window, &model_for_playback_speed_commit);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_half_speed = Arc::clone(&model);
    app_window.on_choose_half_speed_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_half_speed).set_playback_speed(
            SpeedMultiplier::new(0.5).expect("slow playback speed should be valid"),
        );
        apply_model_to_window(&app_window, &model_for_half_speed);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_normal_speed = Arc::clone(&model);
    app_window.on_choose_normal_speed_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_normal_speed).set_playback_speed(
            SpeedMultiplier::new(1.0).expect("normal playback speed should be valid"),
        );
        apply_model_to_window(&app_window, &model_for_normal_speed);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_fast_speed = Arc::clone(&model);
    app_window.on_choose_fast_speed_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_fast_speed).set_playback_speed(
            SpeedMultiplier::new(2.0).expect("fast playback speed should be valid"),
        );
        apply_model_to_window(&app_window, &model_for_fast_speed);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_recording = Arc::clone(&model);
    app_window.on_start_recording_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        let recording_request = {
            let mut model = lock_model(&model_for_recording);
            match model.begin_recording_request() {
                Ok(recording_request) => Some(recording_request),
                Err(message) => {
                    model.note_action_notice(message);
                    None
                }
            }
        };

        apply_model_to_window(&app_window, &model_for_recording);

        let Some(recording_request) = recording_request else {
            return;
        };

        let model_for_completion = Arc::clone(&model_for_recording);
        let app_window_weak = app_window.as_weak();
        std::thread::spawn(move || {
            let action_execution_state = run_recording_request(recording_request);
            let _ = slint::invoke_from_event_loop(move || {
                let state = {
                    let mut model = lock_model(&model_for_completion);
                    model.finish_action(action_execution_state);
                    DesktopShellState::from_model(&model)
                };

                if let Some(app_window) = app_window_weak.upgrade() {
                    state.apply_to(&app_window);
                }
            });
        });
    });

    let app_window_weak = app_window.as_weak();
    let model_for_stop = Arc::clone(&model);
    app_window.on_stop_active_action_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_stop).request_stop_active_action();
        apply_model_to_window(&app_window, &model_for_stop);
    });

    let app_window_weak = app_window.as_weak();
    let model_for_playback = Arc::clone(&model);
    app_window.on_play_selected_recording_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        let playback_request = {
            let mut model = lock_model(&model_for_playback);
            match model.begin_playback_request() {
                Ok(playback_request) => Some(playback_request),
                Err(message) => {
                    model.note_action_notice(message);
                    None
                }
            }
        };

        apply_model_to_window(&app_window, &model_for_playback);

        let Some(playback_request) = playback_request else {
            return;
        };

        let model_for_completion = Arc::clone(&model_for_playback);
        let app_window_weak = app_window.as_weak();
        std::thread::spawn(move || {
            let action_execution_state = run_playback_request(playback_request);
            let _ = slint::invoke_from_event_loop(move || {
                let state = {
                    let mut model = lock_model(&model_for_completion);
                    model.finish_action(action_execution_state);
                    DesktopShellState::from_model(&model)
                };

                if let Some(app_window) = app_window_weak.upgrade() {
                    state.apply_to(&app_window);
                }
            });
        });
    });

    let app_window_weak = app_window.as_weak();
    let model_for_diagnostics = Arc::clone(&model);
    app_window.on_refresh_diagnostics_requested(move || {
        let Some(app_window) = app_window_weak.upgrade() else {
            return;
        };

        lock_model(&model_for_diagnostics).refresh_diagnostics();
        apply_model_to_window(&app_window, &model_for_diagnostics);
    });

    app_window.run()
}

fn apply_model_to_window(app_window: &AppWindow, model: &Arc<Mutex<DesktopShellModel>>) {
    let state = {
        let model = lock_model(model);
        DesktopShellState::from_model(&model)
    };

    state.apply_to(app_window);
}

fn lock_model(model: &Arc<Mutex<DesktopShellModel>>) -> MutexGuard<'_, DesktopShellModel> {
    model
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn run_playback_request(playback_request: PlaybackRequest) -> ActionExecutionState {
    let mut total_dispatched_events = 0usize;
    let mut interrupted = false;
    let mut elapsed_millis = 0.0;

    for loop_index in 0..playback_request.loop_count {
        if loop_index > 0 && playback_request.stop_requested.load(Ordering::SeqCst) {
            interrupted = true;
            break;
        }

        let playback_result = ControlController.run_playback_action(
            &playback_request.recording,
            playback_request.speed,
            playback_request.stop_hotkey.as_ref(),
            &playback_request.stop_requested,
        );

        match playback_result {
            Ok(playback_outcome) => {
                total_dispatched_events += playback_outcome.playback_report.dispatched_events;
                elapsed_millis += playback_outcome.playback_report.elapsed.as_secs_f64() * 1_000.0;
                interrupted |= playback_outcome.playback_report.interrupted;

                if playback_outcome.playback_report.interrupted {
                    break;
                }
            }
            Err(error) => {
                return ActionExecutionState::PlaybackFailed {
                    recording_title: playback_request.recording_title,
                    speed: playback_request.speed,
                    reason: error.to_string(),
                };
            }
        }
    }

    ActionExecutionState::PlaybackCompleted {
        recording_title: playback_request.recording_title,
        speed: playback_request.speed,
        dispatched_events: total_dispatched_events,
        interrupted,
        elapsed_millis,
    }
}

fn run_recording_request(recording_request: RecordingRequest) -> ActionExecutionState {
    let recording_result = ControlController.run_recording_action(
        recording_request.strategy,
        Some(recording_request.recording_title.clone()),
        recording_request.stop_hotkey.as_ref(),
        &recording_request.stop_requested,
    );

    match recording_result {
        Ok(recording_outcome) => {
            match write_recording_file(&recording_request.output_path, &recording_outcome.recording)
            {
                Ok(()) => ActionExecutionState::RecordingCompleted {
                    recording_title: recording_request.recording_title,
                    output_path: recording_request.output_path,
                    event_count: recording_outcome.recording.event_count(),
                    duration_micros: recording_outcome.recording.duration().as_micros(),
                },
                Err(reason) => ActionExecutionState::RecordingFailed {
                    recording_title: recording_request.recording_title,
                    reason,
                },
            }
        }
        Err(error) => ActionExecutionState::RecordingFailed {
            recording_title: recording_request.recording_title,
            reason: error.to_string(),
        },
    }
}

fn load_environment_report() -> EnvironmentDoctorReport {
    let hotkey_probe = HotkeyBinding::new(
        vec![
            HotkeyModifier::Control,
            HotkeyModifier::Alt,
            HotkeyModifier::Shift,
        ],
        HotkeyKey::F12,
    )
    .expect("desktop-shell diagnostics probe hotkey should always be valid");

    diagnose_windows_environment(hotkey_probe)
}

fn load_recording_entry(path: &Path) -> Result<RecordingLibraryEntry, String> {
    let input = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let recording = Recording::from_json_str(&input).map_err(|error| error.to_string())?;
    let file_size_bytes = fs::metadata(path).map_err(|error| error.to_string())?.len();
    let metrics = recording.metrics();

    Ok(RecordingLibraryEntry {
        path: path.to_path_buf(),
        recording,
        metrics,
        file_size_bytes,
    })
}

fn rebuild_recording(
    metadata: RecordingMetadata,
    events: Vec<InputEvent>,
) -> Result<Recording, String> {
    let resequenced_events = events
        .into_iter()
        .enumerate()
        .map(|(index, mut event)| {
            event.sequence = index as u64;
            event
        })
        .collect();

    Recording::new(metadata, resequenced_events).map_err(|error| error.to_string())
}

fn build_diagnostics_summary(
    report: &EnvironmentDoctorReport,
    editing_session: Option<&EditingSession>,
) -> String {
    format!(
        "Status {} · recording {} · playback {} · hotkey {}{}",
        format_diagnostic_status(report.summary.overall_status()),
        report.recording_backends.join(", "),
        report.playback_backend,
        report.hotkey_backend,
        editing_session
            .map(|editing_session| format!(" · session {}", editing_session.display_title()))
            .unwrap_or_default(),
    )
}

fn build_diagnostic_checks_text(
    report: &EnvironmentDoctorReport,
    playback_summary: &str,
    playback_checks: &str,
) -> String {
    let environment_issues =
        build_issue_lines(&report.checks, "No environment warnings or failures.");

    format!(
        "Playback\n{}\n{}\n\nEnvironment\n{}",
        playback_summary, playback_checks, environment_issues
    )
}

fn build_settings_summary(model: &DesktopShellModel) -> String {
    format!(
        "Recording strategy {} · playback {}x · stop hotkey {} · {}",
        model.configuration.recording_strategy,
        model.selected_playback_speed.get(),
        model.configuration.stop_action_hotkey,
        compact_process_elevation_status(&model.environment_report)
    )
}

fn format_event_action_label(action: &InputAction) -> String {
    match action {
        InputAction::KeyPressed { .. } => String::from("Key Down"),
        InputAction::KeyReleased { .. } => String::from("Key Up"),
        InputAction::PointerMoved { .. } => String::from("Mouse Move"),
        InputAction::MouseButtonPressed { button } => {
            format!("{} Down", format_mouse_button(*button))
        }
        InputAction::MouseButtonReleased { button } => {
            format!("{} Up", format_mouse_button(*button))
        }
        InputAction::MouseWheelScrolled { axis, delta } => {
            format!("{} Wheel {}", format_scroll_axis(*axis), delta)
        }
    }
}

fn format_event_table_type_label(action: &InputAction) -> String {
    match action {
        InputAction::KeyPressed { .. } => String::from("Key Down"),
        InputAction::KeyReleased { .. } => String::from("Key Up"),
        InputAction::PointerMoved { .. } => String::from("Mouse Move"),
        InputAction::MouseButtonPressed { .. } => String::from("Click Down"),
        InputAction::MouseButtonReleased { .. } => String::from("Click Up"),
        InputAction::MouseWheelScrolled { delta, .. } => {
            if *delta >= 0 {
                String::from("Wheel Up")
            } else {
                String::from("Wheel Down")
            }
        }
    }
}

fn format_event_table_parameter_label(action: &InputAction) -> String {
    match action {
        InputAction::KeyPressed { key } | InputAction::KeyReleased { key } => {
            let mut parameter_label = key
                .logical_name
                .as_ref()
                .filter(|logical_name| !logical_name.trim().is_empty())
                .map(|logical_name| format!("{} · scan {}", logical_name, key.scan_code.get()))
                .unwrap_or_else(|| format!("scan {}", key.scan_code.get()));

            if key.extended {
                parameter_label.push_str(" · ext");
            }

            parameter_label
        }
        InputAction::PointerMoved { position } => {
            format!("({}, {})", position.absolute.x, position.absolute.y)
        }
        InputAction::MouseButtonPressed { button }
        | InputAction::MouseButtonReleased { button } => format_mouse_button(*button).to_string(),
        InputAction::MouseWheelScrolled { axis, delta } => {
            let direction = if *delta >= 0 { "up" } else { "down" };
            format!("{} {} ({})", format_scroll_axis(*axis), direction, delta)
        }
    }
}

fn format_mouse_button(button: regxorder_core::MouseButton) -> &'static str {
    match button {
        regxorder_core::MouseButton::Left => "Left",
        regxorder_core::MouseButton::Right => "Right",
        regxorder_core::MouseButton::Middle => "Middle",
        regxorder_core::MouseButton::X1 => "X1",
        regxorder_core::MouseButton::X2 => "X2",
    }
}

fn format_scroll_axis(axis: regxorder_core::ScrollAxis) -> &'static str {
    match axis {
        regxorder_core::ScrollAxis::Vertical => "Vertical",
        regxorder_core::ScrollAxis::Horizontal => "Horizontal",
    }
}

fn format_elapsed_clock(micros: u64) -> String {
    let total_millis = micros / 1_000;
    let millis = total_millis % 1_000;
    let total_seconds = total_millis / 1_000;
    let seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;

    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn compact_process_elevation_status(report: &EnvironmentDoctorReport) -> String {
    report
        .checks
        .iter()
        .find(|check| check.name == "process_elevation")
        .map(|check| match check.status {
            DiagnosticStatus::Pass => String::from("process elevated"),
            DiagnosticStatus::Warn => String::from("process not elevated"),
            DiagnosticStatus::Fail => String::from("elevation status unavailable"),
        })
        .unwrap_or_else(|| String::from("elevation status unavailable"))
}

fn default_playback_speed() -> SpeedMultiplier {
    SpeedMultiplier::new(DEFAULT_PLAYBACK_SPEED_VALUE)
        .expect("default playback speed should be valid")
}

fn clamp_playback_loop_input(input: &str) -> Option<u16> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return None;
    }

    let parsed_loop_count = trimmed.parse::<i32>().ok()?;

    Some(normalize_playback_loop_count(parsed_loop_count))
}

fn format_playback_loop_input(loop_count: u16) -> String {
    loop_count.to_string()
}

fn clamp_playback_speed_input(input: &str) -> Option<SpeedMultiplier> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return None;
    }

    let parsed_speed = trimmed.parse::<f64>().ok()?;

    if !parsed_speed.is_finite() {
        return None;
    }

    let clamped_speed = normalize_playback_speed_value(parsed_speed);

    SpeedMultiplier::new(clamped_speed).ok()
}

fn format_playback_speed_input(speed: SpeedMultiplier) -> String {
    let mut formatted = format!("{:.2}", speed.get());

    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }

    if formatted.ends_with('.') {
        formatted.push('0');
    }

    formatted
}

fn normalize_playback_loop_count(parsed_loop_count: i32) -> u16 {
    let clamped_loop_count = if parsed_loop_count <= 0 {
        MIN_PLAYBACK_LOOP_COUNT as i32
    } else {
        parsed_loop_count.clamp(
            MIN_PLAYBACK_LOOP_COUNT as i32,
            MAX_PLAYBACK_LOOP_COUNT as i32,
        )
    };

    clamped_loop_count as u16
}

fn normalize_playback_speed_value(parsed_speed: f64) -> f64 {
    let clamped_speed = if parsed_speed <= 0.0 {
        MIN_PLAYBACK_SPEED_VALUE
    } else {
        parsed_speed.clamp(MIN_PLAYBACK_SPEED_VALUE, MAX_PLAYBACK_SPEED_VALUE)
    };

    ((clamped_speed * 100.0).round() / 100.0)
        .clamp(MIN_PLAYBACK_SPEED_VALUE, MAX_PLAYBACK_SPEED_VALUE)
}

fn build_issue_lines(checks: &[regxorder_core::DiagnosticCheck], empty_message: &str) -> String {
    let issues = checks
        .iter()
        .filter(|check| check.status != DiagnosticStatus::Pass)
        .map(|check| {
            format!(
                "[{}] {}: {}",
                format_diagnostic_status(check.status),
                check.name,
                check.summary
            )
        })
        .collect::<Vec<_>>();

    if issues.is_empty() {
        String::from(empty_message)
    } else {
        issues.join("\n")
    }
}

fn format_diagnostic_status(status: DiagnosticStatus) -> &'static str {
    match status {
        DiagnosticStatus::Pass => "pass",
        DiagnosticStatus::Warn => "warn",
        DiagnosticStatus::Fail => "fail",
    }
}

fn hotkey_key_index(key: HotkeyKey) -> i32 {
    HOTKEY_KEY_OPTIONS
        .iter()
        .position(|candidate| *candidate == key)
        .map(|index| index as i32)
        .unwrap_or(0)
}

fn hotkey_key_from_index(index: i32) -> Option<HotkeyKey> {
    usize::try_from(index)
        .ok()
        .and_then(|index| HOTKEY_KEY_OPTIONS.get(index).copied())
}

fn default_session_directory() -> PathBuf {
    std::env::current_dir()
        .map(|directory| directory.join(SESSION_DIRECTORY_NAME))
        .unwrap_or_else(|_| PathBuf::from(SESSION_DIRECTORY_NAME))
}

fn next_recording_destination(
    session_directory: &Path,
    recording_library: &RecordingLibraryState,
) -> (String, PathBuf) {
    let mut next_index = 1_usize;

    while next_index
        <= recording_library.recordings.len() + recording_library.invalid_recordings.len() + 1
    {
        let recording_stem = format!("capture-{next_index:03}");
        let candidate_path = session_directory.join(format!("{recording_stem}.json"));

        if !candidate_path.exists() {
            return (format!("Capture {next_index:03}"), candidate_path);
        }

        next_index += 1;
    }

    loop {
        let recording_stem = format!("capture-{next_index:03}");
        let candidate_path = session_directory.join(format!("{recording_stem}.json"));

        if !candidate_path.exists() {
            return (format!("Capture {next_index:03}"), candidate_path);
        }

        next_index += 1;
    }
}

fn write_recording_file(path: &Path, recording: &Recording) -> Result<(), String> {
    if let Some(parent_directory) = path.parent() {
        fs::create_dir_all(parent_directory).map_err(|error| error.to_string())?;
    }

    let json = recording
        .to_json_pretty()
        .map_err(|error| error.to_string())?;
    fs::write(path, json).map_err(|error| error.to_string())
}

fn format_duration(micros: u64) -> String {
    if micros >= 1_000_000 {
        format!("{:.3} s", micros as f64 / 1_000_000.0)
    } else if micros >= 1_000 {
        format!("{:.3} ms", micros as f64 / 1_000.0)
    } else {
        format!("{micros} us")
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use regxorder_core::{
        AbsoluteScreenPoint, DiagnosticCheck, DisplayMetadata, ElapsedTime, HotkeyBinding,
        HotkeyKey, HotkeyModifier, InputAction, InputEvent, KeyDescriptor, MouseButton,
        NormalizedCoordinate, NormalizedScreenPoint, PointerPosition, Recording, RecordingMetadata,
        ScanCode, SchemaVersion, ScreenSize, SpeedMultiplier,
    };
    use slint::SharedString;

    use super::{
        DesktopShellModel, DesktopShellState, EventListFilter, InvalidRecordingEntry,
        RecordingLibraryEntry, RecordingLibraryState, build_diagnostic_checks_text,
        build_recording_title_items, build_visible_event_rows, display_title_for_recording,
        load_recording_entry,
    };

    #[test]
    fn shell_state_from_model_mentions_live_environment_status() {
        let state = DesktopShellState::from_model(&sample_model());

        assert!(state.diagnostics_summary.to_string().contains("Status"));
    }

    #[test]
    fn dirty_editor_disables_recording_and_playback_controls() {
        let mut model = sample_model();

        model
            .delete_visible_event(0)
            .expect("deleting a visible event should dirty the editor");

        let state = DesktopShellState::from_model(&model);
        assert!(!state.can_start_recording);
        assert!(!state.can_play_selected_recording);
        assert!(!state.can_delete_selected_recording);
    }

    #[test]
    fn recording_list_builders_return_titles_and_subtitles() {
        let model = sample_model();
        let recording_titles = build_recording_title_items(&model.recording_library);

        assert_eq!(recording_titles[0], SharedString::from("Alpha sample"));
    }

    #[test]
    fn blank_recording_titles_fall_back_to_the_file_name() {
        let recording = sample_recording(None);

        assert_eq!(
            display_title_for_recording(Path::new("sessions/capture-001.json"), &recording),
            "capture-001.json"
        );
    }

    #[test]
    fn load_recording_entry_reads_a_valid_session_file() {
        let test_directory = create_test_directory();
        let recording_path = test_directory.join("sample.json");
        let recording = sample_recording(Some("Library sample"));
        fs::write(
            &recording_path,
            recording
                .to_json_pretty()
                .expect("sample recording should serialize"),
        )
        .expect("sample recording should write");

        let entry = load_recording_entry(&recording_path).expect("recording entry should load");

        assert_eq!(entry.recording.event_count(), 2);
        assert_eq!(
            entry.recording.metadata().title.as_deref(),
            Some("Library sample")
        );

        fs::remove_dir_all(test_directory).expect("test directory should clean up");
    }

    #[test]
    fn diagnostic_checks_text_renders_check_names_and_statuses() {
        let report = sample_model().environment_report;
        let checks_text = build_diagnostic_checks_text(
            &report,
            "Playback ready.",
            "No playback warnings or failures.",
        );

        assert!(checks_text.contains("[warn] process_elevation"));
        assert!(!checks_text.contains("[pass] recording_backends"));
    }

    #[test]
    fn playback_request_uses_selected_recording_and_speed() {
        let mut model = sample_model();
        model.set_playback_loop_input_text(String::from("3"));
        model.commit_playback_loop_input_text();
        model.set_playback_speed(SpeedMultiplier::new(2.0).expect("speed should be valid"));

        let playback_request = model
            .begin_playback_request()
            .expect("playback request should be available");

        assert_eq!(playback_request.recording_title, "Alpha sample");
        assert_eq!(playback_request.loop_count, 3);
        assert_eq!(playback_request.speed.get(), 2.0);
        assert!(matches!(
            model.action_execution_state,
            super::ActionExecutionState::PlaybackRunning { .. }
        ));
        assert!(model.current_action_stop_requested.is_some());
    }

    #[test]
    fn playback_loop_input_clamps_to_supported_range() {
        let mut model = sample_model();

        model.set_playback_loop_input_text(String::from("0"));
        assert_eq!(model.selected_playback_loop_count, 1);
        assert_eq!(model.playback_loop_input_text, "0");

        model.commit_playback_loop_input_text();
        assert_eq!(model.playback_loop_input_text, "1");

        model.set_playback_loop_input_text(String::from("12000"));
        assert_eq!(model.selected_playback_loop_count, 9999);

        model.commit_playback_loop_input_text();
        assert_eq!(model.playback_loop_input_text, "9999");
    }

    #[test]
    fn playback_speed_input_accepts_valid_decimal_values() {
        let mut model = sample_model();

        model.set_playback_speed_input_text(String::from("2.5"));

        assert_eq!(model.selected_playback_speed.get(), 2.5);
        assert_eq!(model.playback_speed_input_text, "2.5");
        assert_eq!(model.playback_speed_validation_message, None);
        assert!(DesktopShellState::from_model(&model).can_play_selected_recording);
    }

    #[test]
    fn playback_speed_input_clamps_values_above_ten() {
        let mut model = sample_model();

        model.set_playback_speed_input_text(String::from("11"));

        assert_eq!(model.selected_playback_speed.get(), 10.0);
        assert_eq!(model.playback_speed_input_text, "11");
        assert_eq!(model.playback_speed_validation_message, None);
        assert!(DesktopShellState::from_model(&model).can_play_selected_recording);

        model.commit_playback_speed_input_text();
        assert_eq!(model.playback_speed_input_text, "10.0");

        let playback_request = model
            .begin_playback_request()
            .expect("clamped playback speed should remain playable");
        assert_eq!(playback_request.speed.get(), 10.0);
    }

    #[test]
    fn playback_speed_input_clamps_below_minimum_and_rounds_to_two_decimals() {
        let mut model = sample_model();

        model.set_playback_speed_input_text(String::from("0.045"));

        assert_eq!(model.selected_playback_speed.get(), 0.1);
        assert_eq!(model.playback_speed_input_text, "0.045");

        model.commit_playback_speed_input_text();
        assert_eq!(model.playback_speed_input_text, "0.1");

        model.set_playback_speed_input_text(String::from("1.236"));
        model.commit_playback_speed_input_text();

        assert_eq!(model.selected_playback_speed.get(), 1.24);
        assert_eq!(model.playback_speed_input_text, "1.24");
    }

    #[test]
    fn recording_request_uses_next_available_capture_path() {
        let mut model = sample_model();

        let recording_request = model
            .begin_recording_request()
            .expect("recording request should be available");

        assert_eq!(recording_request.recording_title, "Capture 001");
        assert_eq!(
            recording_request.output_path,
            PathBuf::from("sessions").join("capture-001.json")
        );
        assert!(matches!(
            model.action_execution_state,
            super::ActionExecutionState::RecordingRunning { .. }
        ));
    }

    #[test]
    fn deleting_selected_event_marks_editor_dirty_and_resequences() {
        let mut model = sample_model();

        model
            .delete_selected_event()
            .expect("selected event should delete cleanly");

        let editing_session = model
            .editing_session
            .as_ref()
            .expect("editing session should still exist");
        assert!(editing_session.is_dirty());
        assert_eq!(editing_session.working_recording.event_count(), 1);
        assert_eq!(editing_session.working_recording.events()[0].sequence, 0);
    }

    #[test]
    fn duplicating_selected_event_inserts_a_new_editable_row() {
        let mut model = sample_model();

        model
            .duplicate_selected_event()
            .expect("selected event should duplicate cleanly");

        let editing_session = model
            .editing_session
            .as_ref()
            .expect("editing session should still exist");
        assert!(editing_session.is_dirty());
        assert_eq!(editing_session.working_recording.event_count(), 3);
        assert_eq!(editing_session.selected_event_index, Some(1));
        assert_eq!(
            editing_session.working_recording.events()[1].action,
            editing_session.working_recording.events()[0].action
        );
    }

    #[test]
    fn toggling_selected_key_event_switches_pressed_and_released_state() {
        let mut model = sample_model();

        model
            .toggle_selected_event_state()
            .expect("selected key event should toggle cleanly");

        let editing_session = model
            .editing_session
            .as_ref()
            .expect("editing session should still exist");
        assert!(matches!(
            editing_session.working_recording.events()[0].action,
            InputAction::KeyReleased { .. }
        ));
    }

    #[test]
    fn deleting_visible_event_updates_only_the_working_copy_until_saved() {
        let test_directory = create_test_directory();
        let session_directory = test_directory.join("sessions");
        let recording_path = session_directory.join("mixed.json");
        let recording = sample_mixed_recording();

        super::write_recording_file(&recording_path, &recording)
            .expect("sample recording should write");
        let recording_entry =
            load_recording_entry(&recording_path).expect("recording entry should reload from disk");
        let mut model = sample_model();
        model.recording_library = RecordingLibraryState {
            session_directory: session_directory.clone(),
            recordings: vec![recording_entry.clone()],
            invalid_recordings: Vec::new(),
            selected_recording_index: Some(0),
        };
        model.editing_session = Some(super::EditingSession::from_library_entry(&recording_entry));
        model.event_list_filter = EventListFilter::default();

        model
            .delete_visible_event(1)
            .expect("visible button row should delete cleanly");

        let working_recording = &model
            .editing_session
            .as_ref()
            .expect("editing session should remain available")
            .working_recording;
        assert_eq!(working_recording.event_count(), 4);
        assert!(!working_recording.events().iter().any(|event| {
            matches!(
                event.action,
                InputAction::MouseButtonPressed {
                    button: MouseButton::Left
                }
            )
        }));

        let reloaded_entry = load_recording_entry(&recording_path)
            .expect("source recording should still parse before save");
        assert_eq!(reloaded_entry.recording.event_count(), 5);

        model
            .save_editor_changes()
            .expect("buffered delete should save cleanly");
        let persisted_entry =
            load_recording_entry(&recording_path).expect("recording should still parse after save");
        assert_eq!(persisted_entry.recording.event_count(), 4);

        fs::remove_dir_all(test_directory).expect("test directory should clean up");
    }

    #[test]
    fn nudging_selected_event_clamps_against_neighbor_timings() {
        let mut model = sample_model();
        model.select_event(1);

        model
            .nudge_selected_event(-100_000)
            .expect("selected event timing should update");

        let editing_session = model
            .editing_session
            .as_ref()
            .expect("editing session should still exist");
        assert_eq!(
            editing_session.working_recording.events()[1]
                .elapsed_time
                .as_micros(),
            editing_session.working_recording.events()[0]
                .elapsed_time
                .as_micros()
        );
    }

    #[test]
    fn visible_event_rows_hide_pointer_moves_by_default() {
        let recording_entry = RecordingLibraryEntry {
            path: PathBuf::from("sessions/mixed.json"),
            metrics: sample_mixed_recording().metrics(),
            file_size_bytes: 640,
            recording: sample_mixed_recording(),
        };
        let editing_session = super::EditingSession::from_library_entry(&recording_entry);
        let visible_rows =
            build_visible_event_rows(Some(&editing_session), EventListFilter::default());

        assert_eq!(visible_rows.len(), 3);
        assert!(
            !visible_rows
                .iter()
                .any(|row| row.type_label == "Mouse Move")
        );
    }

    #[test]
    fn shell_state_exposes_filter_counts_for_each_event_type() {
        let mixed_recording = sample_mixed_recording();
        let mixed_recording_entry = RecordingLibraryEntry {
            path: PathBuf::from("sessions/mixed.json"),
            metrics: mixed_recording.metrics(),
            file_size_bytes: 640,
            recording: mixed_recording,
        };
        let mut model = sample_model();
        model.recording_library.recordings = vec![mixed_recording_entry.clone()];
        model.recording_library.selected_recording_index = Some(0);
        model.editing_session = Some(super::EditingSession::from_library_entry(
            &mixed_recording_entry,
        ));

        let state = DesktopShellState::from_model(&model);

        assert_eq!(state.filter_key_event_count_label.to_string(), "1");
        assert_eq!(state.filter_mouse_button_event_count_label.to_string(), "2");
        assert_eq!(state.filter_mouse_wheel_event_count_label.to_string(), "0");
        assert_eq!(state.filter_pointer_move_event_count_label.to_string(), "2");
    }

    #[test]
    fn stop_hotkey_settings_update_the_requested_backend_hotkey() {
        let mut model = sample_model();

        model
            .set_stop_hotkey_modifier(HotkeyModifier::Alt, true)
            .expect("adding a modifier should keep the hotkey valid");
        model.set_stop_hotkey_key(HotkeyKey::F9);

        let playback_request = model
            .begin_playback_request()
            .expect("playback request should be available");

        assert_eq!(
            playback_request
                .stop_hotkey
                .as_ref()
                .expect("playback request should carry the configured hotkey")
                .to_string(),
            "Ctrl+Alt+Shift+F9"
        );
    }

    #[test]
    fn stop_hotkey_settings_reject_removing_the_last_modifier() {
        let mut model = sample_model();

        model
            .set_stop_hotkey_modifier(HotkeyModifier::Control, false)
            .expect("one modifier can be removed while one still remains");
        let error = model
            .set_stop_hotkey_modifier(HotkeyModifier::Shift, false)
            .expect_err("the last modifier should not be removable");

        assert!(error.contains("hotkeys require at least one modifier"));
        assert_eq!(
            model.configuration.stop_action_hotkey.to_string(),
            "Shift+F12"
        );
    }

    #[test]
    fn selecting_visible_event_uses_filtered_source_index() {
        let mixed_recording = sample_mixed_recording();
        let mut model = sample_model();
        model.editing_session = Some(super::EditingSession::from_library_entry(
            &RecordingLibraryEntry {
                path: PathBuf::from("sessions/mixed.json"),
                metrics: mixed_recording.metrics(),
                file_size_bytes: 640,
                recording: mixed_recording,
            },
        ));
        model.sync_event_selection_to_filter();

        model.select_event(1);

        assert_eq!(
            model
                .editing_session
                .as_ref()
                .and_then(|editing_session| editing_session.selected_event_index),
            Some(2)
        );
    }

    fn sample_model() -> DesktopShellModel {
        let alpha_recording = sample_recording(Some("Alpha sample"));
        let beta_recording = sample_recording(Some("Beta sample"));

        DesktopShellModel {
            active_view: super::ActiveView::Editor,
            recording_library: RecordingLibraryState {
                session_directory: PathBuf::from("sessions"),
                recordings: vec![
                    RecordingLibraryEntry {
                        path: PathBuf::from("sessions/alpha.json"),
                        metrics: alpha_recording.metrics(),
                        file_size_bytes: 512,
                        recording: alpha_recording,
                    },
                    RecordingLibraryEntry {
                        path: PathBuf::from("sessions/beta.json"),
                        metrics: beta_recording.metrics(),
                        file_size_bytes: 768,
                        recording: beta_recording,
                    },
                ],
                invalid_recordings: vec![InvalidRecordingEntry {
                    path: PathBuf::from("sessions/broken.json"),
                    reason: String::from("invalid json"),
                }],
                selected_recording_index: Some(0),
            },
            editing_session: Some(super::EditingSession::from_library_entry(
                &RecordingLibraryEntry {
                    path: PathBuf::from("sessions/alpha.json"),
                    metrics: sample_recording(Some("Alpha sample")).metrics(),
                    file_size_bytes: 512,
                    recording: sample_recording(Some("Alpha sample")),
                },
            )),
            configuration: super::AppConfiguration::default(),
            event_list_filter: EventListFilter::default(),
            environment_report: regxorder_core::EnvironmentDoctorReport::new(
                String::from("windows"),
                String::from("x86_64"),
                vec![String::from("raw_input"), String::from("low_level_hooks")],
                String::from("send_input"),
                String::from("register_hotkey"),
                HotkeyBinding::new(
                    vec![
                        HotkeyModifier::Control,
                        HotkeyModifier::Alt,
                        HotkeyModifier::Shift,
                    ],
                    HotkeyKey::F12,
                )
                .expect("sample hotkey should be valid"),
                vec![
                    DiagnosticCheck::pass(
                        "recording_backends",
                        "Raw Input and low-level hook recording backends are compiled into this build",
                    ),
                    DiagnosticCheck::warn(
                        "process_elevation",
                        "the current regxorder process is not elevated; playback into elevated targets may be blocked by UIPI",
                    ),
                ],
            ),
            selected_playback_loop_count: 1,
            selected_playback_speed: SpeedMultiplier::new(1.0)
                .expect("sample playback speed should be valid"),
            playback_loop_input_text: String::from("1"),
            playback_speed_input_text: String::from("1.0"),
            playback_speed_validation_message: None,
            action_execution_state: super::ActionExecutionState::Idle,
            current_action_stop_requested: None,
        }
    }

    fn sample_recording(title: Option<&str>) -> Recording {
        Recording::new(
            RecordingMetadata {
                schema_version: SchemaVersion::new(1),
                title: title.map(ToOwned::to_owned),
                display: DisplayMetadata {
                    virtual_origin: AbsoluteScreenPoint { x: 0, y: 0 },
                    virtual_size: ScreenSize {
                        width: 1920,
                        height: 1080,
                    },
                    monitors: Vec::new(),
                },
            },
            vec![
                InputEvent {
                    sequence: 0,
                    elapsed_time: ElapsedTime::from_micros(0),
                    action: InputAction::KeyPressed {
                        key: KeyDescriptor {
                            scan_code: ScanCode::new(30),
                            logical_name: Some(String::from("A")),
                            extended: false,
                        },
                    },
                },
                InputEvent {
                    sequence: 1,
                    elapsed_time: ElapsedTime::from_micros(40_000),
                    action: InputAction::KeyReleased {
                        key: KeyDescriptor {
                            scan_code: ScanCode::new(30),
                            logical_name: Some(String::from("A")),
                            extended: false,
                        },
                    },
                },
            ],
        )
        .expect("sample recording should be valid")
    }

    fn sample_mixed_recording() -> Recording {
        Recording::new(
            RecordingMetadata {
                schema_version: SchemaVersion::new(1),
                title: Some(String::from("Mixed sample")),
                display: DisplayMetadata {
                    virtual_origin: AbsoluteScreenPoint { x: 0, y: 0 },
                    virtual_size: ScreenSize {
                        width: 1920,
                        height: 1080,
                    },
                    monitors: Vec::new(),
                },
            },
            vec![
                InputEvent {
                    sequence: 0,
                    elapsed_time: ElapsedTime::from_micros(0),
                    action: InputAction::KeyPressed {
                        key: KeyDescriptor {
                            scan_code: ScanCode::new(30),
                            logical_name: Some(String::from("A")),
                            extended: false,
                        },
                    },
                },
                InputEvent {
                    sequence: 1,
                    elapsed_time: ElapsedTime::from_micros(5_000),
                    action: InputAction::PointerMoved {
                        position: PointerPosition {
                            absolute: AbsoluteScreenPoint { x: 200, y: 160 },
                            normalized: NormalizedScreenPoint {
                                x: NormalizedCoordinate::new(0.104, "x")
                                    .expect("normalized x should be valid"),
                                y: NormalizedCoordinate::new(0.148, "y")
                                    .expect("normalized y should be valid"),
                            },
                        },
                    },
                },
                InputEvent {
                    sequence: 2,
                    elapsed_time: ElapsedTime::from_micros(8_000),
                    action: InputAction::MouseButtonPressed {
                        button: MouseButton::Left,
                    },
                },
                InputEvent {
                    sequence: 3,
                    elapsed_time: ElapsedTime::from_micros(12_000),
                    action: InputAction::PointerMoved {
                        position: PointerPosition {
                            absolute: AbsoluteScreenPoint { x: 240, y: 180 },
                            normalized: NormalizedScreenPoint {
                                x: NormalizedCoordinate::new(0.125, "x")
                                    .expect("normalized x should be valid"),
                                y: NormalizedCoordinate::new(0.166, "y")
                                    .expect("normalized y should be valid"),
                            },
                        },
                    },
                },
                InputEvent {
                    sequence: 4,
                    elapsed_time: ElapsedTime::from_micros(18_000),
                    action: InputAction::MouseButtonReleased {
                        button: MouseButton::Left,
                    },
                },
            ],
        )
        .expect("mixed sample recording should be valid")
    }

    fn create_test_directory() -> PathBuf {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the unix epoch")
            .as_nanos();
        let test_directory = std::env::temp_dir().join(format!(
            "regxorder-ui-tests-{}-{unique_suffix}",
            std::process::id()
        ));

        fs::create_dir_all(&test_directory).expect("test directory should be created");
        test_directory
    }
}
