//! Archive → in-memory keyword lane bootstrap.
//!
//! `cortex-ingestion` writes one zstd-compressed NDJSON file per
//! hour per stream tag under `<archive_root>/events/year=YYYY/month=MM/day=DD/hour=HH/raw-NNNNN.parquet`
//! (the `.parquet` suffix is a historical naming choice; the bytes
//! are zstd-compressed line-delimited JSON, *not* Apache Parquet).
//!
//! Until the spec-06 / spec-07 / spec-08 indexer pipeline is wired
//! to live Vectorizer / Meilisearch / Nexus, this loader gives
//! `cortex-api` something to query against: at boot time, scan the
//! archive root, parse every canonical envelope, and seed the
//! [`MemoryKeywordLane`] so `/v1/query` returns the user's actual
//! captured prompts and tool calls.
//!
//! This is a deliberately small module — pragma covering the
//! "captured events are queryable" gap. Live indexers replace it
//! when they ship.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use cortex_core::events::{AgentCall, Envelope, Kind, ToolCall, Turn};
use thiserror::Error;

use crate::lanes::{LaneHit, MemoryKeywordLane};

/// Failure modes raised by the loader. The caller treats every
/// variant as "skip and keep going" — a corrupt archive should
/// never block boot.
#[derive(Debug, Error)]
pub enum LoadError {
    /// I/O while reading or decompressing.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Walked into a malformed zstd file.
    #[error("zstd: {0}")]
    Zstd(String),
}

/// Outcome of a load run — useful for tests + the boot-time log
/// line.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    /// Number of `.parquet` files visited.
    pub files_visited: usize,
    /// Number of envelopes successfully parsed.
    pub envelopes_parsed: usize,
    /// Hits seeded into the keyword lane (one per `Turn` /
    /// `ToolCall` / `AgentCall` envelope).
    pub hits_seeded: usize,
    /// Lines that didn't deserialize as a canonical envelope.
    pub lines_dropped: usize,
    /// Files whose zstd stream was truly corrupt (not just a live
    /// trailing frame). Surfaced separately so the health probe at
    /// `/healthz` can flag silent data loss after a hard-killed
    /// `cortex-ingestion`. See
    /// [docs/analysis/adapter/01-tool-call-archive-loss.md](../../../../docs/analysis/adapter/01-tool-call-archive-loss.md).
    pub corrupted_files: Vec<std::path::PathBuf>,
}

/// Default index name the keyword lane is seeded under. Matches the
/// `cortex-code` index the spec-11 strategies hit for free-search /
/// pre-change-context queries.
pub const DEFAULT_INDEX: &str = "cortex-code";

/// Walk `archive_root` recursively, parse every canonical envelope,
/// and return a flat `(report, hits)` pair. The caller seeds the
/// keyword lane explicitly so tests can drive the loader without
/// touching live state.
pub fn load_lane_hits(archive_root: &Path) -> (LoadReport, Vec<LaneHit>) {
    let mut report = LoadReport::default();
    let mut hits: Vec<LaneHit> = Vec::new();
    visit_dir(archive_root, &mut report, &mut hits);
    (report, hits)
}

/// One-call helper that loads + seeds the lane under [`DEFAULT_INDEX`].
/// Returns the report so the caller can log it.
pub fn load_into_keyword_lane(archive_root: &Path, lane: &MemoryKeywordLane) -> LoadReport {
    load_into_keyword_lane_with_metrics(archive_root, lane, None)
}

/// Phase8b — same as [`load_into_keyword_lane`] but also stamps the
/// per-kind seed counters and `last_refresh_ts_ms` on a shared
/// [`crate::LoaderMetrics`] registry. The freshness aggregator reads
/// these to flag a stalled loader.
pub fn load_into_keyword_lane_with_metrics(
    archive_root: &Path,
    lane: &MemoryKeywordLane,
    metrics: Option<&crate::LoaderMetrics>,
) -> LoadReport {
    let (report, hits) = load_lane_hits(archive_root);
    if !hits.is_empty() {
        // Seed the indexes the spec-11 strategies hit for free-search
        // and pre-change-context. The MemoryKeywordLane returns all
        // seeded hits regardless of `query`, so the same hit set
        // surfaces under every alias.
        for index in ["cortex-code", "cortex-docs", "cortex-decisions"] {
            lane.seed(index, hits.clone());
        }
    }
    if let Some(m) = metrics {
        // Phase8b — stamp the per-kind cumulative seed count by
        // walking the hits exactly once. The lane is replaced on
        // every refresh, so the counter is "envelopes ever seen by
        // the loader" rather than "currently in the lane" — which
        // keeps it monotonic.
        let mut by_kind: std::collections::BTreeMap<&str, u64> =
            std::collections::BTreeMap::new();
        for hit in &hits {
            let kind = hit
                .symbol
                .as_deref()
                .map(|sym| {
                    if sym.starts_with("tool_call") {
                        "tool_call"
                    } else if sym.starts_with("agent_call") {
                        "agent_call"
                    } else if sym == "turn" {
                        "turn"
                    } else {
                        "other"
                    }
                })
                .unwrap_or("other");
            *by_kind.entry(kind).or_insert(0) += 1;
        }
        for (kind, n) in by_kind {
            m.add_archive_envelopes_seeded(kind, n);
        }
        m.record_archive_refresh_now();
    }
    report
}

fn visit_dir(dir: &Path, report: &mut LoadReport, hits: &mut Vec<LaneHit>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_dir(&path, report, hits);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
            continue;
        }
        report.files_visited += 1;
        let envelopes_before = report.envelopes_parsed;
        if let Err(e) = read_one_file(&path, report, hits) {
            let recovered = report.envelopes_parsed - envelopes_before;
            // Two distinct shapes here:
            //   * `incomplete frame` — cortex-ingestion holds the
            //     current-hour file open, so the trailing bytes are
            //     a half-flushed zstd frame. Expected noise on every
            //     refresh; log at DEBUG so steady-state stays quiet.
            //   * `Data corruption detected` — the leading bytes of
            //     the file are a broken zstd stream (typically left
            //     by a hard-killed previous run, see
            //     docs/analysis/adapter/01-tool-call-archive-loss.md).
            //     The reader can't cross the boundary, so every
            //     envelope appended after the broken frame is
            //     **silently invisible** to the dashboard until the
            //     operator rotates the file. Promote to WARN so the
            //     incident surfaces immediately.
            let err_str = e.to_string();
            let truly_corrupt = err_str.contains("Data corruption")
                || err_str.contains("Unknown frame");
            if truly_corrupt {
                report.corrupted_files.push(path.clone());
                tracing::warn!(
                    path = %path.display(),
                    recovered_envelopes = recovered,
                    error = %e,
                    "archive loader: corrupt zstd stream — \
                     events appended after the corruption boundary \
                     are invisible. Quarantine the file and \
                     rotate per docs/analysis/adapter/01-tool-call-archive-loss.md."
                );
            } else if recovered > 0 {
                tracing::debug!(
                    path = %path.display(),
                    recovered_envelopes = recovered,
                    error = %e,
                    "archive loader: partial frame (live file)"
                );
            } else {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "archive loader: file unreadable, no envelopes recovered"
                );
                report.corrupted_files.push(path.clone());
            }
        }
    }
}

fn read_one_file(
    path: &PathBuf,
    report: &mut LoadReport,
    hits: &mut Vec<LaneHit>,
) -> Result<(), LoadError> {
    let file = File::open(path)?;
    let decoder = zstd::stream::read::Decoder::new(file)
        .map_err(|e| LoadError::Zstd(e.to_string()))?;
    let reader = BufReader::new(decoder);
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Envelope>(trimmed) {
            Ok(env) => {
                report.envelopes_parsed += 1;
                if let Some(hit) = envelope_to_hit(&env) {
                    hits.push(hit);
                    report.hits_seeded += 1;
                }
            }
            Err(_) => {
                report.lines_dropped += 1;
            }
        }
    }
    Ok(())
}

/// Map a canonical envelope to a [`LaneHit`]. Returns `None` for
/// kinds that don't carry queryable text.
fn envelope_to_hit(env: &Envelope) -> Option<LaneHit> {
    // Captured for the Turn branch below — extras stamping needs
    // both halves separately so the dashboard's conversation
    // pairing can distinguish the user/Stop envelopes by turn_id.
    let mut turn_user_message: Option<String> = None;
    let mut turn_assistant_message: Option<String> = None;
    // ToolCall + AgentCall payloads carry an optional `duration_ms`.
    // Captured here so the dashboard's per-minute P95 helper can
    // reach it via `extras["duration_ms"]` without re-decoding the
    // payload — phase2g §1.3 (`pre_thinking_p95_ms` series).
    let mut duration_ms_payload: Option<u64> = None;

    let (text, symbol) = match env.kind {
        Kind::Turn => {
            let turn: Turn = serde_json::from_value(env.payload.clone()).ok()?;
            let asst_owned = turn.assistant_message.clone();
            // Stop envelopes carry `user_message = ""` + `assistant_message = Some(<text>)`;
            // the prior `format!("{}\n\n{asst}", "")` produced `"\n\n<text>"`, which made
            // the Live timeline row appear with an empty title (the `lines().next()` in
            // `title_from_hit` returns the first empty line) and a blank-leading detail.
            // Mirror the meili_loader's three-way branch so each side prints clean.
            let text = match (turn.user_message.as_str(), asst_owned.as_deref()) {
                (u, Some(a)) if !u.is_empty() && !a.is_empty() => format!("{u}\n\n{a}"),
                (u, _) if !u.is_empty() => u.to_string(),
                (_, Some(a)) if !a.is_empty() => a.to_string(),
                _ => String::new(),
            };
            turn_user_message = Some(turn.user_message);
            turn_assistant_message = asst_owned;
            (text, Some("turn".to_string()))
        }
        Kind::ToolCall => {
            let tc: ToolCall = serde_json::from_value(env.payload.clone()).ok()?;
            duration_ms_payload = tc.duration_ms;
            let mut buf = format!("[{}] {}", tc.tool_name, summarize_value(&tc.input, 320));
            if let Some(ref out) = tc.output {
                if let Some(stdout) = out.stdout.as_deref() {
                    if !stdout.is_empty() {
                        let snippet = clip(stdout, 320);
                        buf.push_str("\n\n");
                        buf.push_str(snippet);
                    }
                }
            }
            (buf, Some(format!("tool_call:{}", tc.tool_name)))
        }
        Kind::AgentCall => {
            let ac: AgentCall = serde_json::from_value(env.payload.clone()).ok()?;
            duration_ms_payload = ac.duration_ms;
            let text = format!(
                "[agent:{}] {}",
                ac.agent_type,
                if ac.description.is_empty() {
                    ac.prompt.as_deref().unwrap_or("(no description)").to_string()
                } else {
                    ac.description.clone()
                }
            );
            (text, Some(format!("agent_call:{}", ac.agent_type)))
        }
        _ => return None,
    };

    // Stamp session_id on extras so the dashboard endpoints can
    // group / filter by session without re-reading the archive.
    // Also stamp `source = "keyword"` so the orchestrator's
    // `lane_label()` (extras["source"] → "keyword") tags the hit
    // honestly when the MemoryKeywordLane fallback is active.
    let mut extras = std::collections::BTreeMap::new();
    extras.insert(
        "session_id".to_string(),
        serde_json::Value::String(env.session_id.clone()),
    );
    extras.insert(
        "source".to_string(),
        serde_json::Value::String("keyword".to_string()),
    );
    // Also stamp the adapter's `turn_id` on extras when present —
    // the dashboard's conversation detail pairs UserPromptSubmit
    // (user_message) with Stop (assistant_message) by this id, and
    // the meili_loader carries it forward separately.
    if let Some(cc) = env
        .context
        .extras
        .get("claude_code")
        .and_then(|v| v.as_object())
    {
        if let Some(tid) = cc.get("turn_id").and_then(|v| v.as_str()) {
            let mut cc_obj = serde_json::Map::new();
            cc_obj.insert(
                "turn_id".to_string(),
                serde_json::Value::String(tid.to_string()),
            );
            extras.insert(
                "claude_code".to_string(),
                serde_json::Value::Object(cc_obj),
            );
        }
    }
    // Surface the turn payload halves so the conversation detail
    // handler can distinguish UserPromptSubmit vs Stop envelopes.
    if let Some(u) = turn_user_message {
        extras.insert("user_message".to_string(), serde_json::Value::String(u));
    }
    if let Some(a) = turn_assistant_message {
        extras.insert(
            "assistant_message".to_string(),
            serde_json::Value::String(a),
        );
    }
    // Stamp wall-clock duration on extras when the source payload
    // carried one. The dashboard's `pre_thinking_p95_ms` series
    // reads it back without round-tripping through `payload`. Turn
    // envelopes don't carry their own duration today; the proxy is
    // ToolCall + AgentCall durations which are the closest signal
    // the lane has until spec-12 lands turn-level latency.
    if let Some(d) = duration_ms_payload {
        extras.insert(
            "duration_ms".to_string(),
            serde_json::Value::Number(serde_json::Number::from(d)),
        );
    }

    Some(LaneHit {
        doc_id: format!("archive|{}", env.event_id),
        text,
        repo: env.context.repo.clone(),
        path: env.context.cwd.clone(),
        symbol,
        content_hash: Some(env.content_hash.clone()),
        // Constant score until a real ranker lands. The orchestrator
        // applies RRF on top of lane order so seeding insertion-order
        // is enough for the captured-events-are-queryable contract.
        score: 1.0,
        ts: parse_rfc3339_to_ms(&env.occurred_at).unwrap_or(0),
        severity: None,
        extras,
    })
}

fn parse_rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

fn summarize_value(v: &serde_json::Value, max: usize) -> String {
    let raw = serde_json::to_string(v).unwrap_or_default();
    clip(&raw, max).to_string()
}

fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{Context, Stream};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn write_archive_file(root: &Path, envelopes: &[Envelope]) -> PathBuf {
        let dir = root.join("events/year=2026/month=04/day=26/hour=19");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw-00000.parquet");
        let file = File::create(&path).unwrap();
        let mut enc = zstd::stream::write::Encoder::new(file, 3).unwrap();
        use std::io::Write;
        for env in envelopes {
            let line = serde_json::to_string(env).unwrap();
            enc.write_all(line.as_bytes()).unwrap();
            enc.write_all(b"\n").unwrap();
        }
        enc.finish().unwrap();
        path
    }

    fn turn_envelope(user_message: &str) -> Envelope {
        Envelope {
            event_id: "01ABCDEFGHJKMNPQRSTVWXYZ12".to_string(),
            schema_version: "1".to_string(),
            occurred_at: "2026-04-26T19:04:11.000Z".to_string(),
            ingested_at: None,
            session_id: "01TURN0SESSION00000000000Z".to_string(),
            stream: Stream::Live,
            tool: "claude-code".to_string(),
            model: None,
            kind: Kind::Turn,
            context: Context {
                repo: Some("Cortex".to_string()),
                branch: None,
                commit: None,
                cwd: Some("e:/HiveLLM/Cortex".to_string()),
                user: None,
                platform: "win32".to_string(),
                ide: Some("claude-code".to_string()),
                extras: BTreeMap::new(),
            },
            payload: serde_json::to_value(Turn {
                user_message: user_message.to_string(),
                assistant_message: None,
                tokens: None,
                tool_call_event_ids: Vec::new(),
            })
            .unwrap(),
            redactions: Vec::new(),
            content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            parent_event_id: None,
        }
    }

    #[test]
    fn loads_canonical_envelopes_into_lane_hits() {
        let dir = tempfile::tempdir().unwrap();
        write_archive_file(
            dir.path(),
            &[
                turn_envelope("how does ef_search work?"),
                turn_envelope("explain RRF fusion"),
            ],
        );
        let (report, hits) = load_lane_hits(dir.path());
        assert_eq!(report.files_visited, 1);
        assert_eq!(report.envelopes_parsed, 2);
        assert_eq!(report.hits_seeded, 2);
        assert_eq!(report.lines_dropped, 0);
        assert_eq!(hits.len(), 2);
        assert!(hits[0].text.contains("ef_search"));
        assert_eq!(hits[0].repo.as_deref(), Some("Cortex"));
        assert!(hits[0].doc_id.starts_with("archive|"));
    }

    /// Stop-hook turn envelopes carry `user_message = ""` and the
    /// assistant's reply on `assistant_message`. The dashboard timeline
    /// row's title is derived from `text.lines().next()`, so the prior
    /// `format!("{}\n\n{asst}", "")` produced a leading empty line and
    /// the row appeared blank — what the user reported on 2026-04-28
    /// ("AI responses stopped showing in Live timeline").
    #[test]
    fn stop_envelope_text_starts_with_assistant_message_not_blank_line() {
        let mut env = turn_envelope("ignored — overwritten by Stop payload below");
        env.event_id = "01STOP000000000000000000Z2".to_string();
        env.payload = serde_json::to_value(Turn {
            user_message: String::new(),
            assistant_message: Some("the model's reply".to_string()),
            tokens: None,
            tool_call_event_ids: Vec::new(),
        })
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_archive_file(dir.path(), &[env]);
        let (report, hits) = load_lane_hits(dir.path());
        assert_eq!(report.hits_seeded, 1);
        assert_eq!(hits.len(), 1);
        let text = &hits[0].text;
        assert!(
            !text.starts_with('\n'),
            "Stop turn text must not start with a newline (title would render blank): {text:?}"
        );
        assert_eq!(text, "the model's reply");
    }

    #[test]
    fn skips_corrupt_files_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("events/year=2026/month=04/day=26/hour=00/raw-00000.parquet");
        std::fs::create_dir_all(bad.parent().unwrap()).unwrap();
        std::fs::write(&bad, b"not zstd").unwrap();
        let (report, hits) = load_lane_hits(dir.path());
        assert_eq!(report.files_visited, 1);
        assert_eq!(report.envelopes_parsed, 0);
        assert_eq!(hits.len(), 0);
    }

    #[test]
    fn empty_archive_root_returns_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let (report, hits) = load_lane_hits(dir.path());
        assert_eq!(report.files_visited, 0);
        assert_eq!(report.envelopes_parsed, 0);
        assert_eq!(hits.len(), 0);
    }

    #[test]
    fn nonexistent_root_returns_zeros() {
        let (report, hits) = load_lane_hits(Path::new("/this/path/does/not/exist"));
        assert_eq!(report.files_visited, 0);
        assert_eq!(report.envelopes_parsed, 0);
        assert_eq!(hits.len(), 0);
    }

    #[test]
    fn re_seed_after_archive_grows_replaces_lane_contents() {
        let dir = tempfile::tempdir().unwrap();
        let lane = MemoryKeywordLane::new();

        // First scan: one envelope.
        write_archive_file(dir.path(), &[turn_envelope("v1 only")]);
        let r1 = load_into_keyword_lane(dir.path(), &lane);
        assert_eq!(r1.hits_seeded, 1);
        let hits1 = lane.hits.lock().unwrap()["cortex-code"].len();
        drop(lane.hits.lock().unwrap()); // release mutex
        assert_eq!(hits1, 1);

        // Second hour rolls over with two more envelopes; the loader
        // walks both directories and replaces the lane contents.
        let dir2 =
            dir.path().join("events/year=2026/month=04/day=26/hour=20");
        std::fs::create_dir_all(&dir2).unwrap();
        let path2 = dir2.join("raw-00000.parquet");
        let file2 = File::create(&path2).unwrap();
        let mut enc = zstd::stream::write::Encoder::new(file2, 3).unwrap();
        use std::io::Write;
        for prompt in ["v2 first", "v2 second"] {
            let env = turn_envelope(prompt);
            enc.write_all(serde_json::to_string(&env).unwrap().as_bytes())
                .unwrap();
            enc.write_all(b"\n").unwrap();
        }
        enc.finish().unwrap();

        let r2 = load_into_keyword_lane(dir.path(), &lane);
        assert_eq!(r2.files_visited, 2);
        assert_eq!(r2.hits_seeded, 3);
        let hits2 = lane.hits.lock().unwrap();
        let cortex_code = hits2.get("cortex-code").unwrap();
        assert_eq!(cortex_code.len(), 3);
        let texts: Vec<_> = cortex_code.iter().map(|h| h.text.as_str()).collect();
        assert!(texts.contains(&"v1 only"));
        assert!(texts.contains(&"v2 first"));
        assert!(texts.contains(&"v2 second"));
    }

    #[test]
    fn seeds_keyword_lane_under_canonical_indexes() {
        let dir = tempfile::tempdir().unwrap();
        write_archive_file(dir.path(), &[turn_envelope("seed me")]);
        let lane = MemoryKeywordLane::new();
        let report = load_into_keyword_lane(dir.path(), &lane);
        assert_eq!(report.hits_seeded, 1);
        let hits = lane.hits.lock().unwrap();
        assert!(hits.contains_key("cortex-code"));
        assert!(hits.contains_key("cortex-docs"));
        assert!(hits.contains_key("cortex-decisions"));
        assert_eq!(hits["cortex-code"].len(), 1);
    }

    #[test]
    fn drops_lines_that_dont_deserialize_as_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events/year=2026/month=04/day=26/hour=19/raw-00000.parquet");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = File::create(&path).unwrap();
        let mut enc = zstd::stream::write::Encoder::new(file, 3).unwrap();
        use std::io::Write;
        // Mix of one valid + two garbage lines.
        let env = turn_envelope("valid");
        enc.write_all(serde_json::to_string(&env).unwrap().as_bytes())
            .unwrap();
        enc.write_all(b"\n").unwrap();
        enc.write_all(b"{\"not\":\"an envelope\"}\n").unwrap();
        enc.write_all(b"plain text trash\n").unwrap();
        enc.finish().unwrap();

        let (report, hits) = load_lane_hits(dir.path());
        assert_eq!(report.envelopes_parsed, 1);
        assert_eq!(report.lines_dropped, 2);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn renders_tool_call_with_summary_text() {
        let mut env = turn_envelope("ignored");
        env.kind = Kind::ToolCall;
        env.payload = serde_json::to_value(ToolCall {
            tool_name: "Edit".to_string(),
            input: json!({ "file_path": "x.rs", "new_string": "abc" }),
            output: Some(cortex_core::events::ToolCallOutput {
                stdout: Some("ok".to_string()),
                stderr: None,
                exit_code: Some(0),
                truncated: false,
                cas_ref: None,
                size: None,
            }),
            duration_ms: Some(12),
            touched: Vec::new(),
            outcome: "success".to_string(),
        })
        .unwrap();
        let hit = envelope_to_hit(&env).unwrap();
        assert!(hit.text.starts_with("[Edit]"));
        assert!(hit.text.contains("file_path"));
        assert!(hit.text.contains("ok"));
        assert_eq!(hit.symbol.as_deref(), Some("tool_call:Edit"));
    }
}
