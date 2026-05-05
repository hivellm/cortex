//! Phase11p §1.2 — Live session source.
//!
//! Reads every envelope under a single `session_id` from the
//! parquet archive (via
//! [`cortex_storage::archive::scan_envelopes_by_session`]), derives
//! the owning repo from the majority-vote of `context.repo` across
//! the matched envelopes, and returns a producer-facing
//! [`crate::consolidator::producer::session::SessionInput`].

use std::collections::HashMap;
use std::path::PathBuf;

use cortex_core::events::Envelope;

use crate::consolidator::producer::session::SessionInput;

use super::SourceError;

/// Live source backed by the parquet archive.
#[derive(Debug, Clone)]
pub struct LiveSessionSource {
    archive_root: PathBuf,
}

impl LiveSessionSource {
    /// Build a new source with the configured archive root. The
    /// caller is expected to thread `CORTEX_ARCHIVE_ROOT` (or the
    /// `--archive-root` flag) through here.
    pub fn new(archive_root: impl Into<PathBuf>) -> Self {
        Self {
            archive_root: archive_root.into(),
        }
    }

    /// Materialise a session's envelope set + derive the owning
    /// repo. Returns [`SourceError::EmptyResult`] when the archive
    /// has zero envelopes for the id (the bin treats this as a
    /// clean exit-0 path).
    pub fn fetch(&self, session_id: &str) -> Result<SessionInput, SourceError> {
        let envelopes = cortex_storage::archive::scan_envelopes_by_session(
            &self.archive_root,
            session_id,
        )?;
        if envelopes.is_empty() {
            return Err(SourceError::EmptyResult);
        }
        let repo = majority_repo(&envelopes);
        Ok(SessionInput {
            session_id: session_id.to_string(),
            repo,
            envelopes,
        })
    }
}

/// Derive the repo slug a multi-envelope session is "about" by
/// majority-voting `context.repo` across the matched envelopes.
/// Empty / repo-less sessions return `None`. Tie-break: lexicographic
/// order of the repo strings (deterministic across runs).
fn majority_repo(envelopes: &[Envelope]) -> Option<String> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for env in envelopes {
        if let Some(repo) = env.context.repo.as_deref() {
            if !repo.is_empty() {
                *counts.entry(repo.to_string()).or_insert(0) += 1;
            }
        }
    }
    if counts.is_empty() {
        return None;
    }
    let mut best: Option<(String, u32)> = None;
    for (repo, n) in counts {
        match &best {
            None => best = Some((repo, n)),
            Some((cur, cur_n)) => {
                if n > *cur_n || (n == *cur_n && repo < *cur) {
                    best = Some((repo, n));
                }
            }
        }
    }
    best.map(|(r, _)| r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{Context, Kind, Stream, Turn};
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    fn envelope_with_repo(
        event_id: &str,
        session_id: &str,
        occurred_at: &str,
        repo: Option<&str>,
    ) -> Envelope {
        Envelope {
            event_id: event_id.to_string(),
            schema_version: "1".to_string(),
            occurred_at: occurred_at.to_string(),
            ingested_at: None,
            session_id: session_id.to_string(),
            stream: Stream::Live,
            tool: "claude-code".to_string(),
            model: None,
            kind: Kind::Turn,
            context: Context {
                repo: repo.map(str::to_string),
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
            parent_event_id: None,
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
    fn fetch_happy_path_returns_session_input() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[
                envelope_with_repo("E1", "S1", "2026-04-26T19:00:00Z", Some("cortex")),
                envelope_with_repo("E2", "S1", "2026-04-26T19:01:00Z", Some("cortex")),
            ],
        );
        let src = LiveSessionSource::new(dir.path());
        let input = src.fetch("S1").unwrap();
        assert_eq!(input.session_id, "S1");
        assert_eq!(input.repo.as_deref(), Some("cortex"));
        assert_eq!(input.envelopes.len(), 2);
    }

    #[test]
    fn fetch_empty_session_returns_empty_result_error() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[envelope_with_repo("E1", "OTHER", "2026-04-26T19:00:00Z", Some("cortex"))],
        );
        let src = LiveSessionSource::new(dir.path());
        let err = src.fetch("MISSING").unwrap_err();
        assert!(matches!(err, SourceError::EmptyResult));
    }

    #[test]
    fn fetch_sorts_envelopes_by_occurred_at() {
        let dir = tempfile::tempdir().unwrap();
        // Out-of-order on disk: T+5, T+0, T+2.
        write_archive(
            dir.path(),
            &[
                envelope_with_repo("E5", "S1", "2026-04-26T19:05:00Z", Some("cortex")),
                envelope_with_repo("E0", "S1", "2026-04-26T19:00:00Z", Some("cortex")),
                envelope_with_repo("E2", "S1", "2026-04-26T19:02:00Z", Some("cortex")),
            ],
        );
        let src = LiveSessionSource::new(dir.path());
        let input = src.fetch("S1").unwrap();
        let ids: Vec<&str> = input.envelopes.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids, vec!["E0", "E2", "E5"]);
    }

    #[test]
    fn fetch_multi_repo_session_picks_majority() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[
                envelope_with_repo("E1", "S1", "2026-04-26T19:00:00Z", Some("cortex")),
                envelope_with_repo("E2", "S1", "2026-04-26T19:01:00Z", Some("cortex")),
                envelope_with_repo("E3", "S1", "2026-04-26T19:02:00Z", Some("cortex")),
                envelope_with_repo("E4", "S1", "2026-04-26T19:03:00Z", Some("nexus")),
                envelope_with_repo("E5", "S1", "2026-04-26T19:04:00Z", Some("nexus")),
            ],
        );
        let src = LiveSessionSource::new(dir.path());
        let input = src.fetch("S1").unwrap();
        assert_eq!(input.repo.as_deref(), Some("cortex"));
    }

    #[test]
    fn fetch_repo_less_session_returns_none_repo() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[envelope_with_repo("E1", "S1", "2026-04-26T19:00:00Z", None)],
        );
        let src = LiveSessionSource::new(dir.path());
        let input = src.fetch("S1").unwrap();
        assert!(input.repo.is_none());
    }
}
