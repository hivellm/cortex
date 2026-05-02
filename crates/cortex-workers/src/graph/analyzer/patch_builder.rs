//! Phase11k §1.4 — bridge between [`CodeEdge`] (analyzer output) and
//! [`GraphPatch`] / [`EdgeOp`] (the wire format the graph writer
//! consumes).
//!
//! The analyzer leaves three things unresolved:
//!
//! 1. **Source artifact content_hash.** The analyzer trait (`extract`)
//!    sees `(source, repo, path)` only — content_hash is owned by the
//!    enclosing [`crate::embedder::EnrichedEvent`]. Every
//!    [`CodeEdge::from_node`] whose label is `Artifact` carries the
//!    *logical* `repo|path` key produced by
//!    [`super::artifact_logical_key`]; the patch builder rewrites
//!    those to the canonical `repo|path|content_hash` triple via
//!    [`crate::graph::identity::artifact_natural_key`].
//! 2. **Target dispatch.** Every [`ResolutionTarget`] is run through
//!    the [`crate::graph::resolver::SymbolResolver`]; the resolved
//!    node decides the final `EdgeOp`'s shape.
//! 3. **Cross-artifact content_hash.** When tier-2 produces a target
//!    `:Artifact` (the file that owns the resolved symbol), the
//!    builder asks the caller-supplied [`ContentHashLookup`] for that
//!    file's current hash. A miss falls through to a sentinel
//!    `*` marker so the upsert is still idempotent under repeat
//!    runs against the same workspace state.
//!
//! Schema invariants the builder relies on (added in
//! [`crate::graph::schema::SCHEMA_STATEMENTS`]):
//!
//! - `:ExternalPackage` keyed on `natural_key`.
//! - `:UnresolvedImport` keyed on `natural_key`.
//! - `:Artifact` keyed on the composite `repo|path|content_hash`.
//! - `:Symbol` keyed on `repo|language|qualified_name`.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::{artifact_logical_key, CodeEdge, EdgeType, NodeRef};
use crate::graph::identity::artifact_natural_key;
use crate::graph::patch::{EdgeOp, GraphPatch, NodeOp};
use crate::graph::resolver::{ResolutionTier, ResolvedTarget, SymbolResolver};

/// Sentinel content-hash component used when the cross-artifact
/// hash is not known. Keeps `:Artifact` upserts idempotent without
/// forging a real hash.
pub const UNKNOWN_CONTENT_HASH: &str = "*";

/// Callback shape — `(repo, path) → Option<content_hash>`. The patch
/// builder holds a reference to one of these so it can fold target-
/// artifact hashes into [`EdgeOp::to_key`].
pub type ContentHashLookup<'a> = dyn Fn(&str, &str) -> Option<String> + 'a;

/// Per-call context for a single source artifact.
pub struct PatchBuildContext<'a> {
    /// Repo identifier the *source* artifact lives in.
    pub source_repo: &'a str,
    /// Repo-relative path of the source artifact.
    pub source_path: &'a str,
    /// Source artifact's content hash. Used to rebuild the canonical
    /// natural key (see [`artifact_natural_key`]).
    pub source_content_hash: &'a str,
    /// Optional event id stamped on every produced [`EdgeOp::props`]
    /// so the stale-edge sweeper (§5.3) can find edges whose source
    /// event is no longer current.
    pub source_event_id: Option<&'a str>,
    /// Resolver bound to the workspace's module + package maps.
    pub resolver: &'a SymbolResolver<'a>,
    /// Cross-artifact content-hash lookup. Returning `None` is fine —
    /// the builder substitutes [`UNKNOWN_CONTENT_HASH`] so the wire
    /// shape stays valid.
    pub content_hash_for: &'a ContentHashLookup<'a>,
    /// Static-extraction version stamp. Carried on the
    /// [`EdgeOp::props`] under the key `analyzer_version` so the
    /// coalescer (§5.4) can dedupe by `(content_hash,
    /// analyzer_version)`.
    pub analyzer_version: &'a str,
}

/// Convert one analyzer batch into a [`GraphPatch`]. Idempotent
/// against the same `(source_content_hash, analyzer_version)` because
/// every node + edge upsert keys on natural identity and never
/// timestamps the props.
pub fn build_graph_patch(edges: &[CodeEdge], ctx: &PatchBuildContext<'_>) -> GraphPatch {
    let mut patch = GraphPatch::empty();
    let source_artifact_key =
        artifact_natural_key(ctx.source_repo, ctx.source_path, ctx.source_content_hash);
    upsert_artifact_node(
        &mut patch,
        ctx.source_repo,
        ctx.source_path,
        ctx.source_content_hash,
    );

    for edge in edges {
        let from_key = rewrite_source_key(&edge.from_node, ctx, &source_artifact_key);
        let resolved = ctx.resolver.resolve(&edge.to_target);
        emit_edge(
            &mut patch,
            edge,
            &from_key,
            edge.from_node.label.as_str(),
            resolved,
            ctx,
        );
    }
    patch
}

fn rewrite_source_key(
    from: &NodeRef,
    ctx: &PatchBuildContext<'_>,
    canonical_artifact_key: &str,
) -> String {
    if from.label == "Artifact"
        && from.natural_key == artifact_logical_key(ctx.source_repo, ctx.source_path)
    {
        canonical_artifact_key.to_string()
    } else {
        from.natural_key.clone()
    }
}

fn emit_edge(
    patch: &mut GraphPatch,
    edge: &CodeEdge,
    from_key: &str,
    from_label: &str,
    resolved: ResolvedTarget,
    ctx: &PatchBuildContext<'_>,
) {
    match resolved {
        ResolvedTarget::Workspace {
            symbol,
            artifact,
            tier,
        } => {
            emit_workspace_edge(
                patch, edge, from_key, from_label, &symbol, &artifact, tier, ctx,
            );
        }
        ResolvedTarget::ExternalPackage { node } => {
            emit_external_edge(patch, edge, from_key, from_label, &node, ctx);
        }
        ResolvedTarget::Unresolved { hint } => {
            emit_unresolved_edge(patch, edge, from_key, from_label, &hint, ctx);
        }
        ResolvedTarget::PreResolved { node } => {
            emit_preresolved_edge(patch, edge, from_key, from_label, &node, ctx);
        }
    }
}

fn emit_preresolved_edge(
    patch: &mut GraphPatch,
    edge: &CodeEdge,
    from_key: &str,
    from_label: &str,
    node: &NodeRef,
    ctx: &PatchBuildContext<'_>,
) {
    let final_key = if node.label == "Artifact" {
        let canonical = canonicalize_artifact_key(&node.natural_key, ctx);
        upsert_artifact_from_key(patch, &canonical);
        canonical
    } else {
        upsert_node(patch, &node.label, &node.natural_key, |props| {
            props.insert("name".into(), Value::String(node.natural_key.clone()));
        });
        node.natural_key.clone()
    };
    patch.edges.push(make_edge(
        edge.edge_type,
        from_label,
        from_key,
        &node.label,
        &final_key,
        ResolutionTier::IntraCrate,
        edge,
        ctx,
    ));
}

fn emit_workspace_edge(
    patch: &mut GraphPatch,
    edge: &CodeEdge,
    from_key: &str,
    from_label: &str,
    symbol: &NodeRef,
    artifact: &NodeRef,
    tier: ResolutionTier,
    ctx: &PatchBuildContext<'_>,
) {
    upsert_node(patch, &symbol.label, &symbol.natural_key, |props| {
        props.insert(
            "qualified_name".into(),
            Value::String(extract_qualified_name(&symbol.natural_key)),
        );
    });

    let canonical_artifact_key = canonicalize_artifact_key(&artifact.natural_key, ctx);
    upsert_artifact_from_key(patch, &canonical_artifact_key);

    let (target_label, target_key) = match edge.edge_type {
        EdgeType::ImportsFile
        | EdgeType::ReExports
        | EdgeType::LinksTo
        | EdgeType::Documents
        | EdgeType::DescribesPath
        | EdgeType::Cites
        | EdgeType::DerivedFrom => ("Artifact", canonical_artifact_key.as_str()),
        EdgeType::Calls
        | EdgeType::UsesType
        | EdgeType::Implements
        | EdgeType::Extends
        | EdgeType::Mentions
        | EdgeType::DocstringReferences
        | EdgeType::ImportsExternal
        | EdgeType::UnresolvedImport
        | EdgeType::LinksToSection
        | EdgeType::DocumentedBy
        | EdgeType::Contains => (symbol.label.as_str(), symbol.natural_key.as_str()),
    };
    patch.edges.push(make_edge(
        edge.edge_type,
        from_label,
        from_key,
        target_label,
        target_key,
        tier,
        edge,
        ctx,
    ));
}

fn emit_external_edge(
    patch: &mut GraphPatch,
    edge: &CodeEdge,
    from_key: &str,
    from_label: &str,
    node: &NodeRef,
    ctx: &PatchBuildContext<'_>,
) {
    upsert_node(patch, &node.label, &node.natural_key, |props| {
        props.insert("name".into(), Value::String(node.natural_key.clone()));
    });

    let edge_type = match edge.edge_type {
        EdgeType::ImportsFile => EdgeType::ImportsExternal,
        other => other,
    };
    patch.edges.push(make_edge(
        edge_type,
        from_label,
        from_key,
        &node.label,
        &node.natural_key,
        ResolutionTier::External,
        edge,
        ctx,
    ));
}

fn emit_unresolved_edge(
    patch: &mut GraphPatch,
    edge: &CodeEdge,
    from_key: &str,
    from_label: &str,
    hint: &str,
    ctx: &PatchBuildContext<'_>,
) {
    let label = "UnresolvedImport";
    upsert_node(patch, label, hint, |props| {
        props.insert("hint".into(), Value::String(hint.to_string()));
    });
    let edge_type = match edge.edge_type {
        EdgeType::ImportsFile | EdgeType::ImportsExternal | EdgeType::ReExports => {
            EdgeType::UnresolvedImport
        }
        other => other,
    };
    patch.edges.push(make_edge(
        edge_type,
        from_label,
        from_key,
        label,
        hint,
        ResolutionTier::Unresolved,
        edge,
        ctx,
    ));
}

fn make_edge(
    edge_type: EdgeType,
    from_label: &str,
    from_key: &str,
    to_label: &str,
    to_key: &str,
    tier: ResolutionTier,
    edge: &CodeEdge,
    ctx: &PatchBuildContext<'_>,
) -> EdgeOp {
    let mut props: BTreeMap<String, Value> = BTreeMap::new();
    props.insert("kind".into(), Value::String(edge.kind.to_string()));
    props.insert("tier".into(), Value::String(tier_label(tier).into()));
    if let Some(line) = edge.source_line {
        props.insert("source_line".into(), json!(line));
    }
    props.insert(
        "analyzer_version".into(),
        Value::String(ctx.analyzer_version.to_string()),
    );
    if let Some(event_id) = ctx.source_event_id {
        props.insert(
            "source_event_id".into(),
            Value::String(event_id.to_string()),
        );
    }
    EdgeOp {
        edge_type: edge_type.label().to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        props,
    }
}

fn tier_label(tier: ResolutionTier) -> &'static str {
    match tier {
        ResolutionTier::LocalFile => "local_file",
        ResolutionTier::IntraCrate => "intra_crate",
        ResolutionTier::External => "external",
        ResolutionTier::Unresolved => "unresolved",
    }
}

fn upsert_node(
    patch: &mut GraphPatch,
    label: &str,
    natural_key: &str,
    fill_props: impl FnOnce(&mut BTreeMap<String, Value>),
) {
    if patch
        .nodes
        .iter()
        .any(|n| n.label == label && n.natural_key == natural_key)
    {
        return;
    }
    let mut props: BTreeMap<String, Value> = BTreeMap::new();
    fill_props(&mut props);
    let mut node = NodeOp::with_identity(label, natural_key);
    node.props = props;
    patch.nodes.push(node);
}

fn upsert_artifact_node(patch: &mut GraphPatch, repo: &str, path: &str, content_hash: &str) {
    let key = artifact_natural_key(repo, path, content_hash);
    upsert_node(patch, "Artifact", &key, |props| {
        props.insert("repo".into(), Value::String(repo.to_string()));
        props.insert("path".into(), Value::String(path.to_string()));
        props.insert(
            "content_hash".into(),
            Value::String(content_hash.to_string()),
        );
    });
}

fn upsert_artifact_from_key(patch: &mut GraphPatch, natural_key: &str) {
    upsert_node(patch, "Artifact", natural_key, |props| {
        if let Some((repo, path, hash)) = split_artifact_key(natural_key) {
            props.insert("repo".into(), Value::String(repo));
            props.insert("path".into(), Value::String(path));
            props.insert("content_hash".into(), Value::String(hash));
        }
    });
}

fn canonicalize_artifact_key(raw: &str, ctx: &PatchBuildContext<'_>) -> String {
    let parts: Vec<&str> = raw.splitn(3, '|').collect();
    match parts.len() {
        2 => {
            let repo = parts[0];
            let path = parts[1];
            let hash = if repo == ctx.source_repo && path == ctx.source_path {
                ctx.source_content_hash.to_string()
            } else {
                (ctx.content_hash_for)(repo, path)
                    .unwrap_or_else(|| UNKNOWN_CONTENT_HASH.to_string())
            };
            artifact_natural_key(repo, path, &hash)
        }
        3 => raw.to_string(),
        _ => raw.to_string(),
    }
}

fn split_artifact_key(key: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = key.splitn(3, '|').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ))
}

fn extract_qualified_name(symbol_key: &str) -> String {
    // Symbol keys: `repo|language|qualified_name`. The qualified name
    // is the third pipe-delimited slot; everything past the second
    // pipe (verbatim) so `::` separators round-trip.
    let mut parts = symbol_key.splitn(3, '|');
    parts.next();
    parts.next();
    parts.next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::analyzer::{CodeAnalyzer, EdgeType, NodeRef, ResolutionTarget, RustAnalyzer};
    use crate::graph::resolver::{
        ExternalPackage, LocalSymbols, ModuleEntry, ModuleMap, PackageMap, SymbolResolver,
    };

    fn no_hash_lookup(_repo: &str, _path: &str) -> Option<String> {
        None
    }

    fn with_known_hash<'a>(
        map: &'a BTreeMap<(String, String), String>,
    ) -> impl Fn(&str, &str) -> Option<String> + 'a {
        move |repo: &str, path: &str| map.get(&(repo.to_string(), path.to_string())).cloned()
    }

    fn fixtures() -> (ModuleMap, PackageMap, LocalSymbols) {
        let mut mm = ModuleMap::new();
        mm.insert(ModuleEntry {
            repo: "cortex".into(),
            language: "rust",
            qualified_name: "crate::module_a::helper".into(),
            artifact_path: "src/module_a.rs".into(),
        });
        let mut pm = PackageMap::new();
        pm.insert(ExternalPackage {
            name: "tokio".into(),
            natural_key: "tokio".into(),
            local_path: None,
            crate_name: None,
        });
        let ls = LocalSymbols::new("cortex", "rust", "src/lib.rs");
        (mm, pm, ls)
    }

    fn run(edges: &[CodeEdge]) -> GraphPatch {
        let (mm, pm, ls) = fixtures();
        let resolver = SymbolResolver::new(&mm, &pm, &ls);
        let ctx = PatchBuildContext {
            source_repo: "cortex",
            source_path: "src/lib.rs",
            source_content_hash: "sha256:abc",
            source_event_id: Some("evt-1"),
            resolver: &resolver,
            content_hash_for: &no_hash_lookup,
            analyzer_version: "phase11k.1",
        };
        build_graph_patch(edges, &ctx)
    }

    #[test]
    fn source_artifact_node_is_emitted_with_canonical_key() {
        let p = run(&[]);
        assert_eq!(p.nodes.len(), 1);
        assert_eq!(p.nodes[0].label, "Artifact");
        assert_eq!(p.nodes[0].natural_key, "cortex|src/lib.rs|sha256:abc");
    }

    #[test]
    fn imports_file_workspace_emits_artifact_to_artifact_edge() {
        let edges =
            RustAnalyzer::new().extract("use crate::module_a::helper;\n", "cortex", "src/lib.rs");
        let p = run(&edges);
        let imports: Vec<&EdgeOp> = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "IMPORTS_FILE")
            .collect();
        assert_eq!(imports.len(), 1);
        let e = imports[0];
        assert_eq!(e.from_label, "Artifact");
        assert_eq!(e.from_key, "cortex|src/lib.rs|sha256:abc");
        assert_eq!(e.to_label, "Artifact");
        assert_eq!(e.to_key, "cortex|src/module_a.rs|*");
        assert_eq!(
            e.props.get("tier").and_then(|v| v.as_str()),
            Some("intra_crate")
        );
        assert_eq!(
            e.props.get("source_event_id").and_then(|v| v.as_str()),
            Some("evt-1")
        );
    }

    #[test]
    fn imports_file_external_package_rewrites_to_imports_external() {
        let edges = RustAnalyzer::new().extract("use tokio::spawn;\n", "cortex", "src/lib.rs");
        let p = run(&edges);
        let imports: Vec<&EdgeOp> = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "IMPORTS_EXTERNAL")
            .collect();
        assert_eq!(imports.len(), 1);
        let e = imports[0];
        assert_eq!(e.to_label, "ExternalPackage");
        assert_eq!(e.to_key, "tokio");
        assert!(p
            .nodes
            .iter()
            .any(|n| n.label == "ExternalPackage" && n.natural_key == "tokio"));
    }

    #[test]
    fn imports_file_unresolved_falls_back_to_unresolved_import() {
        let edges =
            RustAnalyzer::new().extract("use no_such_crate::Frob;\n", "cortex", "src/lib.rs");
        let p = run(&edges);
        let imports: Vec<&EdgeOp> = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "UNRESOLVED_IMPORT")
            .collect();
        assert_eq!(imports.len(), 1);
        let e = imports[0];
        assert_eq!(e.to_label, "UnresolvedImport");
        assert_eq!(e.to_key, "no_such_crate::Frob");
        assert_eq!(
            e.props.get("tier").and_then(|v| v.as_str()),
            Some("unresolved")
        );
    }

    #[test]
    fn pub_use_resolved_keeps_re_exports_label() {
        let edges = RustAnalyzer::new().extract(
            "pub use crate::module_a::helper;\n",
            "cortex",
            "src/lib.rs",
        );
        let p = run(&edges);
        let re: Vec<&EdgeOp> = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "RE_EXPORTS")
            .collect();
        assert_eq!(re.len(), 1);
        assert_eq!(re[0].to_label, "Artifact");
    }

    #[test]
    fn calls_anchored_at_symbol_emit_calls_edge() {
        let edges =
            RustAnalyzer::new().extract("fn outer() { helper(); }\n", "cortex", "src/lib.rs");
        let p = run(&edges);
        let calls: Vec<&EdgeOp> = p.edges.iter().filter(|e| e.edge_type == "CALLS").collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from_label, "Symbol");
        assert_eq!(calls[0].to_key, "cortex|rust|crate::module_a::helper");
    }

    #[test]
    fn implements_emits_implements_edge_against_symbol() {
        let edges = RustAnalyzer::new().extract(
            "impl MyTrait for MyType { fn run(&self) {} }\n",
            "cortex",
            "src/lib.rs",
        );
        let p = run(&edges);
        let imp: Vec<&EdgeOp> = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "IMPLEMENTS")
            .collect();
        assert_eq!(imp.len(), 1);
        assert_eq!(imp[0].from_label, "Symbol");
        assert_eq!(imp[0].from_key, "cortex|rust|MyType");
        assert_eq!(imp[0].to_label, "UnresolvedImport");
    }

    #[test]
    fn known_target_hash_replaces_sentinel() {
        let mut hashes: BTreeMap<(String, String), String> = BTreeMap::new();
        hashes.insert(
            ("cortex".into(), "src/module_a.rs".into()),
            "sha256:def".into(),
        );
        let lookup = with_known_hash(&hashes);

        let (mm, pm, ls) = fixtures();
        let resolver = SymbolResolver::new(&mm, &pm, &ls);
        let ctx = PatchBuildContext {
            source_repo: "cortex",
            source_path: "src/lib.rs",
            source_content_hash: "sha256:abc",
            source_event_id: None,
            resolver: &resolver,
            content_hash_for: &lookup,
            analyzer_version: "phase11k.1",
        };
        let edges =
            RustAnalyzer::new().extract("use crate::module_a::helper;\n", "cortex", "src/lib.rs");
        let p = build_graph_patch(&edges, &ctx);
        let imp = p
            .edges
            .iter()
            .find(|e| e.edge_type == "IMPORTS_FILE")
            .expect("imports edge");
        assert_eq!(imp.to_key, "cortex|src/module_a.rs|sha256:def");
    }

    #[test]
    fn analyzer_version_stamped_on_every_edge() {
        let edges = RustAnalyzer::new().extract(
            "use tokio::spawn;\nfn outer() { helper(); }\n",
            "cortex",
            "src/lib.rs",
        );
        let p = run(&edges);
        for e in &p.edges {
            assert_eq!(
                e.props.get("analyzer_version").and_then(|v| v.as_str()),
                Some("phase11k.1")
            );
        }
    }

    #[test]
    fn empty_input_still_emits_source_artifact_only() {
        let p = run(&[]);
        assert_eq!(p.nodes.len(), 1);
        assert!(p.edges.is_empty());
    }

    #[test]
    fn manual_code_edge_with_logical_source_key_is_rewritten() {
        let edge = CodeEdge {
            from_node: NodeRef {
                label: "Artifact".into(),
                natural_key: artifact_logical_key("cortex", "src/lib.rs"),
            },
            edge_type: EdgeType::ImportsFile,
            to_target: ResolutionTarget::ExternalPackage {
                name: "serde".into(),
            },
            source_line: Some(1),
            kind: "use_decl",
        };
        let p = run(&[edge]);
        let e = p.edges.first().expect("one edge");
        assert_eq!(e.from_key, "cortex|src/lib.rs|sha256:abc");
    }
}
