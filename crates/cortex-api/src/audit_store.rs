//! Phase10j — in-memory store of recent audit envelopes keyed by
//! `query_id` so the new `cortex_audit` MCP tool (and the
//! `GET /v1/audit/{query_id}` HTTP endpoint that backs it) can return
//! the envelope synchronously.
//!
//! The store is fire-and-forget: every successful pipeline run pushes
//! its envelope here in addition to the existing
//! [`AuditPublisher`](crate::audit::AuditPublisher) wire. The two
//! sinks are independent — losing the bus publisher (synap down) does
//! not lose the in-memory envelope, and vice versa.
//!
//! The bound (default 1024) is the trade-off the spec calls out: an
//! agent debugging "why did this turn surface zero snippets?" needs
//! the envelope only seconds-to-minutes after the call. A ring
//! buffer that fits the last N requests is enough for that workflow,
//! and bounded memory keeps the daemon honest under sustained load.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::Value;

/// Default number of envelopes the in-memory store keeps. A turn
/// produces ~1 envelope, so 1024 covers ~30 min of heavy
/// pre-thinking traffic.
pub const DEFAULT_STORE_CAPACITY: usize = 1024;

/// In-memory audit store. Push on every publish, look up by
/// `query_id`. The eviction strategy is FIFO so the most recent
/// envelopes always survive — the reverse of what an LRU would
/// give, which is what we want here (debugging a fresh turn matters
/// more than re-reading an hour-old one).
pub struct AuditStore {
    inner: Mutex<AuditStoreInner>,
}

struct AuditStoreInner {
    capacity: usize,
    entries: VecDeque<(String, Value)>,
}

impl AuditStore {
    /// Build a store with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_STORE_CAPACITY)
    }

    /// Build a store with an explicit capacity (mostly for tests).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(AuditStoreInner {
                capacity: capacity.max(1),
                entries: VecDeque::with_capacity(capacity.max(1)),
            }),
        }
    }

    /// Record one envelope. Pulls the `query_id` out of the JSON; an
    /// envelope without one is dropped silently — there is nothing
    /// the audit endpoint could look it up under.
    pub fn record(&self, envelope: &Value) {
        let qid = match envelope.get("query_id").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return,
        };
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if g.entries.len() >= g.capacity {
            g.entries.pop_front();
        }
        g.entries.push_back((qid, envelope.clone()));
    }

    /// Look one envelope up by `query_id`. Latest match wins when the
    /// same id is recorded twice (cache hits republish their
    /// envelope).
    pub fn get(&self, query_id: &str) -> Option<Value> {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.entries
            .iter()
            .rev()
            .find(|(q, _)| q == query_id)
            .map(|(_, env)| env.clone())
    }

    /// Number of envelopes currently retained.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.entries.len()).unwrap_or(0)
    }

    /// `true` when no envelope is retained.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for AuditStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn record_and_get_round_trips_envelope() {
        let store = AuditStore::with_capacity(8);
        let env = json!({
            "kind": "query_audit",
            "query_id": "01KQDX",
            "intent": "free_search",
        });
        store.record(&env);
        let got = store
            .get("01KQDX")
            .expect("envelope retrievable by query_id");
        assert_eq!(got["intent"], "free_search");
    }

    #[test]
    fn fifo_evicts_oldest_when_capacity_exceeded() {
        let store = AuditStore::with_capacity(2);
        store.record(&json!({ "query_id": "a" }));
        store.record(&json!({ "query_id": "b" }));
        store.record(&json!({ "query_id": "c" }));
        assert!(store.get("a").is_none(), "oldest should be evicted");
        assert!(store.get("b").is_some());
        assert!(store.get("c").is_some());
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn missing_query_id_is_dropped_silently() {
        let store = AuditStore::new();
        store.record(&json!({ "kind": "query_audit" }));
        assert!(store.is_empty());
    }

    #[test]
    fn duplicate_query_id_returns_latest() {
        let store = AuditStore::new();
        store.record(&json!({ "query_id": "x", "intent": "first" }));
        store.record(&json!({ "query_id": "x", "intent": "second" }));
        let got = store.get("x").unwrap();
        assert_eq!(got["intent"], "second");
    }
}
