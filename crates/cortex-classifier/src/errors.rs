//! Classifier error type.

/// Error returned by [`crate::Classifier`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum ClassifierError {
    /// Haiku / backend invocation failure.
    #[error("backend: {0}")]
    Backend(String),
    /// JSON (de)serialization failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Cache failure.
    #[error("cache: {0}")]
    Cache(String),
    /// Output validation failure.
    #[error("validation: {0}")]
    Validation(String),
    /// I/O error (CLI spawn, file read).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Budget tracker said stop.
    #[error("budget halted: {0}")]
    BudgetHalt(String),
    /// Output length did not match input length.
    #[error("output length mismatch: expected {expected}, got {actual}")]
    LengthMismatch {
        /// Input batch size.
        expected: usize,
        /// Records returned.
        actual: usize,
    },
}
