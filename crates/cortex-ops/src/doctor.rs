//! Cross-backend consistency doctor (phase4d).
//!
//! The 2026-04-27 audit caught real drift between Meilisearch and
//! the event archive only because an operator hand-curled the
//! backends and decompressed the zstd archive in Python. This
//! module is the automation that replaces that workflow.
//!
//! v1 ships **coverage mode** for the Meili ↔ archive axis:
//!
//! - [`ArchiveProbe`] walks `~/.cortex/archive/events/**/*.parquet`
//!   (zstd-NDJSON) and counts envelopes grouped by
//!   `(repo_slug, family)` using `cortex_fulltext::routing` so the
//!   partition view exactly matches what the live indexer produces.
//! - [`MeiliCoverageProbe`] consumes any
//!   `cortex_fulltext::MeiliClient` (the same trait the live worker
//!   uses) and lifts its `list_indexes()` output into the same
//!   `(repo_slug, family) → count` shape.
//! - [`coverage_report`] computes the union of partitions across
//!   probes, marks rows where the archive has events but Meili is
//!   empty (or missing) as `inconsistent`, and returns a structured
//!   report the CLI renders as Markdown / JSON.
//!
//! Vectorizer and Nexus probes, plus probe mode (Jaccard overlap on
//! a query), are out of scope for v1 — see
//! `phase4h_doctor_vec_nexus_probes` and
//! `phase4i_doctor_query_overlap_mode` for the carve-outs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use cortex_fulltext::routing::{family_for_event, is_canonical_index_name, FAMILIES};
use cortex_fulltext::MeiliClient;
use serde::{Deserialize, Serialize};

/// One `(repo_slug, family)` partition key. Matches the Meili
/// index naming convention `cortex-{repo_slug}-{family}` and the
/// fulltext routing function output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PartitionKey {
    /// Repo slug (already lowercased / canonicalised).
    pub repo: String,
    /// Family suffix (`code`, `docs`, …).
    pub family: String,
}

impl PartitionKey {
    /// Compose the canonical Meili index name this partition maps
    /// to (`cortex-{repo}-{family}`).
    pub fn meili_index(&self) -> String {
        format!("cortex-{}-{}", self.repo, self.family)
    }
}

/// Aggregate counts captured by [`ArchiveProbe::scan`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveSummary {
    /// Total `.parquet` files visited.
    pub files_visited: u64,
    /// Total NDJSON lines parsed (regardless of routing).
    pub envelopes_parsed: u64,
    /// Per-`(repo, family)` envelope counts.
    pub partitions: BTreeMap<PartitionKey, u64>,
}

/// Walks a `cortex-ingestion` archive root and counts envelopes
/// per `(repo_slug, family)` using the same routing function the
/// live fulltext worker applies. Read-only — never writes back.
pub struct ArchiveProbe {
    /// Archive root, typically `$CORTEX_ARCHIVE_ROOT` (default
    /// `~/.cortex/archive`). The probe walks
    /// `<root>/events/**/raw-*.parquet`.
    pub root: PathBuf,
}

impl ArchiveProbe {
    /// Build a probe rooted at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { root: path.into() }
    }

    /// Walk the archive and produce the per-partition summary.
    /// Errors short-circuit only on top-level I/O failures; per-file
    /// decode failures are skipped silently the same way
    /// `cortex_api::archive_loader` does.
    pub fn scan(&self) -> Result<ArchiveSummary> {
        let mut summary = ArchiveSummary::default();
        let events_dir = self.root.join("events");
        if !events_dir.exists() {
            return Ok(summary);
        }
        scan_dir(&events_dir, &mut summary);
        Ok(summary)
    }
}

fn scan_dir(dir: &Path, summary: &mut ArchiveSummary) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, summary);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
            continue;
        }
        summary.files_visited += 1;
        if let Err(e) = scan_file(&path, summary) {
            tracing::debug!(path = %path.display(), error = %e, "doctor archive scan: skipping unreadable file");
        }
    }
}

fn scan_file(path: &Path, summary: &mut ArchiveSummary) -> Result<()> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path)?;
    let decoder = zstd::stream::read::Decoder::new(file)?;
    let reader = BufReader::new(decoder);
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let env: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        summary.envelopes_parsed += 1;
        let key = match partition_key_for_envelope(&env) {
            Some(k) => k,
            None => continue,
        };
        *summary.partitions.entry(key).or_insert(0) += 1;
    }
    Ok(())
}

/// Project a raw envelope JSON (no canonical struct decode) onto
/// the same `(repo_slug, family)` shape the fulltext routing
/// produces. Returns `None` for envelopes that have no `kind` or
/// no `context.repo` — they cannot be honestly placed in any
/// partition.
fn partition_key_for_envelope(env: &serde_json::Value) -> Option<PartitionKey> {
    let kind_str = env.get("kind").and_then(|v| v.as_str())?;
    let kind = kind_from_str(kind_str)?;
    let ctx = env.get("context")?;
    let repo = ctx.get("repo").and_then(|v| v.as_str())?;
    if repo.is_empty() {
        return None;
    }
    let path = ctx.get("path").and_then(|v| v.as_str()).map(|s| s.to_string());
    let topics: Vec<String> = env
        .get("classifier")
        .and_then(|c| c.get("topics"))
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let family = family_for_event(kind, &topics, path.as_deref());
    Some(PartitionKey {
        repo: cortex_storage::names::slug_for_repo(repo),
        family: family.to_string(),
    })
}

/// Map the canonical envelope `kind` string to the `cortex-core`
/// enum. Anything outside the closed set (8 kinds today) returns
/// `None`; the doctor drops those events from the partition count
/// rather than guessing a placement.
fn kind_from_str(s: &str) -> Option<cortex_core::events::Kind> {
    use cortex_core::events::Kind;
    Some(match s {
        "turn" => Kind::Turn,
        "tool_call" => Kind::ToolCall,
        "agent_call" => Kind::AgentCall,
        "memory" => Kind::Memory,
        "decision" => Kind::Decision,
        "analysis" => Kind::Analysis,
        "law_violation" => Kind::LawViolation,
        "artifact" => Kind::Artifact,
        _ => return None,
    })
}

/// Per-`(repo, family)` row in the coverage report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageRow {
    /// Partition key.
    pub partition: PartitionKey,
    /// Number of envelopes the archive scan attributed to this
    /// partition.
    pub archive_events: u64,
    /// `numberOfDocuments` from the matching Meili index.
    /// `None` when the index does not exist.
    pub meili_docs: Option<u64>,
    /// Vector count from the matching Vectorizer collection
    /// (`cortex-{repo}-{family}`). `None` when the probe was not
    /// run or the collection does not exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vec_vectors: Option<u64>,
    /// Artifact count from the Nexus graph for this row's repo.
    /// Nexus is repo-grain only — the same value repeats for every
    /// `(repo, *)` row that shares a `repo`. `None` when the probe
    /// was not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_artifacts: Option<u64>,
    /// `true` when the archive has events for this partition but
    /// Meili is missing the index (or has zero docs).
    pub inconsistent: bool,
    /// `true` when both `meili_docs` and `vec_vectors` are positive
    /// but their ratio exceeds [`CoverageOptions::vec_to_meili_ratio_max`].
    /// Suspicious rows do **not** flip [`DoctorReport::failed`] —
    /// chunking can legitimately multiply, but a 50× expansion still
    /// warrants a manual look.
    #[serde(default)]
    pub suspicious: bool,
    /// Free-text reason when `inconsistent` or `suspicious` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Tunable thresholds for [`coverage_report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageOptions {
    /// Largest tolerated `vec_vectors / meili_docs` ratio before a
    /// row is flagged `suspicious`. Default `50` — well above the
    /// expected chunk-fan-out of code (≤ 20) and docs (≤ 10) but
    /// below an obvious blunder (e.g. forgetting to delete a stale
    /// collection on rerouting).
    pub vec_to_meili_ratio_max: u64,
}

impl Default for CoverageOptions {
    fn default() -> Self {
        Self {
            vec_to_meili_ratio_max: 50,
        }
    }
}

/// Top-level report shape returned by [`coverage_report`] and
/// rendered by [`render_coverage_markdown`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DoctorReport {
    /// Archive scan summary (always populated).
    pub archive: ArchiveSummary,
    /// Per-partition rows in lexicographic order on `(repo, family)`.
    pub rows: Vec<CoverageRow>,
    /// Names of Meili indexes that violate the canonical
    /// `cortex-{repo}-{family}` shape — surfaced separately so the
    /// operator can clean them up via `phase4a` sweep semantics.
    pub non_canonical_meili_indexes: Vec<String>,
    /// Names of Vectorizer collections that violate the canonical
    /// `cortex-{repo}-{family}` shape. Same operator-action contract
    /// as the Meili siblings — surface, do not auto-clean.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub non_canonical_vectorizer_collections: Vec<String>,
    /// Phase4i query-overlap reports — one entry per `--query`
    /// passed to the CLI. Empty when no queries were supplied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<crate::probe::QueryReport>,
    /// Phase3 — coverage of `content_hash` on archive-sourced
    /// `tool_call` envelopes from the last 24 h. `None` when the
    /// caller did not run the probe (e.g. cold dev stack with no
    /// archive root). Sets `failed` on its own when the ratio falls
    /// below the threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_coverage: Option<HashCoverageSummary>,
    /// `true` when at least one row has `inconsistent = true` OR at
    /// least one query is `below_threshold`. The CLI exits non-zero
    /// on this flag. Suspicious rows (the ratio probe) do not
    /// contribute.
    pub failed: bool,
}

/// Default coverage threshold for the `tool_call_hash_coverage`
/// probe — 99% per the proposal in
/// `.rulebook/tasks/phase3_tool_call_hash_preview/proposal.md`.
pub const HASH_COVERAGE_THRESHOLD: f64 = 0.99;

/// Default rolling window for the probe — 24 h.
pub const HASH_COVERAGE_WINDOW_HOURS: i64 = 24;

/// Phase3 `tool_call_hash_coverage` probe summary. Walks the
/// archive parquet files and counts `tool_call` envelopes whose
/// `content_hash` is non-empty, scoped to the last
/// `window_hours` window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HashCoverageSummary {
    /// Total `tool_call` envelopes observed inside the window.
    pub tool_calls_total: u64,
    /// Subset of `tool_calls_total` whose `content_hash` is a
    /// non-empty string. The proposal expects this to track 1:1 with
    /// `tool_calls_total` because the spec-18 plugin stamps the hash
    /// on every envelope.
    pub tool_calls_with_hash: u64,
    /// `tool_calls_with_hash / tool_calls_total` — `0.0` when no
    /// envelopes fell inside the window, which the probe treats as
    /// "skip" (does not flip `failed`).
    pub ratio: f64,
    /// Threshold the probe fails below.
    pub threshold: f64,
    /// Window size in hours. Mirrors
    /// [`HASH_COVERAGE_WINDOW_HOURS`] unless the caller overrode it.
    pub window_hours: i64,
    /// `true` when `tool_calls_total > 0` AND `ratio < threshold`.
    pub failed: bool,
}

/// Run the phase3 `tool_call_hash_coverage` probe. Walks the same
/// archive root [`ArchiveProbe`] uses and computes the
/// `content_hash` coverage ratio for `tool_call` envelopes whose
/// `occurred_at` falls inside the last `window_hours` window.
///
/// `now_ms` lets tests pin "now" for deterministic windowing;
/// production callers pass `chrono::Utc::now().timestamp_millis()`.
/// The probe is read-only and never propagates an error: an
/// unreadable archive root produces an empty summary so the CLI
/// surfaces the absence as "skip" instead of a hard failure.
pub fn scan_hash_coverage(
    root: &Path,
    now_ms: i64,
    window_hours: i64,
    threshold: f64,
) -> HashCoverageSummary {
    let mut totals = HashCoverageScanState::default();
    let cutoff_ms = now_ms.saturating_sub(window_hours.max(0).saturating_mul(3_600_000));
    let events_dir = root.join("events");
    if events_dir.exists() {
        scan_hash_dir(&events_dir, cutoff_ms, &mut totals);
    }
    let ratio = if totals.tool_calls_total == 0 {
        0.0
    } else {
        totals.tool_calls_with_hash as f64 / totals.tool_calls_total as f64
    };
    let failed = totals.tool_calls_total > 0 && ratio < threshold;
    HashCoverageSummary {
        tool_calls_total: totals.tool_calls_total,
        tool_calls_with_hash: totals.tool_calls_with_hash,
        ratio,
        threshold,
        window_hours,
        failed,
    }
}

#[derive(Default)]
struct HashCoverageScanState {
    tool_calls_total: u64,
    tool_calls_with_hash: u64,
}

fn scan_hash_dir(dir: &Path, cutoff_ms: i64, state: &mut HashCoverageScanState) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_hash_dir(&path, cutoff_ms, state);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
            continue;
        }
        let _ = scan_hash_file(&path, cutoff_ms, state);
    }
}

fn scan_hash_file(
    path: &Path,
    cutoff_ms: i64,
    state: &mut HashCoverageScanState,
) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path)?;
    let decoder = zstd::stream::read::Decoder::new(file)?;
    let reader = BufReader::new(decoder);
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let env: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if env.get("kind").and_then(|v| v.as_str()) != Some("tool_call") {
            continue;
        }
        let occurred_ms = env
            .get("occurred_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0);
        if occurred_ms < cutoff_ms {
            continue;
        }
        state.tool_calls_total = state.tool_calls_total.saturating_add(1);
        let has_hash = env
            .get("content_hash")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if has_hash {
            state.tool_calls_with_hash = state.tool_calls_with_hash.saturating_add(1);
        }
    }
    Ok(())
}

/// Coverage probe over a Meili cluster. Wraps any
/// `cortex_fulltext::MeiliClient` so tests can drive it through
/// `MemoryMeiliClient`.
pub struct MeiliCoverageProbe<'a, C: MeiliClient + ?Sized> {
    /// The Meili client.
    pub client: &'a C,
}

impl<'a, C: MeiliClient + ?Sized> MeiliCoverageProbe<'a, C> {
    /// Build a probe that delegates to `client`.
    pub fn new(client: &'a C) -> Self {
        Self { client }
    }
}

#[async_trait]
trait MeiliCoverageScan {
    async fn scan(
        &self,
    ) -> anyhow::Result<(BTreeMap<PartitionKey, u64>, Vec<String>)>;
}

#[async_trait]
impl<C: MeiliClient + ?Sized + Sync> MeiliCoverageScan for MeiliCoverageProbe<'_, C> {
    async fn scan(
        &self,
    ) -> anyhow::Result<(BTreeMap<PartitionKey, u64>, Vec<String>)> {
        let indexes = self
            .client
            .list_indexes()
            .await
            .map_err(|e| anyhow::anyhow!("meili list_indexes: {e}"))?;
        let mut partitions: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        let mut non_canonical: Vec<String> = Vec::new();
        for index in indexes {
            if !is_canonical_index_name(&index.uid) {
                non_canonical.push(index.uid);
                continue;
            }
            // Strip the `cortex-` prefix and the trailing
            // `-{family}` suffix to recover (repo, family).
            let rest = match index.uid.strip_prefix("cortex-") {
                Some(r) => r,
                None => continue,
            };
            let mut parsed: Option<(&str, &str)> = None;
            for fam in FAMILIES.iter() {
                let suffix = format!("-{fam}");
                if let Some(slug) = rest.strip_suffix(&suffix) {
                    parsed = Some((slug, *fam));
                    break;
                }
            }
            let (repo, family) = match parsed {
                Some(p) => p,
                None => continue,
            };
            partitions.insert(
                PartitionKey {
                    repo: repo.to_string(),
                    family: family.to_string(),
                },
                index.number_of_documents,
            );
        }
        Ok((partitions, non_canonical))
    }
}

/// Compose the doctor's coverage report from an archive summary
/// and the Meili partition map. v1 entry point — preserves the
/// pre-phase4h shape so callers that don't yet pipe Vectorizer /
/// Nexus counters keep working.
///
/// A row is marked `inconsistent` when:
/// - the archive has events for the partition AND
/// - Meili either lacks the matching index entirely OR has zero
///   documents in it.
///
/// Rows where the archive is empty are still emitted (meili-only
/// partitions surface, e.g. when the bootstrap walked a repo whose
/// archive entries have rotated out of the local FS) but are not
/// marked inconsistent — they're informational.
pub fn coverage_report(
    archive: ArchiveSummary,
    meili_partitions: BTreeMap<PartitionKey, u64>,
    non_canonical_meili_indexes: Vec<String>,
) -> DoctorReport {
    coverage_report_full(
        archive,
        meili_partitions,
        non_canonical_meili_indexes,
        BTreeMap::new(),
        Vec::new(),
        BTreeMap::new(),
        CoverageOptions::default(),
    )
}

/// Compose the doctor's coverage report from every backend probe.
/// Callers that want the full archive ↔ Meili ↔ Vectorizer ↔ Nexus
/// matrix go through this entry. The narrower [`coverage_report`]
/// remains for v1 clients that only have the archive + Meili axis.
///
/// Suspicious-row policy: a row is marked `suspicious` (but not
/// `inconsistent`) when both `meili_docs > 0` and `vec_vectors > 0`
/// and `vec_vectors / meili_docs > opts.vec_to_meili_ratio_max`.
/// Suspicious rows do not flip [`DoctorReport::failed`].
pub fn coverage_report_full(
    archive: ArchiveSummary,
    meili_partitions: BTreeMap<PartitionKey, u64>,
    non_canonical_meili_indexes: Vec<String>,
    vec_partitions: BTreeMap<PartitionKey, u64>,
    non_canonical_vectorizer_collections: Vec<String>,
    nexus_repo_counts: NexusCounts,
    opts: CoverageOptions,
) -> DoctorReport {
    let mut rows: Vec<CoverageRow> = Vec::new();
    let mut keys: std::collections::BTreeSet<PartitionKey> = Default::default();
    for k in archive.partitions.keys() {
        keys.insert(k.clone());
    }
    for k in meili_partitions.keys() {
        keys.insert(k.clone());
    }
    for k in vec_partitions.keys() {
        keys.insert(k.clone());
    }

    let nexus_in_use = !nexus_repo_counts.is_empty();

    for key in keys {
        let archive_events = archive.partitions.get(&key).copied().unwrap_or(0);
        let meili_docs = meili_partitions.get(&key).copied();
        let vec_vectors = vec_partitions.get(&key).copied();
        let nexus_artifacts = if nexus_in_use {
            // Repo-grain only — every `(repo, *)` row carries the
            // same value. `None` when the repo is in archive/meili
            // but the graph hasn't seen it yet.
            nexus_repo_counts.get(&key.repo).copied()
        } else {
            None
        };

        let mut inconsistent = false;
        let mut suspicious = false;
        let mut reason: Option<String> = None;

        if archive_events > 0 {
            match meili_docs {
                None => {
                    inconsistent = true;
                    reason = Some(format!(
                        "archive has {archive_events} events for {0}/{1}; meili index missing",
                        key.repo, key.family,
                    ));
                }
                Some(0) => {
                    inconsistent = true;
                    reason = Some(format!(
                        "archive has {archive_events} events for {0}/{1}; meili index empty",
                        key.repo, key.family,
                    ));
                }
                _ => {}
            }
        }

        // Ratio probe — only fires when both sides are present and
        // populated. A missing Vectorizer probe (no value) is silent;
        // an empty collection (`Some(0)`) is also silent because the
        // chunker may legitimately produce zero vectors for an
        // event with no body.
        if !inconsistent {
            if let (Some(meili), Some(vec)) = (meili_docs, vec_vectors) {
                if meili > 0 && vec > meili.saturating_mul(opts.vec_to_meili_ratio_max) {
                    suspicious = true;
                    reason = Some(format!(
                        "vec/meili ratio {0:.1} exceeds threshold {1} for {2}/{3}",
                        vec as f64 / meili as f64,
                        opts.vec_to_meili_ratio_max,
                        key.repo,
                        key.family,
                    ));
                }
            }
        }

        rows.push(CoverageRow {
            partition: key,
            archive_events,
            meili_docs,
            vec_vectors,
            nexus_artifacts,
            inconsistent,
            suspicious,
            reason,
        });
    }

    let failed = rows.iter().any(|r| r.inconsistent);
    DoctorReport {
        archive,
        rows,
        non_canonical_meili_indexes,
        non_canonical_vectorizer_collections,
        queries: Vec::new(),
        hash_coverage: None,
        failed,
    }
}

/// Render `report` as a Markdown table the operator can paste into
/// a runbook or read on stderr. JSON output is the raw
/// `serde_json::to_string_pretty(&report)`.
pub fn render_coverage_markdown(report: &DoctorReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Archive: {} files, {} envelopes parsed, {} partitions\n\n",
        report.archive.files_visited,
        report.archive.envelopes_parsed,
        report.archive.partitions.len(),
    ));
    out.push_str(
        "| repo | family | archive | meili | vec | nexus | status |\n\
         |------|--------|--------:|------:|----:|------:|--------|\n",
    );
    for row in &report.rows {
        let meili_str = match row.meili_docs {
            Some(n) => n.to_string(),
            None => "—".to_string(),
        };
        let vec_str = match row.vec_vectors {
            Some(n) => n.to_string(),
            None => "—".to_string(),
        };
        let nexus_str = match row.nexus_artifacts {
            Some(n) => n.to_string(),
            None => "—".to_string(),
        };
        let status = if row.inconsistent {
            row.reason
                .clone()
                .unwrap_or_else(|| "inconsistent".into())
        } else if row.suspicious {
            row.reason
                .clone()
                .map(|r| format!("suspicious: {r}"))
                .unwrap_or_else(|| "suspicious".into())
        } else {
            "ok".into()
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            row.partition.repo,
            row.partition.family,
            row.archive_events,
            meili_str,
            vec_str,
            nexus_str,
            status,
        ));
    }
    if !report.non_canonical_meili_indexes.is_empty() {
        out.push_str("\nNon-canonical Meili indexes (sweep candidates):\n");
        for name in &report.non_canonical_meili_indexes {
            out.push_str(&format!("- {name}\n"));
        }
    }
    if !report.non_canonical_vectorizer_collections.is_empty() {
        out.push_str("\nNon-canonical Vectorizer collections (sweep candidates):\n");
        for name in &report.non_canonical_vectorizer_collections {
            out.push_str(&format!("- {name}\n"));
        }
    }
    for q in &report.queries {
        out.push_str(&crate::probe::render_query_markdown(q));
    }
    if let Some(hash) = &report.hash_coverage {
        out.push_str("\n## Tool-call hash coverage (last ");
        out.push_str(&hash.window_hours.to_string());
        out.push_str("h)\n");
        if hash.tool_calls_total == 0 {
            out.push_str("- skip: no `tool_call` envelopes inside the window\n");
        } else {
            let pct = hash.ratio * 100.0;
            let threshold_pct = hash.threshold * 100.0;
            let status = if hash.failed { "FAIL" } else { "ok" };
            out.push_str(&format!(
                "- {status}: {} / {} hashed ({pct:.2}%, threshold {threshold_pct:.2}%)\n",
                hash.tool_calls_with_hash, hash.tool_calls_total,
            ));
        }
    }
    out
}

/// Convenience: run the Meili probe scan against any client.
/// Public-facing async wrapper so the binary doesn't need to know
/// the [`MeiliCoverageScan`] trait exists.
pub async fn meili_partition_counts<C: MeiliClient + ?Sized + Sync>(
    client: &C,
) -> anyhow::Result<(BTreeMap<PartitionKey, u64>, Vec<String>)> {
    MeiliCoverageProbe::new(client).scan().await
}

/// Per-collection vector count surfaced by a Vectorizer probe.
/// One row per Vectorizer collection — non-canonical collections are
/// returned in the second field so the operator can sweep them like
/// the Meili siblings.
pub type VectorizerCounts = (BTreeMap<PartitionKey, u64>, Vec<String>);

/// Coverage probe over a Vectorizer cluster. The trait shape mirrors
/// [`MeiliCoverageScan`] so the report logic can stitch the two
/// backends with the same partition vocabulary. The Live SDK call is
/// gated behind [`LiveVectorizerCoverageProbe`]; tests use the
/// in-memory [`MemoryVectorizerCoverageProbe`].
#[async_trait]
pub trait VectorizerCoverageScan: Send + Sync {
    /// Return the per-`(repo, family)` vector counts plus the list
    /// of non-canonical collection uids that the Vectorizer cluster
    /// holds.
    async fn scan(&self) -> anyhow::Result<VectorizerCounts>;
}

/// In-memory [`VectorizerCoverageScan`] for unit tests. Seeded via
/// [`MemoryVectorizerCoverageProbe::seed`].
#[derive(Debug, Default, Clone)]
pub struct MemoryVectorizerCoverageProbe {
    partitions: BTreeMap<PartitionKey, u64>,
    non_canonical: Vec<String>,
}

impl MemoryVectorizerCoverageProbe {
    /// Build an empty probe.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the in-memory state with one canonical collection.
    pub fn seed(&mut self, repo: &str, family: &str, vectors: u64) {
        self.partitions.insert(
            PartitionKey {
                repo: repo.to_string(),
                family: family.to_string(),
            },
            vectors,
        );
    }

    /// Seed a non-canonical collection name. The probe surfaces
    /// these in the second tuple slot but does **not** map them
    /// onto a partition key (their shape is unknown).
    pub fn seed_non_canonical(&mut self, name: &str) {
        self.non_canonical.push(name.to_string());
    }
}

#[async_trait]
impl VectorizerCoverageScan for MemoryVectorizerCoverageProbe {
    async fn scan(&self) -> anyhow::Result<VectorizerCounts> {
        Ok((self.partitions.clone(), self.non_canonical.clone()))
    }
}

/// Live Vectorizer probe — wraps `vectorizer-sdk`'s authenticated
/// admin client. Authenticates once via `POST /auth/login`, then
/// calls `list_collections()` and parses each canonical name back to
/// `(repo, family)`. The SDK already retries transient transport
/// failures, so this surface stays plain `anyhow::Result`.
pub struct LiveVectorizerCoverageProbe {
    client: vectorizer_sdk::VectorizerClient,
}

impl LiveVectorizerCoverageProbe {
    /// Build a Live probe by authenticating against `base_url` with
    /// the given `username` / `password`. The minted JWT is bound to
    /// a follow-up SDK client via the `api_key` slot — the SDK
    /// transport sniffs the three-segment JWT shape and sends it
    /// as `Authorization: Bearer …`. Same flow `cortex-embedder`'s
    /// `LiveVectorizerClient::login` follows.
    pub async fn new(
        base_url: &str,
        username: &str,
        password: &str,
    ) -> anyhow::Result<Self> {
        let pre_auth = vectorizer_sdk::ClientConfig {
            base_url: Some(base_url.to_string()),
            api_key: None,
            timeout_secs: Some(30),
            ..vectorizer_sdk::ClientConfig::default()
        };
        let auth_client = vectorizer_sdk::VectorizerClient::new(pre_auth)
            .map_err(|e| anyhow::anyhow!("vectorizer client: {e}"))?;
        let token = auth_client
            .login(username, password)
            .await
            .map_err(|e| anyhow::anyhow!("vectorizer login: {e}"))?;
        let bearer = vectorizer_sdk::ClientConfig {
            base_url: Some(base_url.to_string()),
            api_key: Some(token.access_token),
            timeout_secs: Some(30),
            ..vectorizer_sdk::ClientConfig::default()
        };
        let client = vectorizer_sdk::VectorizerClient::new(bearer)
            .map_err(|e| anyhow::anyhow!("vectorizer authenticated client: {e}"))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl VectorizerCoverageScan for LiveVectorizerCoverageProbe {
    async fn scan(&self) -> anyhow::Result<VectorizerCounts> {
        let collections = self
            .client
            .list_collections()
            .await
            .map_err(|e| anyhow::anyhow!("vectorizer list_collections: {e}"))?;
        let mut partitions: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        let mut non_canonical: Vec<String> = Vec::new();
        for col in collections {
            let key = parse_canonical_name(&col.name);
            let vectors = u64::try_from(col.vector_count).unwrap_or(u64::MAX);
            match key {
                Some(k) => {
                    partitions.insert(k, vectors);
                }
                None => non_canonical.push(col.name),
            }
        }
        Ok((partitions, non_canonical))
    }
}

/// Repo-grain artifact counts surfaced by a Nexus probe. The map key
/// is the `Repo.name` value the graph carries; the value is the
/// number of `Artifact` nodes with an `IN_REPO` edge to that repo.
pub type NexusCounts = BTreeMap<String, u64>;

/// Coverage probe over the Nexus graph. The graph is repo-grain only
/// (no family discriminator on the `Artifact` ↔ `Repo` edge), so the
/// trait returns a flat map keyed on `Repo.name`.
#[async_trait]
pub trait NexusCoverageScan: Send + Sync {
    /// Return the per-repo `Artifact` count.
    async fn scan(&self) -> anyhow::Result<NexusCounts>;
}

/// In-memory [`NexusCoverageScan`] for unit tests.
#[derive(Debug, Default, Clone)]
pub struct MemoryNexusCoverageProbe {
    counts: NexusCounts,
}

impl MemoryNexusCoverageProbe {
    /// Build an empty probe.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the in-memory state with one repo's artifact count.
    pub fn seed(&mut self, repo: &str, artifacts: u64) {
        self.counts.insert(repo.to_string(), artifacts);
    }
}

#[async_trait]
impl NexusCoverageScan for MemoryNexusCoverageProbe {
    async fn scan(&self) -> anyhow::Result<NexusCounts> {
        Ok(self.counts.clone())
    }
}

/// Live Nexus probe — wraps the same `LiveNexusClient` the writer
/// uses. Runs the canonical
/// `MATCH (a:Artifact)-[:IN_REPO]->(r:Repo) RETURN r.name AS repo,
/// count(a) AS artifacts` query and projects the rows.
pub struct LiveNexusCoverageProbe {
    client: cortex_graph::LiveNexusClient,
}

impl LiveNexusCoverageProbe {
    /// Build a Live probe from a [`cortex_graph::GraphConfig`] (the
    /// caller is expected to pull the config from env via
    /// `GraphConfig::from_env`).
    pub fn new(config: cortex_graph::GraphConfig) -> anyhow::Result<Self> {
        let client = cortex_graph::LiveNexusClient::new(config)
            .map_err(|e| anyhow::anyhow!("nexus client: {e}"))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl NexusCoverageScan for LiveNexusCoverageProbe {
    async fn scan(&self) -> anyhow::Result<NexusCounts> {
        let cypher = "MATCH (a:Artifact)-[:IN_REPO]->(r:Repo) \
                      RETURN r.name AS repo, count(a) AS artifacts";
        let result = self
            .client
            .execute_with_retry(cypher, None)
            .await
            .map_err(|e| anyhow::anyhow!("nexus probe: {e}"))?;
        let mut out: NexusCounts = BTreeMap::new();
        for row in &result.rows {
            // Each row is a JSON array of `[repo_name, artifact_count]`.
            let arr = match row.as_array() {
                Some(a) => a,
                None => continue,
            };
            if arr.len() < 2 {
                continue;
            }
            let repo = match arr[0].as_str() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            let count = match arr[1].as_u64() {
                Some(n) => n,
                None => continue,
            };
            out.insert(repo, count);
        }
        Ok(out)
    }
}

/// Parse a canonical `cortex-{repo}-{family}` name back into a
/// [`PartitionKey`]. Shared by the Vectorizer probe and tests; the
/// Meili side has its own copy because it iterates indexes inline.
fn parse_canonical_name(name: &str) -> Option<PartitionKey> {
    let rest = name.strip_prefix("cortex-")?;
    for fam in FAMILIES.iter() {
        let suffix = format!("-{fam}");
        if let Some(slug) = rest.strip_suffix(&suffix) {
            if !slug.is_empty()
                && slug
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !slug.starts_with('-')
                && !slug.ends_with('-')
            {
                return Some(PartitionKey {
                    repo: slug.to_string(),
                    family: (*fam).to_string(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_fulltext::MemoryMeiliClient;

    fn key(repo: &str, family: &str) -> PartitionKey {
        PartitionKey {
            repo: repo.to_string(),
            family: family.to_string(),
        }
    }

    #[tokio::test]
    async fn meili_probe_buckets_canonical_indexes_into_partitions() {
        let client = MemoryMeiliClient::new();
        client.seed_index("cortex-cortex-code", 1862);
        client.seed_index("cortex-rulebook-docs", 1456);
        client.seed_index("cortex-tml-code", 184_754);
        // Non-canonical name: collected separately, not bucketed.
        client.seed_index("cortex-code", 0);
        let (partitions, non_canon) = meili_partition_counts(&client).await.unwrap();
        assert_eq!(partitions.get(&key("cortex", "code")), Some(&1862));
        assert_eq!(partitions.get(&key("rulebook", "docs")), Some(&1456));
        assert_eq!(partitions.get(&key("tml", "code")), Some(&184_754));
        assert_eq!(partitions.len(), 3);
        assert_eq!(non_canon, vec!["cortex-code".to_string()]);
    }

    fn write_zstd_ndjson(path: &std::path::Path, lines: &[&str]) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut enc = zstd::stream::write::Encoder::new(file, 0).unwrap();
        for line in lines {
            writeln!(enc, "{line}").unwrap();
        }
        enc.finish().unwrap();
    }

    #[test]
    fn hash_coverage_full_pass_when_every_envelope_has_hash() {
        // Phase3 — every `tool_call` inside the window stamps a
        // `content_hash`. The probe returns ratio = 1.0 and
        // `failed = false`.
        let tmp = tempfile::tempdir().unwrap();
        let events = tmp.path().join("events");
        std::fs::create_dir_all(&events).unwrap();
        let path = events.join("a.parquet");
        write_zstd_ndjson(
            &path,
            &[
                r#"{"kind":"tool_call","occurred_at":"2026-04-28T20:00:00Z","content_hash":"sha256:aaa"}"#,
                r#"{"kind":"tool_call","occurred_at":"2026-04-28T20:01:00Z","content_hash":"sha256:bbb"}"#,
                r#"{"kind":"turn","occurred_at":"2026-04-28T20:02:00Z"}"#,
            ],
        );
        let now_ms = chrono::DateTime::parse_from_rfc3339("2026-04-28T21:00:00Z")
            .unwrap()
            .timestamp_millis();
        let summary = scan_hash_coverage(tmp.path(), now_ms, 24, HASH_COVERAGE_THRESHOLD);
        assert_eq!(summary.tool_calls_total, 2);
        assert_eq!(summary.tool_calls_with_hash, 2);
        assert!((summary.ratio - 1.0).abs() < 1e-9);
        assert!(!summary.failed);
    }

    #[test]
    fn hash_coverage_fails_when_below_threshold_and_skips_outside_window() {
        // Phase3 — one `tool_call` is missing its hash AND another
        // is outside the 24 h window (so it must NOT count). The
        // resulting ratio falls below 99 % → `failed = true`.
        let tmp = tempfile::tempdir().unwrap();
        let events = tmp.path().join("events");
        std::fs::create_dir_all(&events).unwrap();
        write_zstd_ndjson(
            &events.join("recent.parquet"),
            &[
                r#"{"kind":"tool_call","occurred_at":"2026-04-28T20:00:00Z","content_hash":"sha256:aaa"}"#,
                r#"{"kind":"tool_call","occurred_at":"2026-04-28T20:30:00Z","content_hash":""}"#,
            ],
        );
        write_zstd_ndjson(
            &events.join("old.parquet"),
            &[
                // Outside the 24 h window — must be skipped entirely.
                r#"{"kind":"tool_call","occurred_at":"2026-04-20T10:00:00Z","content_hash":"sha256:old"}"#,
            ],
        );
        let now_ms = chrono::DateTime::parse_from_rfc3339("2026-04-28T21:00:00Z")
            .unwrap()
            .timestamp_millis();
        let summary = scan_hash_coverage(tmp.path(), now_ms, 24, HASH_COVERAGE_THRESHOLD);
        assert_eq!(summary.tool_calls_total, 2, "the old envelope must not count");
        assert_eq!(summary.tool_calls_with_hash, 1);
        assert!(summary.ratio < HASH_COVERAGE_THRESHOLD);
        assert!(summary.failed);
    }

    #[test]
    fn hash_coverage_empty_window_does_not_fail() {
        // Phase3 — zero envelopes inside the window is "skip", not
        // a regression: the CLI surfaces it but never flips
        // `report.failed`.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("events")).unwrap();
        let now_ms = 1_700_000_000_000;
        let summary = scan_hash_coverage(tmp.path(), now_ms, 24, HASH_COVERAGE_THRESHOLD);
        assert_eq!(summary.tool_calls_total, 0);
        assert!(!summary.failed);
    }

    #[test]
    fn coverage_marks_archive_only_partitions_inconsistent() {
        let mut archive = ArchiveSummary::default();
        archive.partitions.insert(key("rulebook", "code"), 770);
        archive.partitions.insert(key("cortex", "code"), 1862);
        let mut meili: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        meili.insert(key("cortex", "code"), 1862);
        // rulebook/code missing from meili → inconsistent.
        let report = coverage_report(archive, meili, Vec::new());
        let rb = report
            .rows
            .iter()
            .find(|r| r.partition == key("rulebook", "code"))
            .unwrap();
        assert!(rb.inconsistent);
        assert!(rb.reason.as_deref().unwrap().contains("missing"));
        let cx = report
            .rows
            .iter()
            .find(|r| r.partition == key("cortex", "code"))
            .unwrap();
        assert!(!cx.inconsistent);
        assert!(report.failed);
    }

    #[test]
    fn coverage_marks_zero_meili_with_archive_data_inconsistent() {
        let mut archive = ArchiveSummary::default();
        archive.partitions.insert(key("synap", "turns"), 770);
        let mut meili: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        meili.insert(key("synap", "turns"), 0);
        let report = coverage_report(archive, meili, Vec::new());
        let row = &report.rows[0];
        assert!(row.inconsistent);
        assert!(row.reason.as_deref().unwrap().contains("empty"));
        assert!(report.failed);
    }

    #[test]
    fn coverage_meili_only_partitions_are_informational_not_failed() {
        let archive = ArchiveSummary::default();
        let mut meili: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        meili.insert(key("cortex", "code"), 100);
        let report = coverage_report(archive, meili, Vec::new());
        assert_eq!(report.rows.len(), 1);
        assert!(!report.rows[0].inconsistent);
        assert!(!report.failed);
    }

    #[test]
    fn render_markdown_emits_table_header_and_rows() {
        let mut archive = ArchiveSummary::default();
        archive.partitions.insert(key("cortex", "code"), 10);
        archive.envelopes_parsed = 10;
        archive.files_visited = 1;
        let mut meili: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        meili.insert(key("cortex", "code"), 10);
        let report = coverage_report(archive, meili, vec!["cortex-code".into()]);
        let md = render_coverage_markdown(&report);
        assert!(md.contains("| repo | family | archive | meili | vec | nexus | status |"));
        assert!(md.contains("| cortex | code | 10 | 10 | — | — | ok |"));
        assert!(md.contains("- cortex-code"));
    }

    #[tokio::test]
    async fn memory_vectorizer_probe_buckets_seeded_collections() {
        let mut probe = MemoryVectorizerCoverageProbe::new();
        probe.seed("cortex", "code", 4_500);
        probe.seed("rulebook", "docs", 2_300);
        probe.seed_non_canonical("legacy-foo");

        let (partitions, non_canon) = probe.scan().await.unwrap();
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions.get(&key("cortex", "code")), Some(&4_500));
        assert_eq!(partitions.get(&key("rulebook", "docs")), Some(&2_300));
        assert_eq!(non_canon, vec!["legacy-foo".to_string()]);
    }

    #[tokio::test]
    async fn memory_nexus_probe_returns_repo_grain_counts() {
        let mut probe = MemoryNexusCoverageProbe::new();
        probe.seed("Cortex", 1862);
        probe.seed("Rulebook", 770);

        let counts = probe.scan().await.unwrap();
        assert_eq!(counts.len(), 2);
        assert_eq!(counts.get("Cortex"), Some(&1862));
        assert_eq!(counts.get("Rulebook"), Some(&770));
    }

    #[test]
    fn coverage_full_widens_rows_with_vec_and_nexus_columns() {
        let mut archive = ArchiveSummary::default();
        archive.partitions.insert(key("cortex", "code"), 1862);
        archive.partitions.insert(key("cortex", "docs"), 200);
        archive.envelopes_parsed = 2062;
        archive.files_visited = 1;

        let mut meili: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        meili.insert(key("cortex", "code"), 1862);
        meili.insert(key("cortex", "docs"), 200);

        let mut vec_partitions: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        // Healthy chunk fan-out: 5× on code, 3× on docs.
        vec_partitions.insert(key("cortex", "code"), 9_310);
        vec_partitions.insert(key("cortex", "docs"), 600);

        let mut nexus: NexusCounts = BTreeMap::new();
        nexus.insert("cortex".to_string(), 2062);

        let report = coverage_report_full(
            archive,
            meili,
            Vec::new(),
            vec_partitions,
            Vec::new(),
            nexus,
            CoverageOptions::default(),
        );
        assert!(!report.failed);
        assert_eq!(report.rows.len(), 2);
        for row in &report.rows {
            assert_eq!(row.nexus_artifacts, Some(2062));
            assert!(row.vec_vectors.is_some());
            assert!(!row.suspicious, "{:?} should not be suspicious", row);
        }

        let md = render_coverage_markdown(&report);
        assert!(md.contains("| cortex | code | 1862 | 1862 | 9310 | 2062 | ok |"));
    }

    #[test]
    fn coverage_full_marks_extreme_vec_to_meili_ratio_suspicious() {
        let mut archive = ArchiveSummary::default();
        archive.partitions.insert(key("cortex", "code"), 100);
        archive.envelopes_parsed = 100;
        archive.files_visited = 1;
        let mut meili: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        meili.insert(key("cortex", "code"), 100);
        let mut vec_partitions: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        // 100× expansion — well past the default ratio_max of 50.
        vec_partitions.insert(key("cortex", "code"), 10_000);

        let report = coverage_report_full(
            archive,
            meili,
            Vec::new(),
            vec_partitions,
            Vec::new(),
            BTreeMap::new(),
            CoverageOptions::default(),
        );
        assert!(!report.failed, "suspicious must not flip failed");
        let row = &report.rows[0];
        assert!(row.suspicious);
        assert!(row
            .reason
            .as_deref()
            .unwrap()
            .contains("ratio"));
    }

    #[test]
    fn coverage_full_keeps_inconsistent_priority_over_suspicious() {
        // Inconsistency (archive populated but meili empty) wins
        // over the ratio probe — a missing partition is the more
        // urgent signal.
        let mut archive = ArchiveSummary::default();
        archive.partitions.insert(key("cortex", "code"), 100);
        archive.envelopes_parsed = 100;
        archive.files_visited = 1;
        let mut meili: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        meili.insert(key("cortex", "code"), 0);
        let mut vec_partitions: BTreeMap<PartitionKey, u64> = BTreeMap::new();
        vec_partitions.insert(key("cortex", "code"), 10_000);

        let report = coverage_report_full(
            archive,
            meili,
            Vec::new(),
            vec_partitions,
            Vec::new(),
            BTreeMap::new(),
            CoverageOptions::default(),
        );
        assert!(report.failed);
        let row = &report.rows[0];
        assert!(row.inconsistent);
        assert!(!row.suspicious);
    }

    #[test]
    fn coverage_full_surfaces_non_canonical_vectorizer_collections() {
        let archive = ArchiveSummary::default();
        let report = coverage_report_full(
            archive,
            BTreeMap::new(),
            Vec::new(),
            BTreeMap::new(),
            vec!["cortex-code".into(), "legacy-foo".into()],
            BTreeMap::new(),
            CoverageOptions::default(),
        );
        assert_eq!(
            report.non_canonical_vectorizer_collections,
            vec!["cortex-code".to_string(), "legacy-foo".to_string()],
        );
        let md = render_coverage_markdown(&report);
        assert!(md.contains("Non-canonical Vectorizer collections"));
        assert!(md.contains("- cortex-code"));
    }

    #[test]
    fn parse_canonical_name_matches_vectorizer_naming() {
        assert_eq!(
            parse_canonical_name("cortex-cortex-code"),
            Some(key("cortex", "code"))
        );
        assert_eq!(
            parse_canonical_name("cortex-cortex-mcp-decisions"),
            Some(key("cortex-mcp", "decisions"))
        );
        assert!(parse_canonical_name("cortex-code").is_none());
        assert!(parse_canonical_name("legacy-foo").is_none());
    }

    #[test]
    fn archive_probe_returns_empty_summary_when_root_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let probe = ArchiveProbe::new(tmp.path().join("does-not-exist"));
        let summary = probe.scan().unwrap();
        assert_eq!(summary.files_visited, 0);
        assert_eq!(summary.envelopes_parsed, 0);
        assert!(summary.partitions.is_empty());
    }

    #[test]
    fn archive_probe_buckets_synthetic_envelopes_by_partition() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("events/year=2026/month=04/day=28/hour=18");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw-00000.parquet");
        let mut buf: Vec<u8> = Vec::new();
        for env in [
            serde_json::json!({
                "kind": "tool_call",
                "context": { "repo": "Cortex", "path": "src/lib.rs" },
                "classifier": { "topics": [] },
            }),
            serde_json::json!({
                "kind": "artifact",
                "context": { "repo": "Cortex", "path": "src/main.rs" },
                "classifier": { "topics": [] },
            }),
            serde_json::json!({
                "kind": "artifact",
                "context": { "repo": "Rulebook", "path": "README.md" },
                "classifier": { "topics": [] },
            }),
            // Missing repo — must be dropped, not crash.
            serde_json::json!({
                "kind": "turn",
                "context": {},
                "classifier": { "topics": [] },
            }),
        ] {
            let line = serde_json::to_string(&env).unwrap();
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
        }
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 0).unwrap();
        encoder.write_all(&buf).unwrap();
        let compressed = encoder.finish().unwrap();
        std::fs::write(&path, compressed).unwrap();

        let probe = ArchiveProbe::new(tmp.path());
        let summary = probe.scan().unwrap();
        assert_eq!(summary.files_visited, 1);
        assert_eq!(summary.envelopes_parsed, 4);
        // Two distinct partitions:
        //   (cortex, code)   — tool_call(src/lib.rs) + artifact(src/main.rs, .rs)
        //   (rulebook, docs) — artifact(README.md)
        // The repo-less turn is silently dropped.
        assert_eq!(summary.partitions.len(), 2);
        assert_eq!(summary.partitions.get(&key("cortex", "code")), Some(&2));
        assert_eq!(summary.partitions.get(&key("rulebook", "docs")), Some(&1));
    }
}
