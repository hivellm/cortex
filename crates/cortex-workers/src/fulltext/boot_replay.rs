//! Boot-time replay-missing-partitions defense (phase4f).
//!
//! Closes the recovery hole left by phase4a: even with stale-sweep
//! and lazy `ensure_index` in place, the worker still depends on the
//! `cortex.events.enriched` Synap stream catching every event in real
//! time. If the worker crashes before catching up to a bootstrap, the
//! Synap stream rotates past the gap, or the stack starts cold against
//! an archive-only deployment, the corresponding
//! `cortex-{repo_slug}-{family}` partitions never materialise and the
//! keyword lane silently degrades to "missing repo".
//!
//! This module walks the event archive (`raw-*.parquet` zstd NDJSON,
//! same path the `cortex-graph-backfill` binary scans) and replays the
//! envelopes whose target partition is missing from Meili through the
//! production [`MeiliFulltextIndexer`] upsert path. Idempotent: Meili
//! keys on the document id derived from `content_hash`, so every
//! envelope produces the same Meili row regardless of how many times
//! the replay runs.
//!
//! The routine is **off by default**. The worker's `main.rs` only runs
//! it when `CORTEX_FULLTEXT_REPLAY_MISSING=1` is set so a hot-path
//! restart never triggers a multi-minute archive scan.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use crate::embedder::EnrichedEvent;
use cortex_core::events::Envelope;
use cortex_storage::names::{slug_for_repo, UNKNOWN_REPO_SLUG};

use super::indexer::FulltextIndexer;
use super::meili_client::{MeiliClient, MeiliError};
use super::metrics::Metrics;
use super::routing::{family_for_event, FAMILIES};

/// One partition identifier — `(repo_slug, family)`.
pub type Partition = (String, String);

/// Per-run summary the boot path logs and tests assert against. The
/// fields mirror the spec-08 §Observability metric set:
/// `examined_archives` (files scanned), `missing_partitions` (pairs
/// present in archive but absent from Meili), `replayed_events` (total
/// docs handed to the upsert path), and the wall-clock latency.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    /// Number of archive files (`raw-*.parquet`) the routine opened.
    pub examined_archives: u32,
    /// Count of `(repo_slug, family)` partitions present in the archive
    /// but not in Meili at the time the scan ran.
    pub missing_partitions: u32,
    /// Total events the routine fed into the indexer's upsert path.
    pub replayed_events: u32,
    /// Wall-clock latency of the entire replay phase in milliseconds.
    pub latency_ms: u32,
}

/// Compute the set of `(repo_slug, family)` partitions present in the
/// archive but missing from Meili.
///
/// Walks the live cluster via [`MeiliClient::list_indexes`] and parses
/// each canonical-shaped uid back into its `(slug, family)` pair. Then
/// scans the archive for the union of pairs present in any envelope.
/// The set difference is the partitions a replay would create.
pub async fn missing_partitions<C>(
    client: &C,
    archive_root: &Path,
    prefix: &str,
) -> Result<BTreeSet<Partition>, MeiliError>
where
    C: MeiliClient + ?Sized,
{
    let live = client.list_indexes().await?;
    let prefix_trimmed = prefix.trim_end_matches('-');
    let live_set: BTreeSet<Partition> = live
        .into_iter()
        .filter_map(|stat| parse_partition_from_uid(&stat.uid, prefix_trimmed))
        .collect();

    let archive_set = scan_archive_partitions(archive_root);
    Ok(archive_set.difference(&live_set).cloned().collect())
}

/// Walk the archive once, filter every envelope whose
/// `(repo_slug, family)` pair is in `missing`, route it through the
/// production indexer's upsert path, and return a per-run summary.
///
/// `metrics` is incremented per replayed event so operators can confirm
/// the scan landed the expected per-partition counts.
pub async fn replay_missing_partitions<C, I>(
    client: &C,
    indexer: Arc<I>,
    metrics: &Metrics,
    archive_root: &Path,
    prefix: &str,
) -> Result<ReplayReport, MeiliError>
where
    C: MeiliClient + ?Sized,
    I: FulltextIndexer + ?Sized,
{
    let start = Instant::now();
    let missing = missing_partitions(client, archive_root, prefix).await?;
    if missing.is_empty() {
        return Ok(ReplayReport {
            examined_archives: 0,
            missing_partitions: 0,
            replayed_events: 0,
            latency_ms: latency_ms(start),
        });
    }

    let scan = scan_archive_for_partitions(archive_root, &missing);
    if scan.events.is_empty() {
        return Ok(ReplayReport {
            examined_archives: scan.files,
            missing_partitions: u32::try_from(missing.len()).unwrap_or(u32::MAX),
            replayed_events: 0,
            latency_ms: latency_ms(start),
        });
    }

    // Count per-partition before the upsert so the metric reflects the
    // intent even if the indexer drops some events as `Skipped`.
    for (slug, family) in &scan.event_partitions {
        metrics.incr_replay_events(slug, family, 1);
    }

    let report = indexer.index_batch(&scan.events).await?;

    Ok(ReplayReport {
        examined_archives: scan.files,
        missing_partitions: u32::try_from(missing.len()).unwrap_or(u32::MAX),
        replayed_events: report.documents_upserted,
        latency_ms: latency_ms(start),
    })
}

fn latency_ms(start: Instant) -> u32 {
    u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// Parse a canonical `cortex-{slug}-{family}` uid back into the
/// `(slug, family)` pair. Returns `None` for non-canonical uids so the
/// caller's set difference stays scoped to the canonical vocabulary.
fn parse_partition_from_uid(uid: &str, prefix_trimmed: &str) -> Option<Partition> {
    let head = format!("{prefix_trimmed}-");
    let rest = uid.strip_prefix(&head)?;
    for family in FAMILIES.iter() {
        let suffix = format!("-{family}");
        if let Some(slug) = rest.strip_suffix(&suffix) {
            if !slug.is_empty()
                && slug
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !slug.starts_with('-')
                && !slug.ends_with('-')
            {
                return Some((slug.to_string(), (*family).to_string()));
            }
        }
    }
    None
}

struct ScanResult {
    files: u32,
    events: Vec<EnrichedEvent>,
    /// Per-event `(slug, family)` pairs in the same order as `events`,
    /// so the metric increment loop can attribute each replay correctly.
    event_partitions: Vec<Partition>,
}

/// Walk the archive and collect the union of `(slug, family)` pairs
/// every envelope would route to.
fn scan_archive_partitions(archive_root: &Path) -> BTreeSet<Partition> {
    let mut out = BTreeSet::new();
    let mut files = 0u32;
    walk_with_files(archive_root, &mut files, &mut |env| {
        out.insert(partition_for(env));
    });
    out
}

/// Walk the archive a second time and collect the envelopes whose
/// `(slug, family)` is in `targets`, lifted to [`EnrichedEvent`].
fn scan_archive_for_partitions(archive_root: &Path, targets: &BTreeSet<Partition>) -> ScanResult {
    let mut files = 0u32;
    let mut events = Vec::new();
    let mut event_partitions = Vec::new();
    walk_with_files(archive_root, &mut files, &mut |env| {
        let part = partition_for(env);
        if targets.contains(&part) {
            events.push(envelope_to_enriched(env.clone()));
            event_partitions.push(part);
        }
    });
    ScanResult {
        files,
        events,
        event_partitions,
    }
}

fn partition_for(env: &Envelope) -> Partition {
    let slug = env
        .context
        .repo
        .as_deref()
        .map(slug_for_repo)
        .unwrap_or_else(|| UNKNOWN_REPO_SLUG.to_string());
    let path = env.context.extras.get("path").and_then(|v| v.as_str());
    // No classifier topics on a raw archive walk — pass an empty slice.
    // `family_for_event` falls back to the path extension first and
    // `misc` last, which is the right call for an archive-only replay.
    let family = family_for_event(env.kind, &[], path).to_string();
    (slug, family)
}

fn walk_with_files(dir: &Path, files: &mut u32, f: &mut impl FnMut(&Envelope)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(dir = %dir.display(), error = %e, "read_dir failed; skipping");
            return;
        }
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    // Stable order so tests (and operator audits) get deterministic
    // results across runs.
    paths.sort();
    for path in paths {
        if path.is_dir() {
            walk_with_files(&path, files, f);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
            continue;
        }
        *files = files.saturating_add(1);
        if let Err(e) = read_one_file(&path, f) {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "partial frame (live file or trailing corruption)"
            );
        }
    }
}

fn read_one_file(path: &Path, f: &mut impl FnMut(&Envelope)) -> std::io::Result<()> {
    let file = File::open(path)?;
    let decoder = zstd::stream::read::Decoder::new(file)?;
    let reader = BufReader::new(decoder);
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(env) = serde_json::from_str::<Envelope>(trimmed) {
            f(&env);
        }
    }
    Ok(())
}

/// Lift a canonical [`Envelope`] to an [`EnrichedEvent`] with the
/// static-fallback classifier slot — same shape `cortex-graph-backfill`
/// uses. The fulltext indexer reads `classifier.topics` for routing,
/// but `family_for_event` already falls back to path/extension when
/// topics are empty, so this empty slot is correct for replay.
fn envelope_to_enriched(env: Envelope) -> EnrichedEvent {
    let event_id = env.event_id.clone();
    let context_path = env
        .context
        .extras
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let session_id = if env.session_id.is_empty() {
        None
    } else {
        Some(env.session_id.clone())
    };
    EnrichedEvent {
        event_id: event_id.clone(),
        kind: env.kind,
        content_hash: env.content_hash,
        redacted_payload: env.payload,
        classifier: ClassifierOutput {
            event_id,
            kind_refinement: None,
            topics: Vec::new(),
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: Vec::new(),
            summary: None,
            entities: Vec::new(),
            relations: Vec::new(),
            sensitivity: Default::default(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "fulltext-replay-v1".into(),
            model: "fulltext-replay-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        },
        context_repo: env.context.repo,
        context_path,
        parent_event_id: env.parent_event_id,
        session_id,
        // Phase20 §5.2 — boot-time replay parses the envelope's
        // `occurred_at` so re-indexed docs land with the real ts.
        occurred_at_ms: chrono::DateTime::parse_from_rfc3339(&env.occurred_at)
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0),
        class_level: None,
        class_compartments: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fulltext::config::FulltextConfig;
    use crate::fulltext::indexer::MeiliFulltextIndexer;
    use crate::fulltext::meili_client::{MemoryCall, MemoryMeiliClient};
    use cortex_core::events::{Context as EventContext, Kind, Stream};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_archive_file(dir: &Path, name: &str, envs: &[Envelope]) {
        let path = dir.join(name);
        let file = File::create(&path).expect("create archive file");
        let mut encoder = zstd::stream::write::Encoder::new(file, 0)
            .expect("zstd encoder")
            .auto_finish();
        for env in envs {
            let line = serde_json::to_string(env).expect("serialize envelope");
            encoder.write_all(line.as_bytes()).expect("write line");
            encoder.write_all(b"\n").expect("write newline");
        }
    }

    fn make_envelope(
        event_id: &str,
        kind: Kind,
        repo: Option<&str>,
        path: Option<&str>,
    ) -> Envelope {
        let mut extras: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        if let Some(p) = path {
            extras.insert("path".to_string(), json!(p));
        }
        Envelope {
            event_id: event_id.to_string(),
            schema_version: "1".to_string(),
            occurred_at: "2026-04-28T00:00:00Z".to_string(),
            ingested_at: None,
            session_id: "session-test".to_string(),
            stream: Stream::Live,
            tool: "claude-code".to_string(),
            model: None,
            kind,
            context: EventContext {
                repo: repo.map(|s| s.to_string()),
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "linux".to_string(),
                ide: None,
                extras,
            },
            payload: json!({ "fixture": true }),
            redactions: Vec::new(),
            content_hash: format!(
                "sha256:{:0>64}",
                event_id.chars().take(8).collect::<String>()
            ),
            parent_event_id: None,
            class_level: None,
            class_compartments: None,
        }
    }

    #[test]
    fn parse_partition_round_trips_canonical_uids() {
        assert_eq!(
            parse_partition_from_uid("cortex-cortex-code", "cortex"),
            Some(("cortex".to_string(), "code".to_string()))
        );
        assert_eq!(
            parse_partition_from_uid("cortex-cortex-mcp-decisions", "cortex"),
            Some(("cortex-mcp".to_string(), "decisions".to_string()))
        );
        assert_eq!(parse_partition_from_uid("cortex-code", "cortex"), None);
        assert_eq!(parse_partition_from_uid("legacy-foo", "cortex"), None);
        assert_eq!(
            parse_partition_from_uid("cortex-cortex-bogus", "cortex"),
            None
        );
    }

    #[tokio::test]
    async fn missing_partitions_returns_archive_minus_meili() {
        let tmp = TempDir::new().unwrap();
        let envs = vec![
            // Routes to (cortex, code) via .rs extension.
            make_envelope("e1", Kind::Artifact, Some("Cortex"), Some("src/lib.rs")),
            // Routes to (rulebook, decisions) via Decision kind.
            make_envelope("e2", Kind::Decision, Some("Rulebook"), None),
        ];
        write_archive_file(tmp.path(), "raw-001.parquet", &envs);

        let client = MemoryMeiliClient::new();
        // Meili already has the (cortex, code) partition; missing
        // result must be exactly (rulebook, decisions).
        client.seed_index("cortex-cortex-code", 100);

        let missing = missing_partitions(&client, tmp.path(), "cortex-")
            .await
            .unwrap();
        assert_eq!(
            missing.into_iter().collect::<Vec<_>>(),
            vec![("rulebook".to_string(), "decisions".to_string())]
        );
    }

    #[tokio::test]
    async fn replay_creates_missing_partition_and_skips_present_one() {
        let tmp = TempDir::new().unwrap();
        let envs = vec![
            // (cortex, code) — already in Meili, must be skipped.
            make_envelope("e1", Kind::Artifact, Some("Cortex"), Some("src/lib.rs")),
            make_envelope("e2", Kind::Artifact, Some("Cortex"), Some("src/main.rs")),
            // (rulebook, decisions) — missing, must be replayed.
            make_envelope("e3", Kind::Decision, Some("Rulebook"), None),
            make_envelope("e4", Kind::Decision, Some("Rulebook"), None),
            make_envelope("e5", Kind::Decision, Some("Rulebook"), None),
        ];
        write_archive_file(tmp.path(), "raw-001.parquet", &envs);

        let client = Arc::new(MemoryMeiliClient::new());
        client.seed_index("cortex-cortex-code", 2);

        let metrics = Arc::new(Metrics::new());
        let indexer = Arc::new(MeiliFulltextIndexer::new(
            FulltextConfig::default(),
            client.clone(),
            metrics.clone(),
        ));

        let report = replay_missing_partitions(
            client.as_ref(),
            indexer.clone(),
            metrics.as_ref(),
            tmp.path(),
            "cortex-",
        )
        .await
        .unwrap();

        assert_eq!(report.examined_archives, 1);
        assert_eq!(report.missing_partitions, 1);
        // All three Decision envelopes route to the rulebook decisions
        // index; the two Artifact envelopes were skipped because their
        // partition was already present in Meili. Phase11k §2 — Decision
        // events ALSO dual-write to the global `cortex_decisions` index,
        // so the upsert count doubles (3 per-repo + 3 global = 6).
        assert_eq!(report.replayed_events, 6);

        // Per-partition metric records exactly the three replays under
        // (rulebook, decisions); the (cortex, code) partition stays at 0.
        let snap = metrics.replay_events_snapshot();
        assert_eq!(snap.get("rulebook|decisions"), Some(&3));
        assert!(!snap.contains_key("cortex|code"));

        // Indexer wrote into the canonical rulebook-decisions index.
        let calls = client.calls_snapshot();
        let upserts: Vec<&str> = calls
            .iter()
            .filter_map(|c| match c {
                MemoryCall::UpsertDocuments { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            upserts.contains(&"cortex-rulebook-decisions"),
            "expected cortex-rulebook-decisions among upserts, got {upserts:?}"
        );
        assert!(
            !upserts.contains(&"cortex-cortex-code"),
            "must not re-upsert into already-present partition"
        );
    }

    #[tokio::test]
    async fn replay_is_noop_when_archive_matches_meili() {
        let tmp = TempDir::new().unwrap();
        let envs = vec![make_envelope("e1", Kind::Decision, Some("Cortex"), None)];
        write_archive_file(tmp.path(), "raw-001.parquet", &envs);

        let client = Arc::new(MemoryMeiliClient::new());
        client.seed_index("cortex-cortex-decisions", 1);

        let metrics = Arc::new(Metrics::new());
        let indexer = Arc::new(MeiliFulltextIndexer::new(
            FulltextConfig::default(),
            client.clone(),
            metrics.clone(),
        ));

        let report = replay_missing_partitions(
            client.as_ref(),
            indexer.clone(),
            metrics.as_ref(),
            tmp.path(),
            "cortex-",
        )
        .await
        .unwrap();

        assert_eq!(report.missing_partitions, 0);
        assert_eq!(report.replayed_events, 0);
        // No upsert calls — the missing-set was empty, so the routine
        // exits before scanning for events.
        let upserts = client
            .calls_snapshot()
            .into_iter()
            .filter(|c| matches!(c, MemoryCall::UpsertDocuments { .. }))
            .count();
        assert_eq!(upserts, 0);
    }
}
