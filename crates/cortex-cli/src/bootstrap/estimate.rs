//! `--dry-run --estimate` mode — walks the repo without emitting
//! events and prints the spec-09 sizing block.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::config::CortexSection;
use super::git::walk_commits;
use super::walker::{walk_repo, FileClass, WalkEntry};

/// Sizing block produced by `--estimate`.
///
/// Numbers are back-of-envelope and match spec 09 §"--estimate" mode:
/// real cost depends on classifier cache hit rate and per-file body
/// distribution. Tests assert the values are non-zero on a populated
/// fixture; they don't pin specific magnitudes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Estimate {
    /// Repo identifier (override or directory name).
    pub repo_id: String,
    /// Files surviving the walker filters.
    pub files_kept: u64,
    /// Files dropped (oversize, extension, path-excluded combined).
    pub files_dropped: u64,
    /// Estimated code chunks (one per ~80 LoC after symbol split).
    pub code_chunks_est: u64,
    /// Estimated doc chunks (one per ~600-byte section).
    pub doc_chunks_est: u64,
    /// Number of git commits the historical-turn walker would emit.
    pub commits: u64,
    /// Total events the bootstrap pass would publish.
    pub events_total: u64,
    /// Bytes of body text after redaction (estimated as raw size for
    /// dry-run; redaction shrinks it slightly in practice).
    pub redacted_bytes_est: u64,
    /// Haiku classifier input-token estimate (rough: 1 token per
    /// 4 bytes of body).
    pub classifier_input_tokens_est: u64,
    /// Haiku classifier output-token estimate (rough: 350 tokens per
    /// event for the structured summary record).
    pub classifier_output_tokens_est: u64,
    /// Embedding storage estimate in bytes (1024-d × 4-byte floats per
    /// chunk + 30 % metadata overhead).
    pub embedding_storage_bytes_est: u64,
    /// Graph node estimate (one per artifact + one per decision +
    /// one per law + one per memory).
    pub graph_nodes_est: u64,
    /// Graph edge estimate (~1.8 edges per node on the bootstrap
    /// shape).
    pub graph_edges_est: u64,
    /// Full-text index size estimate in bytes (~3 KB per doc on
    /// Meilisearch).
    pub fulltext_index_bytes_est: u64,
    /// One-time runtime estimate in seconds (events / 500 events-per-
    /// second sustained rate).
    pub runtime_seconds_est: u64,
}

/// Compute the sizing block for one repo.
///
/// Spec 09 §"--estimate" mode treats this as a no-write probe — the
/// caller only needs the [`Estimate`]; nothing reaches Synap.
pub fn estimate_repo(repo_root: &Path, repo_id: &str, cfg: &CortexSection) -> Estimate {
    let entries = walk_repo(repo_root, cfg);
    let mut files_kept: u64 = 0;
    let mut files_dropped: u64 = 0;
    let mut code_bytes: u64 = 0;
    let mut doc_bytes: u64 = 0;
    let mut decision_count: u64 = 0;
    let mut law_count: u64 = 0;
    let mut memory_count: u64 = 0;
    let mut analysis_count: u64 = 0;
    let mut other_count: u64 = 0;
    for entry in &entries {
        match entry {
            WalkEntry::Accepted {
                size_bytes, class, ..
            } => {
                files_kept += 1;
                match class {
                    FileClass::Code => code_bytes += *size_bytes,
                    FileClass::Doc => doc_bytes += *size_bytes,
                    FileClass::Decision => decision_count += 1,
                    FileClass::Law => law_count += 1,
                    FileClass::Memory => memory_count += 1,
                    // Analyses chunk like docs (markdown sections); count
                    // their bytes in the doc bucket so the chunk estimate
                    // stays directionally correct without a new bucket.
                    FileClass::Analysis => {
                        analysis_count += 1;
                        doc_bytes += *size_bytes;
                    }
                    // phase10e — knowledge / learnings count
                    // alongside memory for the estimate (small,
                    // markdown-shaped, single-event-per-file).
                    FileClass::Knowledge | FileClass::Learning => memory_count += 1,
                    FileClass::Other => other_count += 1,
                }
            }
            WalkEntry::Dropped { .. } => files_dropped += 1,
        }
    }
    // Code chunk estimate: 1 chunk per ~80 LoC ⇒ assume ~3 200 bytes
    // per chunk. Doc chunks: 1 per ~600-byte section.
    let code_chunks_est = code_bytes / 3_200;
    let doc_chunks_est = doc_bytes / 600;

    let commits: u64 = walk_commits(repo_root, cfg.git.since.as_deref())
        .map(|cs| cs.len() as u64)
        .unwrap_or(0);

    let events_total = code_chunks_est
        + doc_chunks_est
        + commits
        + decision_count
        + law_count
        + memory_count
        + analysis_count
        + other_count;

    let redacted_bytes_est = code_bytes + doc_bytes;
    let classifier_input_tokens_est = redacted_bytes_est / 4;
    let classifier_output_tokens_est = events_total * 350;
    let embedding_storage_bytes_est =
        (code_chunks_est + doc_chunks_est) * (1024 * 4 + 1024); // 4 KB float vector + ~1 KB metadata
    let graph_nodes_est =
        files_kept + commits + decision_count + law_count + memory_count + analysis_count;
    let graph_edges_est = (graph_nodes_est * 18) / 10;
    let fulltext_index_bytes_est = events_total * 3_000;
    let runtime_seconds_est = events_total.div_ceil(500);

    Estimate {
        repo_id: repo_id.to_string(),
        files_kept,
        files_dropped,
        code_chunks_est,
        doc_chunks_est,
        commits,
        events_total,
        redacted_bytes_est,
        classifier_input_tokens_est,
        classifier_output_tokens_est,
        embedding_storage_bytes_est,
        graph_nodes_est,
        graph_edges_est,
        fulltext_index_bytes_est,
        runtime_seconds_est,
    }
}

/// Format the spec-09 sizing block as a human-readable string.
pub fn format_estimate(est: &Estimate) -> String {
    format!(
        concat!(
            "Repo: {repo}\n",
            "  Files (after excludes):   {files_kept:>8}\n",
            "  Files dropped:            {files_dropped:>8}\n",
            "  Code chunks (est):        {code_chunks:>8}\n",
            "  Doc chunks (est):         {doc_chunks:>8}\n",
            "  Commits:                  {commits:>8}\n",
            "  Est. events:              {events:>8}\n",
            "  Est. redacted bytes:      {bytes:>8}\n",
            "  Est. classifier tokens (in/out): {tin}/{tout}\n",
            "  Est. embedding storage:   {emb:>8} bytes\n",
            "  Est. graph nodes/edges:   {gn}/{ge}\n",
            "  Est. fulltext index:      {fi:>8} bytes\n",
            "  Est. one-time runtime:    {rt:>4} s\n",
        ),
        repo = est.repo_id,
        files_kept = est.files_kept,
        files_dropped = est.files_dropped,
        code_chunks = est.code_chunks_est,
        doc_chunks = est.doc_chunks_est,
        commits = est.commits,
        events = est.events_total,
        bytes = est.redacted_bytes_est,
        tin = est.classifier_input_tokens_est,
        tout = est.classifier_output_tokens_est,
        emb = est.embedding_storage_bytes_est,
        gn = est.graph_nodes_est,
        ge = est.graph_edges_est,
        fi = est.fulltext_index_bytes_est,
        rt = est.runtime_seconds_est,
    )
}
