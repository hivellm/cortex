//! Phase11k §5.1 — `cortex-bootstrap --graph-static` mode.
//!
//! Walks every selected repo, runs the per-language code analyzers
//! and the markdown analyzer, builds [`GraphPatch`] entries via the
//! resolver, and writes one canonical envelope per analyzed file
//! to the zstd-NDJSON archive sink that
//! [`cortex_api::archive_loader`] re-reads at boot. Synap, the
//! runner, and the checkpoint logic are bypassed entirely — this
//! mode is a side pass whose only side-effect is the archive
//! partition tree.
//!
//! Wire shape per envelope:
//!
//! ```text
//! Envelope {
//!   kind: Artifact,
//!   stream: Bootstrap,
//!   tool: "cortex-bootstrap-graph-static",
//!   payload: ArtifactPayload {
//!     artifact_type: "file",
//!     path, language, body, size,
//!     metadata: { graph_patch: <serialised GraphPatch>,
//!                 analyzer_version: "phase11k.1" }
//!   },
//!   content_hash: sha256(canonical(payload)),
//!   ...
//! }
//! ```
//!
//! Carrying the patch on `payload.metadata` keeps the wire shape
//! identical to a regular `artifact.code` / `artifact.doc`
//! envelope — the graph-worker (§5.2) reads the patch from
//! metadata and applies it without re-running the analyzer.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use cortex_core::canonical_json::canonicalize;
use cortex_ingestion::archive::ArchiveWriter;
use cortex_storage::external_repos::ExternalRepoIndex;
use cortex_workers::graph::analyzer::{
    build_graph_patch, AnalyzerLanguage, CodeAnalyzer, GoAnalyzer, PatchBuildContext,
    PythonAnalyzer, RustAnalyzer, TypescriptAnalyzer,
};
use cortex_workers::graph::markdown::{is_markdown_path, MarkdownAnalyzer};
use cortex_workers::graph::resolver::{
    build_rust_module_map_from_root, LocalSymbols, ModuleMap, PackageMap, SymbolResolver,
};
use ignore::WalkBuilder;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use super::config::CortexSection;
use super::walker::{walk_repo, WalkEntry};

/// Stream tag every graph-static archive partition gets. Matches
/// the `archive_loader` prefix-match (`bootstrap-*`) so the
/// envelopes seed the keyword lane on the next daemon boot.
pub const GRAPH_STATIC_STREAM_TAG: &str = "bootstrap-graph-static";

/// Static-extraction version stamped on every emitted patch. The
/// coalescer (§5.4) dedupes by `(content_hash, analyzer_version)`,
/// so bumping this string forces the next pass to re-emit even
/// when content has not changed.
pub const GRAPH_STATIC_ANALYZER_VERSION: &str = "phase11k.1";

/// Default external-repos TOML path (relative to the repo root).
/// Matches the convention `cortex_storage::external_repos` documents.
const EXTERNAL_REPOS_TOML: &str = ".rulebook/external_repos.toml";

/// Per-repo report of one graph-static pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphStaticReport {
    /// Repo identifier the pass walked.
    pub repo_id: String,
    /// Files visited (accepted by the walker).
    pub files_visited: u64,
    /// Files matched by an analyzer language.
    pub files_analyzed: u64,
    /// Total edges across every emitted patch.
    pub edges_emitted: u64,
    /// Total nodes across every emitted patch.
    pub nodes_emitted: u64,
    /// Envelopes successfully written to the archive sink.
    pub envelopes_written: u64,
    /// Wall-clock duration in seconds.
    pub duration_secs: f64,
}

/// Run the graph-static pass for one repo. Reads files from disk,
/// dispatches to the per-language analyzer + markdown analyzer,
/// resolves edges through [`SymbolResolver`], and writes one
/// canonical envelope per analyzed file via `writer`.
pub fn run_repo_graph_static(
    repo_id: &str,
    repo_root: &Path,
    cfg: &CortexSection,
    writer: &dyn ArchiveWriter,
) -> Result<GraphStaticReport> {
    let started = std::time::Instant::now();
    let mut report = GraphStaticReport {
        repo_id: repo_id.to_string(),
        ..Default::default()
    };

    let module_map = build_repo_module_map(repo_id, repo_root);
    let package_map = build_repo_package_map(repo_root);

    let session_id = format!("graph-static-{}", Ulid::new());
    let entries = walk_repo(repo_root, cfg);
    let bodies = read_bodies(&entries, repo_root);
    let hash_index = build_hash_index(&bodies);

    for (rel_path, body) in &bodies {
        report.files_visited = report.files_visited.saturating_add(1);
        let Some(kind) = analyzer_kind_for(rel_path) else {
            continue;
        };
        let edges = match kind {
            AnalyzerKind::Rust => RustAnalyzer::new().extract(body, repo_id, rel_path),
            AnalyzerKind::Typescript => TypescriptAnalyzer::new().extract(body, repo_id, rel_path),
            AnalyzerKind::Python => PythonAnalyzer::new().extract(body, repo_id, rel_path),
            AnalyzerKind::Go => GoAnalyzer::new().extract(body, repo_id, rel_path),
            AnalyzerKind::Markdown => MarkdownAnalyzer::new().extract(body, repo_id, rel_path),
        };
        report.files_analyzed = report.files_analyzed.saturating_add(1);

        let language = kind.local_language();
        let local_symbols = LocalSymbols::new(repo_id, language, rel_path.clone());
        let resolver = SymbolResolver::new(&module_map, &package_map, &local_symbols);
        let content_hash = sha256_canonical_text(body);
        let event_id = Ulid::new().to_string();
        let lookup = |repo: &str, path: &str| -> Option<String> {
            hash_index
                .get(&(repo.to_string(), path.to_string()))
                .cloned()
        };
        let ctx = PatchBuildContext {
            source_repo: repo_id,
            source_path: rel_path,
            source_content_hash: &content_hash,
            source_event_id: Some(&event_id),
            resolver: &resolver,
            content_hash_for: &lookup,
            analyzer_version: GRAPH_STATIC_ANALYZER_VERSION,
        };
        let patch = build_graph_patch(&edges, &ctx);
        report.edges_emitted = report
            .edges_emitted
            .saturating_add(patch.edges.len() as u64);
        report.nodes_emitted = report
            .nodes_emitted
            .saturating_add(patch.nodes.len() as u64);

        let envelope = build_envelope(
            &event_id,
            &session_id,
            repo_id,
            rel_path,
            kind,
            body,
            &patch,
        )?;
        writer
            .write(GRAPH_STATIC_STREAM_TAG, &envelope)
            .with_context(|| format!("write graph-static envelope for {rel_path}"))?;
        report.envelopes_written = report.envelopes_written.saturating_add(1);
    }

    report.duration_secs = started.elapsed().as_secs_f64();
    Ok(report)
}

/// Static-extraction analyzer pick for a path. `None` when the path
/// doesn't match any analyzer the runner ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalyzerKind {
    Rust,
    Typescript,
    Python,
    Go,
    Markdown,
}

impl AnalyzerKind {
    /// Snake-case label carried on `LocalSymbols` + the envelope's
    /// `payload.language`.
    fn local_language(self) -> &'static str {
        match self {
            AnalyzerKind::Rust => "rust",
            AnalyzerKind::Typescript => "typescript",
            AnalyzerKind::Python => "python",
            AnalyzerKind::Go => "go",
            AnalyzerKind::Markdown => "markdown",
        }
    }
}

fn analyzer_kind_for(rel_path: &str) -> Option<AnalyzerKind> {
    if is_markdown_path(rel_path) {
        return Some(AnalyzerKind::Markdown);
    }
    match AnalyzerLanguage::from_path_extension(rel_path)? {
        AnalyzerLanguage::Rust => Some(AnalyzerKind::Rust),
        AnalyzerLanguage::Typescript => Some(AnalyzerKind::Typescript),
        AnalyzerLanguage::Python => Some(AnalyzerKind::Python),
        AnalyzerLanguage::Go => Some(AnalyzerKind::Go),
    }
}

/// Best-effort module map: walk every `**/src/lib.rs` and
/// `**/src/main.rs` under `repo_root`, build a per-crate module
/// map, and merge them into one workspace-wide map. Path slots on
/// the produced [`ModuleEntry`] entries are rewritten to the
/// repo-relative form so `:Artifact` natural keys round-trip
/// against what the analyzer emits.
fn build_repo_module_map(repo_id: &str, repo_root: &Path) -> ModuleMap {
    let mut merged = ModuleMap::new();
    if !repo_root.exists() {
        return merged;
    }
    let prefix = repo_root.to_string_lossy().replace('\\', "/");
    let mut walker = WalkBuilder::new(repo_root);
    walker.standard_filters(true).hidden(false);
    for entry in walker.build().flatten() {
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !metadata.is_file() {
            continue;
        }
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name != "lib.rs" && name != "main.rs" {
            continue;
        }
        // Only treat files under a `src/` directory as crate roots.
        let parent_is_src = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            == Some("src");
        if !parent_is_src {
            continue;
        }
        let Ok(map) = build_rust_module_map_from_root(repo_id, path) else {
            continue;
        };
        for e in map.iter() {
            let mut rewritten = e.clone();
            rewritten.artifact_path = strip_repo_prefix(&rewritten.artifact_path, &prefix);
            merged.insert(rewritten);
        }
    }
    merged
}

fn strip_repo_prefix(absolute: &str, repo_prefix: &str) -> String {
    let cleaned = absolute.replace('\\', "/");
    let with_slash = format!("{repo_prefix}/");
    if let Some(rest) = cleaned.strip_prefix(&with_slash) {
        rest.to_string()
    } else if cleaned == repo_prefix {
        String::new()
    } else {
        cleaned
    }
}

/// Build a [`PackageMap`] from every `Cargo.toml` under `repo_root`
/// plus the optional `.rulebook/external_repos.toml` registry.
fn build_repo_package_map(repo_root: &Path) -> PackageMap {
    let mut deps: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if repo_root.exists() {
        let mut walker = WalkBuilder::new(repo_root);
        walker.standard_filters(true).hidden(false);
        for entry in walker.build().flatten() {
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !metadata.is_file() {
                continue;
            }
            if entry.file_name() != "Cargo.toml" {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                collect_cargo_dep_names(&text, &mut deps);
            }
        }
    }
    let index =
        ExternalRepoIndex::load_from_file(&repo_root.join(EXTERNAL_REPOS_TOML)).unwrap_or_default();
    PackageMap::from_workspace(deps, &index)
}

fn collect_cargo_dep_names(toml_text: &str, out: &mut std::collections::BTreeSet<String>) {
    let parsed: Value = match toml::from_str::<Value>(toml_text) {
        Ok(v) => v,
        Err(_) => return,
    };
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(Value::Object(map)) = parsed.get(table) {
            for k in map.keys() {
                out.insert(k.clone());
            }
        }
    }
    if let Some(Value::Object(workspace)) = parsed.get("workspace") {
        if let Some(Value::Object(deps)) = workspace.get("dependencies") {
            for k in deps.keys() {
                out.insert(k.clone());
            }
        }
    }
}

fn read_bodies(entries: &[WalkEntry], repo_root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in entries {
        let WalkEntry::Accepted { rel_path, .. } = entry else {
            continue;
        };
        let abs = repo_root.join(rel_path);
        match std::fs::read_to_string(&abs) {
            Ok(body) => out.push((rel_path.clone(), body)),
            Err(err) => {
                tracing::debug!(
                    path = %abs.display(),
                    error = %err,
                    "graph-static: skip unreadable file"
                );
            }
        }
    }
    out
}

fn build_hash_index(bodies: &[(String, String)]) -> BTreeMap<(String, String), String> {
    // The patch builder's lookup is keyed on `(repo, path)`. The
    // graph-static pass is repo-local — every entry shares the same
    // repo identifier, so the outer key collapses to a `String`-only
    // index in practice. We still carry the tuple so the lookup
    // closure type matches `ContentHashLookup`.
    let mut idx = BTreeMap::new();
    for (path, body) in bodies {
        idx.insert((String::new(), path.clone()), sha256_canonical_text(body));
    }
    idx
}

fn sha256_canonical_text(body: &str) -> String {
    // Hash the raw file body. The patch builder treats this hash as
    // opaque — its only requirement is determinism per body.
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn build_envelope(
    event_id: &str,
    session_id: &str,
    repo_id: &str,
    rel_path: &str,
    kind: AnalyzerKind,
    body: &str,
    patch: &cortex_workers::graph::patch::GraphPatch,
) -> Result<Value> {
    let mut metadata: Map<String, Value> = Map::new();
    metadata.insert("graph_patch".into(), serde_json::to_value(patch)?);
    metadata.insert(
        "analyzer_version".into(),
        Value::String(GRAPH_STATIC_ANALYZER_VERSION.into()),
    );
    metadata.insert(
        "graph_static_stream_tag".into(),
        Value::String(GRAPH_STATIC_STREAM_TAG.into()),
    );
    let payload = json!({
        "artifact_type": "file",
        "path": rel_path,
        "language": kind.local_language(),
        "body": body,
        "size": body.len() as u64,
        "metadata": Value::Object(metadata),
    });
    let canonical = canonicalize(&payload).context("canonicalise graph-static payload")?;
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    let digest = hasher.finalize();
    let mut content_hash = String::with_capacity(7 + 64);
    content_hash.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut content_hash, "{byte:02x}");
    }

    let mut context = Map::new();
    context.insert("repo".into(), Value::String(repo_id.into()));
    context.insert(
        "platform".into(),
        Value::String(std::env::consts::OS.to_string()),
    );

    Ok(json!({
        "event_id": event_id,
        "schema_version": "1",
        "occurred_at": Utc::now().to_rfc3339(),
        "session_id": session_id,
        "stream": "bootstrap",
        "tool": "cortex-bootstrap-graph-static",
        "kind": "artifact",
        "context": Value::Object(context),
        "payload": payload,
        "content_hash": content_hash,
    }))
}

/// On-disk archive constructor used by the binary main. Wraps
/// [`cortex_ingestion::archive::NdJsonZstdArchive`] at the supplied
/// root with a sensible default zstd level.
pub fn open_archive_writer(root: &Path) -> std::sync::Arc<dyn ArchiveWriter> {
    std::sync::Arc::new(cortex_ingestion::archive::NdJsonZstdArchive::new(
        root.to_path_buf(),
        3,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_ingestion::archive::InMemoryArchive;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, contents: &str) -> PathBuf {
        let abs = dir.join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&abs, contents).unwrap();
        abs
    }

    fn cfg() -> CortexSection {
        // Default cortex.toml `[cortex]` block — `walk_repo` only
        // touches `cfg.exclude.*`.
        CortexSection::default()
    }

    #[test]
    fn analyzer_kind_routes_known_extensions() {
        assert_eq!(analyzer_kind_for("src/lib.rs"), Some(AnalyzerKind::Rust));
        assert_eq!(
            analyzer_kind_for("docs/spec.md"),
            Some(AnalyzerKind::Markdown)
        );
        assert_eq!(
            analyzer_kind_for("frontend/app.tsx"),
            Some(AnalyzerKind::Typescript)
        );
        assert_eq!(analyzer_kind_for("README.txt"), None);
        assert_eq!(analyzer_kind_for("data.json"), None);
    }

    #[test]
    fn strip_repo_prefix_normalises_separators() {
        assert_eq!(
            strip_repo_prefix("/tmp/repo/src/lib.rs", "/tmp/repo"),
            "src/lib.rs"
        );
        assert_eq!(
            strip_repo_prefix(r"C:\tmp\repo\src\lib.rs", "C:/tmp/repo"),
            "src/lib.rs"
        );
        assert_eq!(strip_repo_prefix("not-rooted", "/tmp/repo"), "not-rooted");
    }

    #[test]
    fn collect_cargo_dep_names_sees_dependency_tables() {
        let toml = r#"
[package]
name = "x"

[dependencies]
serde = "1"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
proptest = "1"

[build-dependencies]
cc = "1"

[workspace.dependencies]
shared-thing = { workspace = true }
"#;
        let mut deps = std::collections::BTreeSet::new();
        collect_cargo_dep_names(toml, &mut deps);
        assert!(deps.contains("serde"));
        assert!(deps.contains("tokio"));
        assert!(deps.contains("proptest"));
        assert!(deps.contains("cc"));
        assert!(deps.contains("shared-thing"));
    }

    #[test]
    fn graph_static_emits_one_envelope_per_analyzed_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        write_file(
            root,
            "Cargo.toml",
            "[package]\nname = \"x\"\n[dependencies]\ntokio = \"1\"\n",
        );
        write_file(root, "src/lib.rs", "use tokio::spawn;\npub fn outer() {}\n");
        write_file(root, "docs/spec.md", "# Title\n\nbody\n");
        write_file(root, "extras/notes.txt", "ignored\n");

        let writer = Arc::new(InMemoryArchive::default());
        let report = run_repo_graph_static("repo", root, &cfg(), writer.as_ref()).expect("run");

        // Both lib.rs and spec.md should produce one envelope each.
        assert_eq!(report.envelopes_written, 2, "report: {report:?}");
        assert_eq!(report.files_analyzed, 2);
        let rows = writer.rows();
        assert_eq!(rows.len(), 2);
        for (tag, env) in &rows {
            assert_eq!(tag, GRAPH_STATIC_STREAM_TAG);
            assert_eq!(env.get("kind").and_then(|v| v.as_str()), Some("artifact"));
            assert_eq!(
                env.get("tool").and_then(|v| v.as_str()),
                Some("cortex-bootstrap-graph-static")
            );
            assert!(env.get("payload").is_some());
            assert!(env
                .get("payload")
                .and_then(|p| p.get("metadata"))
                .and_then(|m| m.get("graph_patch"))
                .is_some());
            assert!(
                env.get("payload")
                    .and_then(|p| p.get("metadata"))
                    .and_then(|m| m.get("analyzer_version"))
                    .and_then(|v| v.as_str())
                    == Some(GRAPH_STATIC_ANALYZER_VERSION)
            );
        }
    }

    #[test]
    fn graph_static_envelope_carries_resolved_external_edge() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write_file(
            root,
            "Cargo.toml",
            "[package]\nname = \"x\"\n[dependencies]\ntokio = \"1\"\n",
        );
        write_file(root, "src/lib.rs", "use tokio::spawn;\n");

        let writer = Arc::new(InMemoryArchive::default());
        run_repo_graph_static("repo", root, &cfg(), writer.as_ref()).unwrap();

        let rows = writer.rows();
        let lib = rows
            .iter()
            .find(|(_, env)| {
                env.get("payload")
                    .and_then(|p| p.get("path"))
                    .and_then(|s| s.as_str())
                    == Some("src/lib.rs")
            })
            .expect("lib envelope");
        let edges = lib
            .1
            .get("payload")
            .and_then(|p| p.get("metadata"))
            .and_then(|m| m.get("graph_patch"))
            .and_then(|p| p.get("edges"))
            .and_then(|e| e.as_array())
            .expect("edges array");
        assert!(
            edges
                .iter()
                .any(
                    |e| e.get("edge_type").and_then(|v| v.as_str()) == Some("IMPORTS_EXTERNAL")
                        && e.get("to_key").and_then(|v| v.as_str()) == Some("tokio")
                ),
            "expected IMPORTS_EXTERNAL → tokio in {edges:?}"
        );
    }

    #[test]
    fn graph_static_skips_unanalyzable_files() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "data.json", "{}\n");
        write_file(root, "img.png", "binary-ish\n");

        let writer = Arc::new(InMemoryArchive::default());
        let report = run_repo_graph_static("repo", root, &cfg(), writer.as_ref()).unwrap();
        assert_eq!(report.files_analyzed, 0);
        assert_eq!(report.envelopes_written, 0);
        assert!(writer.rows().is_empty());
    }

    #[test]
    fn graph_static_envelope_content_hash_is_deterministic() {
        // Two runs against the same fixture must produce envelopes
        // with identical `content_hash` (modulo the per-run event_id
        // / occurred_at fields the archive wire-shape carries).
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "src/lib.rs", "pub fn a() {}\n");

        let w1 = Arc::new(InMemoryArchive::default());
        run_repo_graph_static("repo", root, &cfg(), w1.as_ref()).unwrap();
        let w2 = Arc::new(InMemoryArchive::default());
        run_repo_graph_static("repo", root, &cfg(), w2.as_ref()).unwrap();

        let hash1 = w1.rows()[0].1["content_hash"].as_str().unwrap().to_string();
        let hash2 = w2.rows()[0].1["content_hash"].as_str().unwrap().to_string();
        assert_eq!(hash1, hash2);
    }
}
