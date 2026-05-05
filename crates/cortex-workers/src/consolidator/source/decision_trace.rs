//! Phase11p §1.4 — Live decision-trace source.
//!
//! Walks `payload.parent_event_id` from a `Kind::Decision` envelope
//! through the parquet archive (via
//! [`cortex_storage::archive::scan_envelope_by_event_id`]) up to
//! [`MAX_HOPS`] hops. Cycle detection short-circuits the walk and
//! surfaces a [`SourceError::Storage`] with the offending pair.
//! Missing parents are treated as the chain root (the walk stops
//! cleanly without error).

use std::collections::HashSet;
use std::path::PathBuf;

use cortex_core::events::{Envelope, Kind};

use crate::consolidator::producer::decision_trace::{DecisionTraceInput, MAX_HOPS};

use super::SourceError;

/// Live source backed by the parquet archive.
#[derive(Debug, Clone)]
pub struct LiveDecisionTraceSource {
    archive_root: PathBuf,
}

impl LiveDecisionTraceSource {
    /// Build a new source with the configured archive root.
    pub fn new(archive_root: impl Into<PathBuf>) -> Self {
        Self {
            archive_root: archive_root.into(),
        }
    }

    /// Resolve the decision envelope + walk the parent chain.
    /// Returns [`SourceError::EmptyResult`] when the decision id
    /// itself does not match anything in the archive.
    pub fn fetch(&self, decision_event_id: &str) -> Result<DecisionTraceInput, SourceError> {
        let decision = match cortex_storage::archive::scan_envelope_by_event_id(
            &self.archive_root,
            decision_event_id,
        )? {
            Some(env) => env,
            None => return Err(SourceError::EmptyResult),
        };
        if decision.kind != Kind::Decision {
            return Err(SourceError::Storage(format!(
                "envelope {decision_event_id} is not Kind::Decision (got {:?})",
                decision.kind
            )));
        }
        let repo = decision.context.repo.clone();
        let chain = self.walk_chain(&decision)?;
        Ok(DecisionTraceInput {
            decision,
            chain,
            repo,
        })
    }

    /// Walk the parent chain from the decision back to the root
    /// (or `MAX_HOPS`, whichever comes first). Returned chain is
    /// ordered root → decision.parent (oldest first).
    fn walk_chain(&self, decision: &Envelope) -> Result<Vec<Envelope>, SourceError> {
        let mut chain: Vec<Envelope> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(decision.event_id.clone());
        let mut cursor = decision.parent_event_id.clone();
        while let Some(parent_id) = cursor {
            if chain.len() >= MAX_HOPS {
                break;
            }
            if seen.contains(&parent_id) {
                return Err(SourceError::Storage(format!(
                    "cycle detected in decision-trace walk: {} ↔ {parent_id}",
                    decision.event_id,
                )));
            }
            let parent = match cortex_storage::archive::scan_envelope_by_event_id(
                &self.archive_root,
                &parent_id,
            )? {
                Some(env) => env,
                None => break,
            };
            seen.insert(parent.event_id.clone());
            cursor = parent.parent_event_id.clone();
            chain.push(parent);
        }
        // Caller wants root → decision.parent — reverse so the
        // oldest envelope lands first.
        chain.reverse();
        Ok(chain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{Context, Stream, Turn};
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    fn envelope(
        event_id: &str,
        kind: Kind,
        parent_event_id: Option<&str>,
    ) -> Envelope {
        Envelope {
            event_id: event_id.to_string(),
            schema_version: "1".to_string(),
            occurred_at: "2026-04-26T19:00:00Z".to_string(),
            ingested_at: None,
            session_id: "S1".to_string(),
            stream: Stream::Live,
            tool: "claude-code".to_string(),
            model: None,
            kind,
            context: Context {
                repo: Some("cortex".to_string()),
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "linux".to_string(),
                ide: None,
                extras: BTreeMap::new(),
            },
            payload: serde_json::to_value(Turn {
                user_message: "x".to_string(),
                assistant_message: None,
                tokens: None,
                tool_call_event_ids: Vec::new(),
            })
            .unwrap(),
            redactions: Vec::new(),
            content_hash:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            parent_event_id: parent_event_id.map(str::to_string),
        }
    }

    fn write_archive(root: &Path, envelopes: &[Envelope]) {
        let dir = root.join("events/year=2026/month=04/day=26/hour=19");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw-00000.parquet");
        let file = File::create(&path).unwrap();
        let mut enc = zstd::stream::write::Encoder::new(file, 3).unwrap();
        for env in envelopes {
            let line = serde_json::to_string(env).unwrap();
            enc.write_all(line.as_bytes()).unwrap();
            enc.write_all(b"\n").unwrap();
        }
        enc.finish().unwrap();
    }

    #[test]
    fn fetch_single_hop_chain() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[
                envelope("PARENT", Kind::Turn, None),
                envelope("DECIDE", Kind::Decision, Some("PARENT")),
            ],
        );
        let src = LiveDecisionTraceSource::new(dir.path());
        let input = src.fetch("DECIDE").unwrap();
        assert_eq!(input.decision.event_id, "DECIDE");
        assert_eq!(input.chain.len(), 1);
        assert_eq!(input.chain[0].event_id, "PARENT");
    }

    #[test]
    fn fetch_truncates_at_max_hops() {
        let dir = tempfile::tempdir().unwrap();
        let mut envelopes = vec![envelope("ROOT", Kind::Turn, None)];
        // Build a 25-deep chain so MAX_HOPS = 16 truncates.
        for i in 0..25 {
            let id = format!("HOP{i:02}");
            let parent = if i == 0 {
                "ROOT".to_string()
            } else {
                format!("HOP{:02}", i - 1)
            };
            envelopes.push(envelope(&id, Kind::Turn, Some(&parent)));
        }
        envelopes.push(envelope("DECIDE", Kind::Decision, Some("HOP24")));
        write_archive(dir.path(), &envelopes);
        let src = LiveDecisionTraceSource::new(dir.path());
        let input = src.fetch("DECIDE").unwrap();
        assert_eq!(input.chain.len(), MAX_HOPS);
    }

    #[test]
    fn fetch_detects_cycle() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[
                // Cycle: A → B → A
                envelope("A", Kind::Turn, Some("B")),
                envelope("B", Kind::Turn, Some("A")),
                envelope("DECIDE", Kind::Decision, Some("A")),
            ],
        );
        let src = LiveDecisionTraceSource::new(dir.path());
        let err = src.fetch("DECIDE").unwrap_err();
        assert!(matches!(err, SourceError::Storage(ref s) if s.contains("cycle")));
    }

    #[test]
    fn fetch_treats_missing_parent_as_chain_root() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[
                // DECIDE points to PARENT, but PARENT is not in the
                // archive (e.g. rolled-up / dropped). Walk stops
                // cleanly at the missing edge — chain ends up empty.
                envelope("DECIDE", Kind::Decision, Some("MISSING_PARENT")),
            ],
        );
        let src = LiveDecisionTraceSource::new(dir.path());
        let input = src.fetch("DECIDE").unwrap();
        assert_eq!(input.chain.len(), 0);
        assert_eq!(input.decision.event_id, "DECIDE");
    }
}
