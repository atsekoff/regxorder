use thiserror::Error;

/// Errors surfaced by recording import, export, and validation operations.
#[derive(Debug, Error)]
pub enum RecordingError {
    #[error("recording payload could not be parsed as JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

/// Semantic validation errors for the regxorder core recording model.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum ValidationError {
    #[error("schema version {found} is not supported; expected {expected} for this build")]
    UnsupportedSchemaVersion { expected: u32, found: u32 },

    #[error("recording title must not be empty when provided")]
    EmptyTitle,

    #[error("{context} size must be non-zero, found {width}x{height}")]
    ZeroSizedSurface {
        context: &'static str,
        width: u32,
        height: u32,
    },

    #[error("event sequence numbers must be consecutive; expected {expected}, found {found}")]
    NonConsecutiveSequence { expected: u64, found: u64 },

    #[error(
        "event elapsed times must be monotonic; previous {previous_micros}us, found {found_micros}us"
    )]
    NonMonotonicElapsedTime {
        previous_micros: u64,
        found_micros: u64,
    },

    #[error("speed multiplier must be finite and greater than zero, found {value}")]
    InvalidSpeedMultiplier { value: f64 },

    #[error(
        "normalized coordinate {axis} must be finite and between 0.0 and 1.0 inclusive, found {value}"
    )]
    InvalidNormalizedCoordinate { axis: &'static str, value: f64 },
}
