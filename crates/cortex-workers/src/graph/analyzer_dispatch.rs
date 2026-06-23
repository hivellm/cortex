//! Phase11k §5.2 — live static-analyzer trigger for the graph worker.
//!
//! The live counterpart to the `cortex-bootstrap --graph-static` pass
//! (§5.1). Every time an [`crate::embedder::EnrichedEvent`] of kind
//! [`cortex_core::events::Kind::Artifact`] arrives at the graph worker
//! with a `content_hash` that differs from the last-seen hash for its
//! `(repo, path)` pair, [`run_if_changed`] runs the appropriate static
//! analyzer and returns a [`GraphPatch`] containing the resulting edges.
//!
//! The worker merges that patch into its structural patch list *before*
//! calling `write_patches`, so the writer's existing coalescer + dedup
//! machinery handles any overlap with the structural patch automatically.

use std::collections::BTreeMap;
use std::sync::Mutex;

use cortex_core::events::Kind;

use crate::embedder::EnrichedEvent;
use crate::graph::analyzer::{
    build_graph_patch, AnalyzerLanguage, CodeAnalyzer, GoAnalyzer, PatchBuildContext,
    PythonAnalyzer, RustAnalyzer, TypescriptAnalyzer,
};
use crate::graph::markdown::{is_markdown_path, MarkdownAnalyzer};
use crate::graph::patch::GraphPatch;
use crate::graph::resolver::{LocalSymbols, ModuleMap, PackageMap, SymbolResolver};

/// Static-extraction version stamp. Must match the bootstrap path
/// (`cortex_cli::bootstrap::graph_static::GRAPH_STATIC_ANALYZER_VERSION`)
/// so the coalescer can dedupe by `(content_hash, analyzer_version)` across
/// bootstrap and live passes.
pub const GRAPH_STATIC_ANALYZER_VERSION: &str = "phase11k.1";

/// Per-`(repo, path)` last-seen `content_hash` table. Allows the live
/// worker to skip analyzer re-runs when an event arrives with the same
/// content hash as the previously processed event for that file.
///
/// Wrapped in a `Mutex` so it can be shared behind an `Arc` across the
/// worker pool without additional synchronisation overhead in the caller.
#[derive(Debug, Default)]
pub struct AnalyzerState {
    /// Maps `(repo, path)` → last-seen `content_hash`.
    table: Mutex<BTreeMap<(String, String), String>>,
}

impl AnalyzerState {
    /// Construct an empty state table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `content_hash` was observed for `(repo, path)`.
    ///
    /// Returns `true` when the hash is new or different from the
    /// previously-stored value — meaning the analyzer should run.
    /// Returns `false` when the hash is identical to the last-seen
    /// value for this pair — meaning the analyzer output would be
    /// unchanged and the run can be skipped.
    pub fn observe(&self, repo: &str, path: &str, content_hash: &str) -> bool {
        let key = (repo.to_string(), path.to_string());
        match self.table.lock() {
            Ok(mut guard) => {
                let prev = guard.get(&key).map(|s| s.as_str()).unwrap_or("");
                if prev == content_hash {
                    return false;
                }
                guard.insert(key, content_hash.to_string());
                true
            }
            // Mutex poisoned — treat as changed so we don't silently
            // suppress the analyzer run.
            Err(_) => true,
        }
    }
}

/// Run the static analyzer for `event` **only** when the event's
/// `content_hash` differs from the last-seen hash for its `(repo, path)`
/// pair. Returns `None` when the hash is unchanged (nothing to do) or when
/// the event does not qualify for static analysis.
///
/// Qualifying conditions (all must hold):
/// - `event.kind == Kind::Artifact`
/// - `event.context_repo.is_some()`
/// - `event.context_path.is_some()`
/// - `event.redacted_payload["body"]` is a non-empty string
/// - path has a recognised analyzer extension OR is a markdown file
#[must_use]
pub fn run_if_changed(state: &AnalyzerState, event: &EnrichedEvent) -> Option<GraphPatch> {
    let repo = event.context_repo.as_deref()?;
    let path = event.context_path.as_deref()?;

    if !state.observe(repo, path, &event.content_hash) {
        return None;
    }

    extract_static_patch(event)
}

/// Run the static analyzer for `event` unconditionally (no hash-change
/// guard). Returns `None` when the event does not qualify for static
/// analysis.
///
/// Qualifying conditions are the same as [`run_if_changed`] minus the
/// hash-change check.
#[must_use]
pub fn extract_static_patch(event: &EnrichedEvent) -> Option<GraphPatch> {
    if event.kind != Kind::Artifact {
        return None;
    }

    let repo = event.context_repo.as_deref()?;
    let path = event.context_path.as_deref()?;

    let body = event.redacted_payload.get("body")?.as_str()?;

    // Decide analyzer from path extension; fall back to markdown check.
    let analyzer_kind = if let Some(lang) = AnalyzerLanguage::from_path_extension(path) {
        AnalyzerKind::Code(lang)
    } else if is_markdown_path(path) {
        AnalyzerKind::Markdown
    } else {
        return None;
    };

    let edges = match analyzer_kind {
        AnalyzerKind::Code(AnalyzerLanguage::Rust) => RustAnalyzer::new().extract(body, repo, path),
        AnalyzerKind::Code(AnalyzerLanguage::Typescript) => {
            TypescriptAnalyzer::new().extract(body, repo, path)
        }
        AnalyzerKind::Code(AnalyzerLanguage::Python) => {
            PythonAnalyzer::new().extract(body, repo, path)
        }
        AnalyzerKind::Code(AnalyzerLanguage::Go) => GoAnalyzer::new().extract(body, repo, path),
        AnalyzerKind::Markdown => MarkdownAnalyzer::new().extract(body, repo, path),
    };

    // Build a resolver with empty workspace-wide maps. The live worker does
    // not have a pre-built module or package map; tier-2 misses resolve to
    // `:UnresolvedImport`, which is the documented design for live events.
    let language = match analyzer_kind {
        AnalyzerKind::Code(lang) => lang.label(),
        AnalyzerKind::Markdown => "markdown",
    };
    let empty_module_map = ModuleMap::new();
    let empty_package_map = PackageMap::new();
    let local_symbols = LocalSymbols::new(repo, language, path);
    let resolver = SymbolResolver::new(&empty_module_map, &empty_package_map, &local_symbols);

    let no_hash_lookup = |_repo: &str, _path: &str| -> Option<String> { None };

    let ctx = PatchBuildContext {
        source_repo: repo,
        source_path: path,
        source_content_hash: &event.content_hash,
        source_event_id: Some(&event.event_id),
        resolver: &resolver,
        content_hash_for: &no_hash_lookup,
        analyzer_version: GRAPH_STATIC_ANALYZER_VERSION,
    };

    Some(build_graph_patch(&edges, &ctx))
}

/// Internal discriminator for the two analyzer families.
#[derive(Clone, Copy)]
enum AnalyzerKind {
    Code(AnalyzerLanguage),
    Markdown,
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::classifier::types::{ClassifierSource, PiiRisk, Severity};
    use crate::classifier::ClassifierOutput;
    use serde_json::json;

    fn fixture_classifier(event_id: &str) -> ClassifierOutput {
        ClassifierOutput {
            event_id: event_id.to_string(),
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
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn make_artifact_event(
        repo: &str,
        path: &str,
        body: &str,
        content_hash: &str,
    ) -> EnrichedEvent {
        let event_id = format!("evt-{}-{}", repo, path);
        EnrichedEvent {
            event_id: event_id.clone(),
            kind: Kind::Artifact,
            content_hash: content_hash.to_string(),
            redacted_payload: json!({ "body": body }),
            classifier: fixture_classifier(&event_id),
            context_repo: Some(repo.to_string()),
            context_path: Some(path.to_string()),
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
            class_level: None,
            class_compartments: None,
        }
    }

    fn make_turn_event() -> EnrichedEvent {
        let event_id = "evt-turn".to_string();
        EnrichedEvent {
            event_id: event_id.clone(),
            kind: Kind::Turn,
            content_hash: "sha256:aaa".to_string(),
            redacted_payload: json!({}),
            classifier: fixture_classifier(&event_id),
            context_repo: Some("cortex".to_string()),
            context_path: Some("src/lib.rs".to_string()),
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
            class_level: None,
            class_compartments: None,
        }
    }

    // ------------------------------------------------------------------ //
    // AnalyzerState::observe tests                                        //
    // ------------------------------------------------------------------ //

    #[test]
    fn state_observe_first_hash_returns_true() {
        let state = AnalyzerState::new();
        assert!(
            state.observe("cortex", "src/lib.rs", "sha256:abc"),
            "first observation must return true"
        );
    }

    #[test]
    fn state_observe_same_hash_returns_false() {
        let state = AnalyzerState::new();
        state.observe("cortex", "src/lib.rs", "sha256:abc");
        assert!(
            !state.observe("cortex", "src/lib.rs", "sha256:abc"),
            "repeat of the same hash must return false"
        );
    }

    #[test]
    fn state_observe_changed_hash_returns_true_and_updates() {
        let state = AnalyzerState::new();
        state.observe("cortex", "src/lib.rs", "sha256:abc");
        assert!(
            state.observe("cortex", "src/lib.rs", "sha256:def"),
            "new hash must return true"
        );
        // Confirm the table updated: same new hash → false.
        assert!(
            !state.observe("cortex", "src/lib.rs", "sha256:def"),
            "after update, same new hash must return false"
        );
    }

    // ------------------------------------------------------------------ //
    // extract_static_patch tests                                          //
    // ------------------------------------------------------------------ //

    #[test]
    fn extract_static_patch_returns_none_for_non_artifact() {
        let event = make_turn_event();
        assert!(
            extract_static_patch(&event).is_none(),
            "non-Artifact kind must yield None"
        );
    }

    #[test]
    fn extract_static_patch_returns_none_when_path_missing() {
        let mut event = make_artifact_event("cortex", "src/lib.rs", "fn foo() {}", "sha256:abc");
        event.context_path = None;
        assert!(
            extract_static_patch(&event).is_none(),
            "missing context_path must yield None"
        );
    }

    #[test]
    fn extract_static_patch_returns_none_when_body_missing() {
        let event_id = "evt-nobody".to_string();
        let event = EnrichedEvent {
            event_id: event_id.clone(),
            kind: Kind::Artifact,
            content_hash: "sha256:abc".to_string(),
            // payload has no "body" key
            redacted_payload: json!({ "language": "rust" }),
            classifier: fixture_classifier(&event_id),
            context_repo: Some("cortex".to_string()),
            context_path: Some("src/lib.rs".to_string()),
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
            class_level: None,
            class_compartments: None,
        };
        assert!(
            extract_static_patch(&event).is_none(),
            "missing body field must yield None"
        );
    }

    #[test]
    fn extract_static_patch_runs_rust_analyzer_on_rs_path() {
        let event = make_artifact_event("cortex", "src/lib.rs", "use tokio::spawn;", "sha256:abc");
        let patch = extract_static_patch(&event).expect("should produce a patch");
        // The Rust analyzer extracts a use-decl import for tokio.
        // With an empty PackageMap it falls through to UnresolvedImport.
        let has_import_edge = patch
            .edges
            .iter()
            .any(|e| e.edge_type == "UNRESOLVED_IMPORT" || e.edge_type == "IMPORTS_EXTERNAL");
        assert!(
            has_import_edge,
            "rust analyzer should emit an UNRESOLVED_IMPORT or IMPORTS_EXTERNAL edge for `use tokio::spawn;`"
        );
        // Source artifact node must be present.
        assert!(
            patch.nodes.iter().any(|n| n.label == "Artifact"),
            "patch must contain an Artifact node"
        );
    }

    #[test]
    fn extract_static_patch_runs_markdown_analyzer_on_md_path() {
        let event = make_artifact_event(
            "cortex",
            "docs/spec.md",
            "# Title\n[link](other.md)\n",
            "sha256:md1",
        );
        let patch = extract_static_patch(&event).expect("should produce a patch for markdown");
        // Markdown analyzer emits at least one DocSection node for "# Title".
        let has_doc_section = patch
            .nodes
            .iter()
            .any(|n| n.label == "DocSection" || n.label == "Artifact");
        assert!(
            has_doc_section,
            "markdown patch must contain at least one DocSection or Artifact node; got: {:?}",
            patch.nodes.iter().map(|n| &n.label).collect::<Vec<_>>()
        );
    }

    // ------------------------------------------------------------------ //
    // run_if_changed integration test                                     //
    // ------------------------------------------------------------------ //

    #[test]
    fn run_if_changed_returns_none_on_repeated_hash() {
        let state = AnalyzerState::new();
        let event = make_artifact_event("cortex", "src/lib.rs", "use tokio::spawn;", "sha256:abc");

        // First call: hash is new, should return Some.
        let first = run_if_changed(&state, &event);
        assert!(
            first.is_some(),
            "first call with a new hash must return Some"
        );

        // Second call: same hash, must return None.
        let second = run_if_changed(&state, &event);
        assert!(
            second.is_none(),
            "second call with the same hash must return None"
        );
    }
}
