use slint::SharedString;

use crate::{
    EditingSession, format_elapsed_clock, format_event_table_parameter_label,
    format_event_table_type_label,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EventListFilter {
    pub(super) show_key_events: bool,
    pub(super) show_mouse_button_events: bool,
    pub(super) show_mouse_wheel_events: bool,
    pub(super) show_pointer_move_events: bool,
}

impl Default for EventListFilter {
    fn default() -> Self {
        Self {
            show_key_events: true,
            show_mouse_button_events: true,
            show_mouse_wheel_events: true,
            show_pointer_move_events: false,
        }
    }
}

impl EventListFilter {
    pub(super) fn matches(self, action: &regxorder_core::InputAction) -> bool {
        match action {
            regxorder_core::InputAction::KeyPressed { .. }
            | regxorder_core::InputAction::KeyReleased { .. } => self.show_key_events,
            regxorder_core::InputAction::PointerMoved { .. } => self.show_pointer_move_events,
            regxorder_core::InputAction::MouseButtonPressed { .. }
            | regxorder_core::InputAction::MouseButtonReleased { .. } => {
                self.show_mouse_button_events
            }
            regxorder_core::InputAction::MouseWheelScrolled { .. } => self.show_mouse_wheel_events,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VisibleEventRow {
    pub(super) source_event_index: usize,
    pub(super) index_label: String,
    pub(super) type_label: String,
    pub(super) parameter_label: String,
    pub(super) duration_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct EventFilterCounts {
    pub(super) key_event_count: usize,
    pub(super) mouse_button_event_count: usize,
    pub(super) mouse_wheel_event_count: usize,
    pub(super) pointer_move_event_count: usize,
}

pub(super) fn build_visible_event_rows(
    editing_session: Option<&EditingSession>,
    event_list_filter: EventListFilter,
) -> Vec<VisibleEventRow> {
    editing_session
        .into_iter()
        .flat_map(|editing_session| {
            editing_session
                .working_recording
                .events()
                .iter()
                .enumerate()
                .filter(|(_, event)| event_list_filter.matches(&event.action))
                .map(|(source_event_index, event)| VisibleEventRow {
                    source_event_index,
                    index_label: (source_event_index + 1).to_string(),
                    type_label: format_event_table_type_label(&event.action),
                    parameter_label: format_event_table_parameter_label(&event.action),
                    duration_label: format_elapsed_clock(event.elapsed_time.as_micros()),
                })
        })
        .collect()
}

pub(super) fn visible_event_source_indices(
    editing_session: Option<&EditingSession>,
    event_list_filter: EventListFilter,
) -> Vec<usize> {
    build_visible_event_rows(editing_session, event_list_filter)
        .into_iter()
        .map(|row| row.source_event_index)
        .collect()
}

pub(super) fn build_event_filter_counts(
    editing_session: Option<&EditingSession>,
) -> EventFilterCounts {
    let Some(editing_session) = editing_session else {
        return EventFilterCounts::default();
    };

    editing_session.working_recording.events().iter().fold(
        EventFilterCounts::default(),
        |mut event_filter_counts, event| {
            match event.action {
                regxorder_core::InputAction::KeyPressed { .. }
                | regxorder_core::InputAction::KeyReleased { .. } => {
                    event_filter_counts.key_event_count += 1;
                }
                regxorder_core::InputAction::PointerMoved { .. } => {
                    event_filter_counts.pointer_move_event_count += 1;
                }
                regxorder_core::InputAction::MouseButtonPressed { .. }
                | regxorder_core::InputAction::MouseButtonReleased { .. } => {
                    event_filter_counts.mouse_button_event_count += 1;
                }
                regxorder_core::InputAction::MouseWheelScrolled { .. } => {
                    event_filter_counts.mouse_wheel_event_count += 1;
                }
            }

            event_filter_counts
        },
    )
}

fn event_table_placeholder(
    editing_session: Option<&EditingSession>,
    visible_event_rows: &[VisibleEventRow],
) -> Option<(&'static str, &'static str)> {
    let Some(editing_session) = editing_session else {
        return Some((
            "No session",
            "Select a session from the library to inspect or edit its event stream.",
        ));
    };

    if editing_session.working_recording.events().is_empty() {
        return Some((
            "No events",
            "This session is empty. Record new input or load another session.",
        ));
    }

    if visible_event_rows.is_empty() {
        return Some((
            "No matches",
            "Mouse moves are hidden by default. Enable more filters to show additional events.",
        ));
    }

    None
}

pub(super) fn build_event_index_items(
    editing_session: Option<&EditingSession>,
    visible_event_rows: &[VisibleEventRow],
) -> Vec<SharedString> {
    if event_table_placeholder(editing_session, visible_event_rows).is_some() {
        return vec![SharedString::from("")];
    }

    visible_event_rows
        .iter()
        .map(|row| SharedString::from(row.index_label.clone()))
        .collect()
}

pub(super) fn build_event_type_items(
    editing_session: Option<&EditingSession>,
    visible_event_rows: &[VisibleEventRow],
) -> Vec<SharedString> {
    if let Some((placeholder_type, _)) =
        event_table_placeholder(editing_session, visible_event_rows)
    {
        return vec![SharedString::from(placeholder_type)];
    }

    visible_event_rows
        .iter()
        .map(|row| SharedString::from(row.type_label.clone()))
        .collect()
}

pub(super) fn build_event_parameter_items(
    editing_session: Option<&EditingSession>,
    visible_event_rows: &[VisibleEventRow],
) -> Vec<SharedString> {
    if let Some((_, placeholder_parameter)) =
        event_table_placeholder(editing_session, visible_event_rows)
    {
        return vec![SharedString::from(placeholder_parameter)];
    }

    visible_event_rows
        .iter()
        .map(|row| SharedString::from(row.parameter_label.clone()))
        .collect()
}

pub(super) fn build_event_duration_items(
    editing_session: Option<&EditingSession>,
    visible_event_rows: &[VisibleEventRow],
) -> Vec<SharedString> {
    if event_table_placeholder(editing_session, visible_event_rows).is_some() {
        return vec![SharedString::from("")];
    }

    visible_event_rows
        .iter()
        .map(|row| SharedString::from(row.duration_label.clone()))
        .collect()
}
