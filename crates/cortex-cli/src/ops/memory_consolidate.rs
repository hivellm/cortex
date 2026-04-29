//! Phase9h — Claude Code auto-memory consolidator.
//!
//! Claude Code writes one Markdown file per memory under
//! `~/.claude/projects/<project-slug>/memory/*.md` plus an index
//! `MEMORY.md`. The consolidator treats that directory like any
//! other Cortex memory store: it embeds every entry, clusters
//! near-duplicates within the same `type`, asks a merge agent to
//! produce one denser entry per cluster, and rewrites the index.
//!
//! Library shape:
//!
//! - [`MemoryFile`] — one parsed entry (path + frontmatter + body).
//! - [`Embedder`] / [`Merger`] traits — production wires the live
//!   embedder + Sonnet driver; tests use the deterministic
//!   in-memory fakes shipped here.
//! - [`Plan`] — declarative inputs (project slug, threshold,
//!   thresholds, `apply` flag).
//! - [`run`] — orchestrator that returns a [`Report`] describing
//!   every cluster + the side effects (or, in dry-run, the side
//!   effects that *would* have happened).
//!
//! The CLI binary lives in `cortex-ops` (cortex-cli's `cortex-ops`
//! bin) so the operator surface stays a single CLI.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Auto-memory frontmatter type — one of the four canonical kinds
/// the auto-memory system uses. Files with a missing or unknown
/// `type` are excluded from clustering with a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// `type: user`.
    User,
    /// `type: feedback`.
    Feedback,
    /// `type: project`.
    Project,
    /// `type: reference`.
    Reference,
}

impl MemoryType {
    /// Lowercase string matching the YAML enum tag.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }
    /// Parse the `type:` value from frontmatter.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "user" => Some(MemoryType::User),
            "feedback" => Some(MemoryType::Feedback),
            "project" => Some(MemoryType::Project),
            "reference" => Some(MemoryType::Reference),
            _ => None,
        }
    }
}

/// Parsed frontmatter for one memory file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    /// Memory `name` field — short identifier, also used as the
    /// MEMORY.md hyperlink text.
    pub name: String,
    /// One-line description used by Claude to score memory
    /// relevance during retrieval.
    pub description: String,
    /// Memory `type` enum.
    pub kind: MemoryType,
}

/// One memory file on disk.
#[derive(Debug, Clone)]
pub struct MemoryFile {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Filename only (`feedback_real_tests.md`).
    pub filename: String,
    /// Parsed frontmatter.
    pub frontmatter: Frontmatter,
    /// Body bytes after the closing `---`.
    pub body: String,
}

/// Errors produced by the discovery layer.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// I/O error around the memory directory.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Memory directory is missing entirely.
    #[error("memory directory not found: {0}")]
    MissingDir(PathBuf),
}

/// Resolve the Claude Code project slug for a working tree path.
/// Mirrors Claude Code's slug rule: every `:`, `/`, or `\\`
/// character becomes a single `-`. On Windows, drive paths like
/// `e:\HiveLLM\Cortex` therefore produce `e--HiveLLM-Cortex` (the
/// colon and the trailing backslash each contribute one dash) —
/// matching the `~/.claude/projects/<slug>/` directory Claude Code
/// actually creates. Trailing separators are stripped first so
/// `e:/HiveLLM/Cortex/` and `e:/HiveLLM/Cortex` produce the same
/// slug.
pub fn resolve_project_slug(cwd: &Path) -> String {
    let mut s = cwd.to_string_lossy().into_owned();
    while s.ends_with('/') || s.ends_with('\\') {
        s.pop();
    }
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            ':' | '/' | '\\' => out.push('-'),
            other => out.push(other),
        }
    }
    out
}

/// Locate the memory directory for `project_slug` under the user's
/// `~/.claude/projects/`.
pub fn memory_dir_for(home: &Path, project_slug: &str) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(project_slug)
        .join("memory")
}

/// Walk `dir` and parse every `*.md` other than `MEMORY.md` (and
/// anything inside `_archive/`). Files whose frontmatter is missing
/// or invalid are returned in the second component so the CLI can
/// surface them as warnings without dropping them silently.
pub fn read_memory_dir(
    dir: &Path,
) -> Result<(Vec<MemoryFile>, Vec<(PathBuf, String)>), DiscoveryError> {
    if !dir.exists() {
        return Err(DiscoveryError::MissingDir(dir.to_path_buf()));
    }
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if filename.eq_ignore_ascii_case("MEMORY.md") {
            continue;
        }
        if !filename.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        let body = fs::read_to_string(&path)?;
        match parse_memory_body(&body) {
            Ok((fm, body)) => files.push(MemoryFile {
                path: path.clone(),
                filename,
                frontmatter: fm,
                body,
            }),
            Err(e) => warnings.push((path, e)),
        }
    }
    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok((files, warnings))
}

/// Parse YAML frontmatter from the head of a memory file. Strict:
/// requires the leading `---\n`, a closing `---\n`, and the three
/// required keys (`name`, `description`, `type`).
pub fn parse_memory_body(input: &str) -> Result<(Frontmatter, String), String> {
    // Normalise line endings up front so the byte-offset arithmetic
    // below works on Windows checkouts too.
    let normalised = input.replace("\r\n", "\n");
    let mut rest = normalised.as_str();
    rest = rest
        .strip_prefix("---\n")
        .ok_or_else(|| "frontmatter: missing leading ---".to_string())?;
    let close_idx = rest
        .find("\n---\n")
        .or_else(|| rest.find("\n---"))
        .ok_or_else(|| "frontmatter: missing closing ---".to_string())?;
    let header = &rest[..close_idx];
    // Body starts after the closing `---` line + optional newline.
    let body_start = close_idx + match rest[close_idx..].starts_with("\n---\n") {
        true => "\n---\n".len(),
        false => "\n---".len(),
    };
    let body = rest[body_start..].trim_start_matches('\n').to_string();

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut kind: Option<MemoryType> = None;
    for raw in header.split('\n') {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        match key {
            "name" => name = Some(strip_quotes(value).to_string()),
            "description" => description = Some(strip_quotes(value).to_string()),
            "type" => kind = MemoryType::parse(strip_quotes(value)),
            _ => {}
        }
    }
    let name = name.ok_or_else(|| "frontmatter: name missing".to_string())?;
    let description =
        description.ok_or_else(|| "frontmatter: description missing".to_string())?;
    let kind = kind.ok_or_else(|| "frontmatter: type missing or invalid".to_string())?;
    Ok((
        Frontmatter {
            name,
            description,
            kind,
        },
        body,
    ))
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Re-serialize a memory file body (frontmatter + blank line + body).
pub fn render_memory_body(fm: &Frontmatter, body: &str) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("name: ");
    out.push_str(&fm.name);
    out.push('\n');
    out.push_str("description: ");
    out.push_str(&fm.description);
    out.push('\n');
    out.push_str("type: ");
    out.push_str(fm.kind.as_str());
    out.push('\n');
    out.push_str("---\n\n");
    out.push_str(body.trim_end());
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out
}

// ---- Embedding + clustering ---------------------------------------

/// Trait the consolidator calls to embed text. Production wires the
/// live embedder; tests + offline runs use [`HashingEmbedder`].
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Return a unit-normalised embedding for `text`.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}

/// Deterministic embedder: hashes 4-byte windows of the lowercased
/// input into a `D`-dimensional bag. Two texts with overlapping
/// 4-grams produce overlapping vectors so cosine similarity tracks
/// surface n-gram overlap. This is intentionally cheap so the CLI
/// stays runnable offline; production replaces it with an SDK call.
pub struct HashingEmbedder {
    /// Embedding dimensionality (`D`). Defaults to 256.
    pub dim: usize,
}

impl HashingEmbedder {
    /// Embedder with `dim` bins.
    pub fn with_dim(dim: usize) -> Self {
        Self { dim: dim.max(8) }
    }
}

impl Default for HashingEmbedder {
    fn default() -> Self {
        Self::with_dim(256)
    }
}

#[async_trait]
impl Embedder for HashingEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let mut vec = vec![0.0f32; self.dim];
        let lower = text.to_ascii_lowercase();
        let bytes = lower.as_bytes();
        if bytes.len() < 4 {
            // Single bin for tiny inputs.
            vec[0] = 1.0;
            return Ok(vec);
        }
        for window in bytes.windows(4) {
            let h = fnv1a_64(window);
            let bin = (h as usize) % self.dim;
            vec[bin] += 1.0;
        }
        // Unit-normalise so cosine == dot product.
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }
        Ok(vec)
    }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Cosine similarity between two unit-norm vectors. Falls back to
/// `0.0` on length mismatch so the caller never panics on a bad
/// embedder.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f32>()
}

/// One cluster from the matcher.
#[derive(Debug, Clone)]
pub struct Cluster {
    /// Memory type all members share.
    pub kind: MemoryType,
    /// Index into the input file list for each member.
    pub members: Vec<usize>,
}

/// Greedy clustering inside each `type` bucket. For every unassigned
/// file the matcher attaches it to the highest-similarity existing
/// cluster whose representative is ≥ `threshold`; otherwise the file
/// starts a new cluster of size 1.
pub fn cluster_files(
    files: &[MemoryFile],
    embeddings: &[Vec<f32>],
    threshold: f32,
) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut by_type: BTreeMap<MemoryType, Vec<usize>> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() {
        by_type.entry(f.frontmatter.kind).or_default().push(i);
    }
    for (kind, ids) in by_type {
        let mut local: Vec<Cluster> = Vec::new();
        for id in ids {
            let mut best: Option<(usize, f32)> = None;
            for (ci, c) in local.iter().enumerate() {
                let rep = c.members[0];
                let s = cosine(&embeddings[id], &embeddings[rep]);
                if s >= threshold && best.map_or(true, |(_, bs)| s > bs) {
                    best = Some((ci, s));
                }
            }
            match best {
                Some((ci, _)) => local[ci].members.push(id),
                None => local.push(Cluster {
                    kind,
                    members: vec![id],
                }),
            }
        }
        clusters.extend(local);
    }
    clusters
}

// ---- Sonnet merge with conflict guard -----------------------------

/// One merged entry produced by [`Merger`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedMemory {
    /// Frontmatter for the new file.
    pub frontmatter: Frontmatter,
    /// Markdown body for the new file.
    pub body: String,
}

/// Merge errors. The orchestrator translates these into a
/// per-cluster `skipped` reason in the report.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum MergeError {
    /// The merge agent failed to produce a parseable response.
    #[error("agent: {0}")]
    Agent(String),
    /// Conflict guard: re-embedding the merged body produced a
    /// cosine < 0.6 against at least one source body.
    #[error("merge drifted too far from source bodies (min cosine={min:.2})")]
    DriftedTooFar {
        /// Lowest source-to-merged cosine.
        min: f32,
    },
}

/// Trait the consolidator calls to merge clusters. Production wires
/// the Sonnet CLI driver; tests use [`RuleMerger`].
#[async_trait]
pub trait Merger: Send + Sync {
    /// Merge one cluster into one [`MergedMemory`]. Implementations
    /// own how they format the prompt and parse the model output.
    async fn merge(&self, cluster: &[&MemoryFile]) -> Result<MergedMemory, MergeError>;
}

/// Deterministic in-process merger used by tests + offline runs:
/// concatenates the cluster's bodies into one with a "Merged from N
/// entries" header, takes the first member's `name` / `description`,
/// and tags the merged frontmatter with the shared `type`. Production
/// callers swap this for a Sonnet-backed implementation.
pub struct RuleMerger;

#[async_trait]
impl Merger for RuleMerger {
    async fn merge(&self, cluster: &[&MemoryFile]) -> Result<MergedMemory, MergeError> {
        if cluster.is_empty() {
            return Err(MergeError::Agent("empty cluster".into()));
        }
        let kind = cluster[0].frontmatter.kind;
        if !cluster.iter().all(|m| m.frontmatter.kind == kind) {
            return Err(MergeError::Agent("cluster type mismatch".into()));
        }
        let head = cluster[0];
        let mut body = String::new();
        body.push_str(&format!(
            "_Consolidated from {} entries on {}._\n\n",
            cluster.len(),
            Utc::now().format("%Y-%m-%d")
        ));
        for m in cluster {
            body.push_str(&format!("### {}\n", m.frontmatter.name));
            body.push_str(&format!("{}\n\n", m.body.trim()));
        }
        Ok(MergedMemory {
            frontmatter: Frontmatter {
                name: head.frontmatter.name.clone(),
                description: head.frontmatter.description.clone(),
                kind,
            },
            body,
        })
    }
}

/// Drift-guard: re-embed the merged body and compare to every source.
pub async fn guard_drift(
    merged: &MergedMemory,
    cluster: &[&MemoryFile],
    embedder: &dyn Embedder,
    floor: f32,
) -> Result<(), MergeError> {
    let merged_vec = embedder
        .embed(&merged.body)
        .await
        .map_err(MergeError::Agent)?;
    let mut min: f32 = 1.0;
    for m in cluster {
        let src = embedder.embed(&m.body).await.map_err(MergeError::Agent)?;
        let s = cosine(&merged_vec, &src);
        if s < min {
            min = s;
        }
    }
    if min < floor {
        Err(MergeError::DriftedTooFar { min })
    } else {
        Ok(())
    }
}

// ---- Plan + report + run ------------------------------------------

/// Consolidator plan inputs.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Reference time for archive directory naming.
    pub now: DateTime<Utc>,
    /// Cosine cutoff for greedy clustering. Default 0.78.
    pub threshold: f32,
    /// Minimum source-to-merged cosine for the drift guard. Default 0.6.
    pub drift_floor: f32,
    /// Maximum clusters to merge per run (CLI cap to control spend).
    /// `None` means all.
    pub max_clusters: Option<usize>,
    /// `false` = dry-run (preview only). `true` = mutate filesystem.
    pub apply: bool,
}

impl Plan {
    /// Defaults — `threshold=0.78`, `drift_floor=0.6`, dry-run.
    pub fn default_for(now: DateTime<Utc>) -> Self {
        Self {
            now,
            threshold: 0.78,
            drift_floor: 0.6,
            max_clusters: None,
            apply: false,
        }
    }
}

/// Per-cluster outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum ClusterOutcome {
    /// Single-file cluster — left untouched.
    Singleton,
    /// Merge succeeded; `consolidated_filename` is the new file
    /// (whether or not it was actually written depends on `apply`).
    Merged {
        /// `consolidated_<short-hash>.md`.
        consolidated_filename: String,
        /// Merged frontmatter.
        frontmatter: Frontmatter,
    },
    /// Merge attempted but the drift guard rejected it.
    SkippedDrift {
        /// Lowest source-to-merged cosine observed.
        min_cosine: f32,
    },
    /// Merge attempted but the merger errored.
    SkippedAgentError {
        /// Free-form reason from the merger.
        reason: String,
    },
}

/// One cluster row in the report.
#[derive(Debug, Clone)]
pub struct ClusterReport {
    /// Memory type the cluster shares.
    pub kind: MemoryType,
    /// Source filenames that participated (in clustering order).
    pub members: Vec<String>,
    /// What happened.
    pub outcome: ClusterOutcome,
}

/// Top-level run report.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Files discovered before clustering.
    pub files_in: usize,
    /// Files surviving after a successful run (`apply=false` reports
    /// what *would* survive).
    pub files_out: usize,
    /// One row per cluster (singletons included so the operator can
    /// see the full scan).
    pub clusters: Vec<ClusterReport>,
    /// Files we couldn't parse (frontmatter missing/invalid).
    pub warnings: Vec<(PathBuf, String)>,
    /// `true` when [`Plan::apply`] was set and side effects ran.
    pub applied: bool,
    /// Archive directory the originals moved into (when applied).
    pub archive_dir: Option<PathBuf>,
}

/// Run the consolidator end-to-end against `dir`.
pub async fn run(
    dir: &Path,
    plan: &Plan,
    embedder: &dyn Embedder,
    merger: &dyn Merger,
) -> Result<Report, DiscoveryError> {
    let (files, warnings) = read_memory_dir(dir)?;
    let mut report = Report {
        files_in: files.len(),
        warnings,
        ..Report::default()
    };
    if files.is_empty() {
        report.files_out = 0;
        return Ok(report);
    }
    // Embed every body once.
    let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(files.len());
    for f in &files {
        let v = embedder
            .embed(&f.body)
            .await
            .unwrap_or_else(|_| vec![0.0; 1]);
        embeddings.push(v);
    }
    let clusters = cluster_files(&files, &embeddings, plan.threshold);
    let mut merged_outputs: Vec<(Vec<usize>, MergedMemory, String)> = Vec::new();
    let mut singletons_kept: Vec<usize> = Vec::new();
    let mut clusters_merged_count: usize = 0;
    for c in clusters {
        if c.members.len() == 1 {
            singletons_kept.push(c.members[0]);
            report.clusters.push(ClusterReport {
                kind: c.kind,
                members: vec![files[c.members[0]].filename.clone()],
                outcome: ClusterOutcome::Singleton,
            });
            continue;
        }
        if let Some(cap) = plan.max_clusters {
            if clusters_merged_count >= cap {
                // Treat overflow clusters as singletons so the
                // operator can re-run later with a larger cap.
                for &m in &c.members {
                    singletons_kept.push(m);
                }
                report.clusters.push(ClusterReport {
                    kind: c.kind,
                    members: c.members.iter().map(|i| files[*i].filename.clone()).collect(),
                    outcome: ClusterOutcome::SkippedAgentError {
                        reason: "max-clusters cap reached".into(),
                    },
                });
                continue;
            }
        }
        let cluster_refs: Vec<&MemoryFile> = c.members.iter().map(|i| &files[*i]).collect();
        let merged = match merger.merge(&cluster_refs).await {
            Ok(m) => m,
            Err(MergeError::Agent(reason)) => {
                for &i in &c.members {
                    singletons_kept.push(i);
                }
                report.clusters.push(ClusterReport {
                    kind: c.kind,
                    members: c.members.iter().map(|i| files[*i].filename.clone()).collect(),
                    outcome: ClusterOutcome::SkippedAgentError { reason },
                });
                continue;
            }
            Err(MergeError::DriftedTooFar { min }) => {
                for &i in &c.members {
                    singletons_kept.push(i);
                }
                report.clusters.push(ClusterReport {
                    kind: c.kind,
                    members: c.members.iter().map(|i| files[*i].filename.clone()).collect(),
                    outcome: ClusterOutcome::SkippedDrift { min_cosine: min },
                });
                continue;
            }
        };
        if let Err(e) =
            guard_drift(&merged, &cluster_refs, embedder, plan.drift_floor).await
        {
            for &i in &c.members {
                singletons_kept.push(i);
            }
            let outcome = match e {
                MergeError::DriftedTooFar { min } => ClusterOutcome::SkippedDrift {
                    min_cosine: min,
                },
                MergeError::Agent(reason) => ClusterOutcome::SkippedAgentError { reason },
            };
            report.clusters.push(ClusterReport {
                kind: c.kind,
                members: c.members.iter().map(|i| files[*i].filename.clone()).collect(),
                outcome,
            });
            continue;
        }
        // Successful merge — derive the consolidated filename.
        let body_rendered = render_memory_body(&merged.frontmatter, &merged.body);
        let short_hash = short_hash(&body_rendered);
        let filename = format!("consolidated_{short_hash}.md");
        report.clusters.push(ClusterReport {
            kind: c.kind,
            members: c.members.iter().map(|i| files[*i].filename.clone()).collect(),
            outcome: ClusterOutcome::Merged {
                consolidated_filename: filename.clone(),
                frontmatter: merged.frontmatter.clone(),
            },
        });
        merged_outputs.push((c.members, merged, filename));
        clusters_merged_count += 1;
    }
    // Compute survivors regardless of `apply` so dry-run output is
    // honest about post-state size.
    report.files_out = singletons_kept.len() + merged_outputs.len();

    if plan.apply {
        let archive_dir = dir.join("_archive").join(plan.now.format("%Y-%m-%dT%H-%M-%SZ").to_string());
        if !merged_outputs.is_empty() {
            fs::create_dir_all(&archive_dir)?;
        }
        for (members, merged, filename) in &merged_outputs {
            for &i in members {
                let src = &files[i].path;
                let dst = archive_dir.join(&files[i].filename);
                fs::rename(src, &dst)?;
            }
            let body = render_memory_body(&merged.frontmatter, &merged.body);
            fs::write(dir.join(filename), body)?;
        }
        // Regenerate MEMORY.md.
        let mut survivors: Vec<(String, String, String)> = Vec::new();
        for &i in &singletons_kept {
            let f = &files[i];
            survivors.push((
                f.frontmatter.name.clone(),
                f.filename.clone(),
                f.frontmatter.description.clone(),
            ));
        }
        for (_, merged, filename) in &merged_outputs {
            survivors.push((
                merged.frontmatter.name.clone(),
                filename.clone(),
                merged.frontmatter.description.clone(),
            ));
        }
        survivors.sort_by(|a, b| a.1.cmp(&b.1));
        let memory_md_path = dir.join("MEMORY.md");
        fs::write(&memory_md_path, render_index(&survivors))?;
        report.applied = true;
        report.archive_dir = Some(archive_dir);
    }
    Ok(report)
}

fn short_hash(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    hex
}

/// Render the `MEMORY.md` index from `(name, filename, description)`
/// tuples — one line per entry, capped at 150 chars, no frontmatter
/// on the index itself.
pub fn render_index(entries: &[(String, String, String)]) -> String {
    let mut out = String::new();
    for (name, filename, description) in entries {
        let mut line = format!("- [{name}]({filename}) — {description}");
        if line.len() > 150 {
            line.truncate(150);
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn write_memory(dir: &Path, filename: &str, kind: &str, body: &str) {
        let path = dir.join(filename);
        let mut f = fs::File::create(&path).unwrap();
        let frontmatter = format!(
            "---\nname: {}\ndescription: stub description\ntype: {}\n---\n\n{}\n",
            filename.trim_end_matches(".md"),
            kind,
            body
        );
        f.write_all(frontmatter.as_bytes()).unwrap();
    }

    #[test]
    fn slug_replaces_drive_colon_with_double_dash_and_separator_with_single() {
        // Matches `~/.claude/projects/e--HiveLLM-Cortex/memory/` —
        // the directory Claude Code creates on Windows for this repo.
        assert_eq!(resolve_project_slug(Path::new("e:/HiveLLM/Cortex")), "e--HiveLLM-Cortex");
        assert_eq!(resolve_project_slug(Path::new("e:\\HiveLLM\\Cortex")), "e--HiveLLM-Cortex");
    }

    #[test]
    fn slug_strips_trailing_separators() {
        assert_eq!(resolve_project_slug(Path::new("/repo/")), "-repo");
    }

    #[test]
    fn parse_memory_body_extracts_frontmatter_and_body() {
        let raw = "---\nname: foo\ndescription: bar baz\ntype: feedback\n---\n\nbody line\n";
        let (fm, body) = parse_memory_body(raw).unwrap();
        assert_eq!(fm.name, "foo");
        assert_eq!(fm.description, "bar baz");
        assert_eq!(fm.kind, MemoryType::Feedback);
        assert_eq!(body.trim(), "body line");
    }

    #[test]
    fn parse_memory_body_strips_quotes_around_values() {
        let raw =
            "---\nname: \"quoted name\"\ndescription: 'desc'\ntype: project\n---\n\nbody\n";
        let (fm, _) = parse_memory_body(raw).unwrap();
        assert_eq!(fm.name, "quoted name");
        assert_eq!(fm.description, "desc");
        assert_eq!(fm.kind, MemoryType::Project);
    }

    #[test]
    fn parse_memory_body_rejects_missing_close_marker() {
        let raw = "---\nname: foo\ndescription: bar\ntype: user\n";
        assert!(parse_memory_body(raw).is_err());
    }

    #[test]
    fn parse_memory_body_rejects_unknown_type() {
        let raw = "---\nname: foo\ndescription: bar\ntype: bogus\n---\nbody\n";
        assert!(parse_memory_body(raw).is_err());
    }

    #[test]
    fn read_memory_dir_skips_index_and_collects_warnings() {
        let dir = tempfile::tempdir().unwrap();
        write_memory(dir.path(), "alpha.md", "feedback", "alpha body");
        // MEMORY.md must be skipped (no frontmatter).
        fs::write(dir.path().join("MEMORY.md"), "- [alpha](alpha.md) — stub").unwrap();
        // Bogus file with no frontmatter.
        fs::write(dir.path().join("invalid.md"), "no frontmatter at all").unwrap();
        let (files, warnings) = read_memory_dir(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "alpha.md");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].1.contains("frontmatter"));
    }

    #[tokio::test]
    async fn hashing_embedder_returns_unit_norm_vectors() {
        let e = HashingEmbedder::default();
        let v = e.embed("the quick brown fox").await.unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3);
    }

    #[tokio::test]
    async fn cosine_self_similarity_is_one() {
        let e = HashingEmbedder::default();
        let v = e.embed("hello world").await.unwrap();
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-3);
    }

    fn synth_files(rows: &[(&str, &str, &str)]) -> Vec<MemoryFile> {
        rows.iter()
            .map(|(name, kind, body)| MemoryFile {
                path: PathBuf::from(format!("/tmp/{name}.md")),
                filename: format!("{name}.md"),
                frontmatter: Frontmatter {
                    name: (*name).to_string(),
                    description: "stub".into(),
                    kind: MemoryType::parse(kind).unwrap(),
                },
                body: (*body).to_string(),
            })
            .collect()
    }

    async fn embed_all(files: &[MemoryFile]) -> Vec<Vec<f32>> {
        let e = HashingEmbedder::default();
        let mut out = Vec::new();
        for f in files {
            out.push(e.embed(&f.body).await.unwrap());
        }
        out
    }

    #[tokio::test]
    async fn cluster_groups_near_duplicates_within_same_type() {
        let files = synth_files(&[
            ("a", "feedback", "Always run integration tests against a real database not mocks"),
            ("b", "feedback", "Always run integration tests against the real database (no mocks)"),
            ("c", "feedback", "Always run integration tests against a real database; no mocks"),
            ("d", "project", "Cortex pipeline state is stable"),
        ]);
        let embeddings = embed_all(&files).await;
        let clusters = cluster_files(&files, &embeddings, 0.78);
        // Three feedback rows cluster together; project row stands alone.
        let big = clusters
            .iter()
            .find(|c| c.members.len() == 3)
            .expect("expected one cluster of 3");
        assert!(big.members.iter().all(|i| files[*i].frontmatter.kind == MemoryType::Feedback));
        assert!(clusters.iter().any(|c| c.members.len() == 1
            && files[c.members[0]].frontmatter.kind == MemoryType::Project));
    }

    #[tokio::test]
    async fn cluster_never_mixes_types() {
        let files = synth_files(&[
            ("a", "feedback", "identical body identical body identical body"),
            ("b", "project", "identical body identical body identical body"),
        ]);
        let embeddings = embed_all(&files).await;
        let clusters = cluster_files(&files, &embeddings, 0.5);
        // Two singleton clusters: matcher MUST refuse cross-type even
        // when bodies are byte-identical.
        assert_eq!(clusters.len(), 2);
        assert!(clusters.iter().all(|c| c.members.len() == 1));
    }

    #[tokio::test]
    async fn dry_run_leaves_directory_untouched() {
        let dir = tempfile::tempdir().unwrap();
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            write_memory(
                dir.path(),
                &format!("{name}.md"),
                "feedback",
                &format!("near duplicate body number {i} sharing many tokens"),
            );
        }
        let plan = Plan::default_for(now());
        let report = run(dir.path(), &plan, &HashingEmbedder::default(), &RuleMerger)
            .await
            .unwrap();
        assert_eq!(report.files_in, 3);
        assert!(!report.applied);
        // Filesystem unchanged.
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn apply_archives_originals_and_writes_consolidated() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c"].iter() {
            write_memory(
                dir.path(),
                &format!("{name}.md"),
                "feedback",
                "always run integration tests against a real database not mocks",
            );
        }
        let mut plan = Plan::default_for(now());
        plan.apply = true;
        plan.threshold = 0.5;
        let report = run(dir.path(), &plan, &HashingEmbedder::default(), &RuleMerger)
            .await
            .unwrap();
        assert!(report.applied);
        // Originals are gone from `dir`.
        assert!(!dir.path().join("a.md").exists());
        assert!(!dir.path().join("b.md").exists());
        assert!(!dir.path().join("c.md").exists());
        // Archive contains them.
        let archive = report.archive_dir.unwrap();
        assert!(archive.join("a.md").exists());
        assert!(archive.join("b.md").exists());
        assert!(archive.join("c.md").exists());
        // One consolidated_*.md exists.
        let consolidated: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("consolidated_")
            })
            .collect();
        assert_eq!(consolidated.len(), 1);
        // MEMORY.md regenerated.
        let index = fs::read_to_string(dir.path().join("MEMORY.md")).unwrap();
        assert_eq!(index.lines().count(), 1);
    }

    #[tokio::test]
    async fn re_run_after_apply_finds_no_clusters() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a", "b"].iter() {
            write_memory(
                dir.path(),
                &format!("{name}.md"),
                "feedback",
                "always run integration tests against a real database not mocks",
            );
        }
        let mut plan = Plan::default_for(now());
        plan.apply = true;
        plan.threshold = 0.5;
        let _ = run(dir.path(), &plan, &HashingEmbedder::default(), &RuleMerger)
            .await
            .unwrap();
        // Second run.
        let plan2 = Plan {
            apply: true,
            now: now(),
            ..Plan::default_for(now())
        };
        let report = run(dir.path(), &plan2, &HashingEmbedder::default(), &RuleMerger)
            .await
            .unwrap();
        assert_eq!(
            report
                .clusters
                .iter()
                .filter(|c| matches!(c.outcome, ClusterOutcome::Merged { .. }))
                .count(),
            0
        );
    }

    /// Drift-injecting merger — produces a body unrelated to its
    /// inputs so the guard rejects it.
    struct DriftMerger;
    #[async_trait]
    impl Merger for DriftMerger {
        async fn merge(&self, cluster: &[&MemoryFile]) -> Result<MergedMemory, MergeError> {
            Ok(MergedMemory {
                frontmatter: cluster[0].frontmatter.clone(),
                body: "totally unrelated body about cooking pasta".into(),
            })
        }
    }

    #[tokio::test]
    async fn drifted_merge_is_rejected_and_originals_remain() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a", "b"].iter() {
            write_memory(
                dir.path(),
                &format!("{name}.md"),
                "feedback",
                "always run integration tests against a real database not mocks",
            );
        }
        let mut plan = Plan::default_for(now());
        plan.apply = true;
        plan.threshold = 0.5;
        let report = run(dir.path(), &plan, &HashingEmbedder::default(), &DriftMerger)
            .await
            .unwrap();
        // Cluster is reported as skipped-drift.
        assert!(report
            .clusters
            .iter()
            .any(|c| matches!(c.outcome, ClusterOutcome::SkippedDrift { .. })));
        // Originals remain in place.
        assert!(dir.path().join("a.md").exists());
        assert!(dir.path().join("b.md").exists());
    }

    #[test]
    fn render_index_caps_each_line_at_150_chars() {
        let entries = vec![
            (
                "name".into(),
                "file.md".into(),
                "x".repeat(500),
            ),
        ];
        let index = render_index(&entries);
        let line = index.lines().next().unwrap();
        assert!(line.len() <= 150, "line too long: {}", line.len());
    }

    #[test]
    fn render_memory_body_round_trips_through_parser() {
        let fm = Frontmatter {
            name: "feedback_real_tests".into(),
            description: "always run integration tests against a real database".into(),
            kind: MemoryType::Feedback,
        };
        let body = "Long form body explaining why.\n\nSecond paragraph.";
        let serialised = render_memory_body(&fm, body);
        let (parsed_fm, parsed_body) = parse_memory_body(&serialised).unwrap();
        assert_eq!(parsed_fm, fm);
        assert_eq!(parsed_body.trim(), body.trim());
    }
}
