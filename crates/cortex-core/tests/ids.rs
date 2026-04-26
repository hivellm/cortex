//! Integration tests for `cortex_core::ids`.

use cortex_core::{event_id, EventId};
use std::collections::HashSet;

#[test]
fn event_ids_are_26_chars() {
    let id = event_id();
    assert_eq!(id.len(), 26);
}

#[test]
fn event_ids_are_unique_enough() {
    let mut seen = HashSet::new();
    for _ in 0..1000 {
        assert!(seen.insert(event_id()));
    }
}

#[test]
fn round_trip_parse() {
    let raw = event_id();
    let parsed: EventId = raw.parse().unwrap();
    assert_eq!(parsed.to_string(), raw);
}

#[test]
fn crockford_charset() {
    let id = event_id();
    for c in id.chars() {
        assert!(c.is_ascii_alphanumeric());
        assert!(!matches!(c, 'I' | 'L' | 'O' | 'U'));
    }
}
