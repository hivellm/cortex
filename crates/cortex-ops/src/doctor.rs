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
    /// `true` when the archive has events for this partition but
    /// Meili is missing the index (or has zero docs).
    pub inconsistent: bool,
    /// Free-text reason when `inconsistent` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
    /// `true` when at least one row has `inconsistent = true`. The
    /// CLI exits non-zero on this flag.
    pub failed: bool,
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
/// and the Meili partition map.
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
    let mut rows: Vec<CoverageRow> = Vec::new();
    let mut keys: std::collections::BTreeSet<PartitionKey> = Default::default();
    for k in archive.partitions.keys() {
        keys.insert(k.clone());
    }
    for k in meili_partitions.keys() {
        keys.insert(k.clone());
    }

    for key in keys {
        let archive_events = archive.partitions.get(&key).copied().unwrap_or(0);
        let meili_docs = meili_partitions.get(&key).copied();
        let mut inconsistent = false;
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
        rows.push(CoverageRow {
            partition: key,
            archive_events,
            meili_docs,
            inconsistent,
            reason,
        });
    }

    let failed = rows.iter().any(|r| r.inconsistent);
    DoctorReport {
        archive,
        rows,
        non_canonical_meili_indexes,
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
        "| repo | family | archive | meili | status |\n\
         |------|--------|--------:|------:|--------|\n",
    );
    for row in &report.rows {
        let meili_str = match row.meili_docs {
            Some(n) => n.to_string(),
            None => "—".to_string(),
        };
        let status = if row.inconsistent {
            row.reason.clone().unwrap_or_else(|| "inconsistent".into())
        } else {
            "ok".into()
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            row.partition.repo,
            row.partition.family,
            row.archive_events,
            meili_str,
            status,
        ));
    }
    if !report.non_canonical_meili_indexes.is_empty() {
        out.push_str("\nNon-canonical Meili indexes (sweep candidates):\n");
        for name in &report.non_canonical_meili_indexes {
            out.push_str(&format!("- {name}\n"));
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
        assert!(md.contains("| repo | family | archive | meili | status |"));
        assert!(md.contains("| cortex | code | 10 | 10 | ok |"));
        assert!(md.contains("- cortex-code"));
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
