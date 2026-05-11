use thiserror::Error;

/// Errors surfaced by the Windows-specific recording and playback backends.
#[derive(Debug, Error)]
pub enum WindowsBackendError {
    #[error(transparent)]
    Validation(#[from] regxorder_core::ValidationError),

    #[error("at least one hotkey registration is required")]
    NoHotkeyRegistrations,

    #[error("hotkey identifier {identifier} was registered more than once")]
    DuplicateHotkeyIdentifier { identifier: i32 },

    #[error("playback was interrupted")]
    Interrupted,

    #[error("an internal backend invariant was violated: {0}")]
    Internal(&'static str),

    #[error("the recording thread terminated unexpectedly")]
    ThreadPanic,

    #[error("failed to register global hotkey {identifier} ({description}): {source}")]
    HotkeyRegistrationFailed {
        identifier: i32,
        description: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "SendInput submitted {submitted} of {requested} events; playback may be blocked by UIPI or another input filter"
    )]
    PartialSend { requested: u32, submitted: u32 },

    #[error("{context} failed: {source}")]
    Os {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },
}

impl WindowsBackendError {
    pub(crate) fn last_os_error(context: &'static str) -> Self {
        Self::Os {
            context,
            source: std::io::Error::last_os_error(),
        }
    }
}
