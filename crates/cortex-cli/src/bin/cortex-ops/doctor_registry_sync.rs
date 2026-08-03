//! Phase29 (mcp-surface-doc-and-discovery §2) —
//! `cortex-ops doctor-registry-sync`.
//!
//! Spec 20's Registry table is the human half of the MCP surface;
//! `ToolRegistry::default_set()` is the runtime half. This check
//! parses the table's tool rows (first-column backticked names) and
//! compares names + count against the live registry, reporting
//! missing/extra names on either side.
//!
//! Exit codes (spec 20 "registry drift" requirement):
//! - `0` — in sync.
//! - `1` — drift of exactly one tool (warn).
//! - `2` — drift ≥ 2 tools (critical; blocks PRs via the
//!   registry-sync CI gate), or the spec file is unreadable.

use std::collections::BTreeSet;
use std::process::ExitCode;

/// Extract the tool names documented in spec 20's Registry table:
/// every table row whose first cell is a backticked `cortex_*` name.
/// Pure so the parser is unit-testable on fixture markdown.
pub(super) fn doc_tool_names(spec_markdown: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for line in spec_markdown.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('|') {
            continue;
        }
        let first_cell = trimmed.trim_start_matches('|').trim();
        if let Some(rest) = first_cell.strip_prefix('`') {
            if let Some(end) = rest.find('`') {
                let name = &rest[..end];
                if name.starts_with("cortex_") {
                    names.insert(name.to_string());
                }
            }
        }
    }
    names
}

/// Compare doc vs runtime name sets. Returns
/// `(missing_in_doc, missing_in_registry)` — both sorted.
pub(super) fn diff_names(
    doc: &BTreeSet<String>,
    runtime: &BTreeSet<String>,
) -> (Vec<String>, Vec<String>) {
    let missing_in_doc = runtime.difference(doc).cloned().collect();
    let missing_in_registry = doc.difference(runtime).cloned().collect();
    (missing_in_doc, missing_in_registry)
}

pub(super) fn doctor_registry_sync(spec_path: Option<String>, json: bool) -> ExitCode {
    let path = spec_path.unwrap_or_else(|| "docs/specs/20-mcp-tool-surface.md".to_string());
    let markdown = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("doctor-registry-sync: read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    let doc = doc_tool_names(&markdown);
    let runtime: BTreeSet<String> = cortex_mcp_server::tools::ToolRegistry::default_set()
        .names()
        .into_iter()
        .map(String::from)
        .collect();
    let (missing_in_doc, missing_in_registry) = diff_names(&doc, &runtime);
    let drift = missing_in_doc.len() + missing_in_registry.len();

    if json {
        let payload = serde_json::json!({
            "spec_path": path,
            "doc_count": doc.len(),
            "registry_count": runtime.len(),
            "missing_in_doc": missing_in_doc,
            "missing_in_registry": missing_in_registry,
            "drift": drift,
            "status": if drift == 0 { "ok" } else if drift == 1 { "warn" } else { "critical" },
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!("cortex-ops doctor-registry-sync");
        println!("  spec           = {path}");
        println!("  doc rows       = {}", doc.len());
        println!("  registry tools = {}", runtime.len());
        if drift == 0 {
            println!("  ok — registry table and runtime registry are in sync");
        } else {
            for n in &missing_in_doc {
                println!("  MISSING IN DOC:      {n}");
            }
            for n in &missing_in_registry {
                println!("  MISSING IN REGISTRY: {n}");
            }
            println!(
                "  DRIFT = {drift} ({})",
                if drift >= 2 {
                    "critical — blocks PRs"
                } else {
                    "warn"
                }
            );
        }
    }
    match drift {
        0 => ExitCode::SUCCESS,
        1 => ExitCode::from(1),
        _ => ExitCode::from(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_parser_extracts_backticked_first_cells_only() {
        let md = "\
# Spec\n\
| Tool | Args |\n\
|------|------|\n\
| `cortex_query` | `q` |\n\
| `cortex_forget` | `id` |\n\
| plain_row | `cortex_not_first_cell` |\n\
Some prose mentioning `cortex_prose_only`.\n";
        let names = doc_tool_names(md);
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            vec!["cortex_forget".to_string(), "cortex_query".to_string()]
        );
    }

    #[test]
    fn diff_reports_both_directions_sorted() {
        let doc: BTreeSet<String> = ["cortex_a", "cortex_b", "cortex_stale"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let runtime: BTreeSet<String> = ["cortex_a", "cortex_b", "cortex_new"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (missing_in_doc, missing_in_registry) = diff_names(&doc, &runtime);
        assert_eq!(missing_in_doc, vec!["cortex_new".to_string()]);
        assert_eq!(missing_in_registry, vec!["cortex_stale".to_string()]);
    }

    #[test]
    fn live_spec_and_registry_are_in_sync() {
        // The real invariant, run as a unit test so `cargo test`
        // catches drift even without the CI gate: parse the actual
        // spec 20 next to this workspace and compare with the actual
        // registry.
        let spec = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/specs/20-mcp-tool-surface.md"
        );
        let md = std::fs::read_to_string(spec).expect("read spec 20");
        let doc = doc_tool_names(&md);
        let runtime: BTreeSet<String> = cortex_mcp_server::tools::ToolRegistry::default_set()
            .names()
            .into_iter()
            .map(String::from)
            .collect();
        let (missing_in_doc, missing_in_registry) = diff_names(&doc, &runtime);
        assert!(
            missing_in_doc.is_empty() && missing_in_registry.is_empty(),
            "spec 20 registry table drifted — missing_in_doc: {missing_in_doc:?}, missing_in_registry: {missing_in_registry:?}"
        );
    }
}
