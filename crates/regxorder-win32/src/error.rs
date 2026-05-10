use thiserror::Error;

/// Errors surfaced by the Windows-specific recording and playback backends.
#[derive(Debug, Error)]
pub enum WindowsBackendError {
    #[error(transparent)]
    Validation(#[from] regxorder_core::ValidationError),

    #[error("playback was interrupted")]
    Interrupted,

    #[error("an internal backend invariant was violated: {0}")]
    Internal(&'static str),

    #[error("the recording thread terminated unexpectedly")]
    ThreadPanic,

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
