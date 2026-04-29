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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_id_new_and_default_match() {
        let a = EventId::new();
        let b = EventId::default();
        // Both are Ulid::new() — content differs, type matches.
        assert_eq!(a.to_string().len(), 26);
        assert_eq!(b.to_string().len(), 26);
    }

    #[test]
    fn session_id_new_and_default_match() {
        let a = SessionId::new();
        let b = SessionId::default();
        assert_eq!(a.to_string().len(), 26);
        assert_eq!(b.to_string().len(), 26);
    }

    #[test]
    fn event_id_round_trips_through_string() {
        let id = EventId::new();
        let s = id.to_string();
        let parsed: EventId = s.parse().unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn session_id_round_trips_through_string() {
        let id = SessionId::new();
        let s = id.to_string();
        let parsed: SessionId = s.parse().unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn event_id_rejects_garbage() {
        let err: Result<EventId, _> = "not-a-ulid".parse();
        assert!(err.is_err());
    }

    #[test]
    fn session_id_rejects_garbage() {
        let err: Result<SessionId, _> = "definitely_not_a_ulid".parse();
        assert!(err.is_err());
    }

    #[test]
    fn convenience_helpers_yield_distinct_ulids() {
        let a = event_id();
        let b = event_id();
        assert_eq!(a.len(), 26);
        assert_eq!(b.len(), 26);
        assert_ne!(a, b);
        let c = session_id();
        assert_eq!(c.len(), 26);
    }

    #[test]
    fn ids_are_serde_transparent() {
        let id = EventId::new();
        let json = serde_json::to_string(&id).unwrap();
        // Transparent serde — the JSON is the bare ULID string, not
        // a wrapper object.
        assert!(json.starts_with('"'));
        assert!(!json.contains("EventId"));
        let back: EventId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);

        let sid = SessionId::new();
        let sjson = serde_json::to_string(&sid).unwrap();
        assert!(!sjson.contains("SessionId"));
        let sback: SessionId = serde_json::from_str(&sjson).unwrap();
        assert_eq!(sback, sid);
    }
}

