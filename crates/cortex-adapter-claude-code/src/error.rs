//! Phase14i §1.3 — adapter-wide structured error type.
//!
//! Replaces the legacy `unwrap()` / `expect()` panics in the
//! production paths of `wal.rs`, `publisher.rs`, and `install.rs`.
//! The dispatcher catches every [`AdapterError`], logs at ERROR
//! with full context, and keeps serving the next hook so a single
//! malformed payload cannot take the entire user-session capture
//! down.

use std::fmt;

/// Structured adapter failure surfaced to the daemon's dispatch
/// loop. Every variant carries enough context for the operator
/// to localise the failing call site without scraping logs.
#[derive(Debug)]
pub enum AdapterError {
    /// Incoming `HookFrame` failed validation (bad JSON shape,
    /// unknown discriminator, etc.). Carries a short, low-
    /// cardinality reason label suitable for metric counters.
    MalformedHook(String),
    /// A required envelope field was missing or empty.
    MissingField(&'static str),
    /// Writing to the IPC transport (named pipe on Windows, Unix
    /// domain socket on Linux/macOS) failed.
    IpcWriteFailed(String),
    /// Building the canonical envelope (publisher input or sync
    /// path output) failed.
    EnvelopeBuildFailed(String),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedHook(reason) => write!(f, "malformed hook: {reason}"),
            Self::MissingField(field) => write!(f, "missing required field: {field}"),
            Self::IpcWriteFailed(reason) => write!(f, "ipc write failed: {reason}"),
            Self::EnvelopeBuildFailed(reason) => write!(f, "envelope build failed: {reason}"),
        }
    }
}

impl std::error::Error for AdapterError {}

impl AdapterError {
    /// Short label suitable for metric counters
    /// (`adapter_dispatch_errors_total{reason}`).
    pub fn reason_label(&self) -> &'static str {
        match self {
            Self::MalformedHook(_) => "malformed_hook",
            Self::MissingField(_) => "missing_field",
            Self::IpcWriteFailed(_) => "ipc_write_failed",
            Self::EnvelopeBuildFailed(_) => "envelope_build_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_renders_each_variant_with_a_distinct_prefix() {
        assert_eq!(
            AdapterError::MalformedHook("bad json".into()).to_string(),
            "malformed hook: bad json"
        );
        assert_eq!(
            AdapterError::MissingField("session_id").to_string(),
            "missing required field: session_id"
        );
        assert_eq!(
            AdapterError::IpcWriteFailed("pipe closed".into()).to_string(),
            "ipc write failed: pipe closed"
        );
        assert_eq!(
            AdapterError::EnvelopeBuildFailed("serde: invalid type".into()).to_string(),
            "envelope build failed: serde: invalid type"
        );
    }

    #[test]
    fn reason_label_is_stable_per_variant() {
        assert_eq!(
            AdapterError::MalformedHook("x".into()).reason_label(),
            "malformed_hook"
        );
        assert_eq!(
            AdapterError::MissingField("k").reason_label(),
            "missing_field"
        );
        assert_eq!(
            AdapterError::IpcWriteFailed("x".into()).reason_label(),
            "ipc_write_failed"
        );
        assert_eq!(
            AdapterError::EnvelopeBuildFailed("x".into()).reason_label(),
            "envelope_build_failed"
        );
    }

    #[test]
    fn error_trait_is_implemented_so_anyhow_can_wrap_it() {
        let err: Box<dyn std::error::Error> =
            Box::new(AdapterError::MissingField("turn_id"));
        assert_eq!(err.to_string(), "missing required field: turn_id");
    }
}
