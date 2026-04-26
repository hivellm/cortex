//! ULID generator and typed ID wrappers.
//!
//! Every entity Cortex tracks uses a 26-char Crockford base32 ULID. ULIDs are
//! lex-sortable by generation time, which makes them convenient primary keys
//! for time-ordered storage (Parquet partitioning, Synap streams, SQLite).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Re-export the crate-level ULID type for callers that want to work with it directly.
pub use ulid::Ulid;

/// ULID identifying a single event on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(pub Ulid);

impl EventId {
    /// Generate a new time-ordered ULID.
    pub fn new() -> Self {
        EventId(Ulid::new())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for EventId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(s).map(EventId)
    }
}

/// ULID identifying an AI session. Adapter-owned; stable for the life of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Ulid);

impl SessionId {
    /// Generate a new session ULID.
    pub fn new() -> Self {
        SessionId(Ulid::new())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for SessionId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(s).map(SessionId)
    }
}

/// Convenience: generate a fresh [`EventId`] as a string.
pub fn event_id() -> String {
    EventId::new().to_string()
}

/// Convenience: generate a fresh [`SessionId`] as a string.
pub fn session_id() -> String {
    SessionId::new().to_string()
}

