//! Phase18 §5.1 — `cortex-ops backfill-cross-project` subcommand.
//!
//! Thin operator wrapper over `cortex_workers::graph::cross_project`.
//! Scanning + edge-building + dedup live in that module; this file
//! only orchestrates the file I/O, Cypher writes, and report output.
//!
//! Two source families:
//!
//! 1. **Manifest pass** — `Cargo.toml`, `package.json`,
//!    `gui/package.json` → `Branch --CROSS_PROJECT_REF--> Branch`.
//! 2. **ADR-mention pass** — `.rulebook/decisions/*.md` →
//!    `Decision --CROSS_PROJECT_REF--> Branch` (MATCH-guarded; a
//!    missing Decision is a safe no-op).

use std::process::ExitCode;

use chrono::Utc;
use cortex_workers::graph::cross_project::{
    adr_mention_edge, dedup_edges, manifest_edge, parse_cargo_deps, parse_package_json,
    scan_version_mentions,
};

/// Phase18 §5.1 — entry point for `cortex-ops backfill-cross-project`.
pub(super) fn backfill_cross_project(
    root: String,
    project: String,
    nexus: Option<String>,
    dry_run: bool,
    json: bool,
) -> ExitCode {
    let valid_from = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // ── Manifest pass ─────────────────────────────────────────────────────────

    let mut manifest_edges = Vec::new();

    // Cargo.toml
    let cargo_path = format!("{root}/Cargo.toml");
    if let Ok(src) = std::fs::read_to_string(&cargo_path) {
        for dep in parse_cargo_deps(&src) {
            manifest_edges.push(manifest_edge(&project, &dep, &valid_from));
        }
    }

    // package.json files
    for pkg_rel in &["package.json", "gui/package.json"] {
        let pkg_path = format!("{root}/{pkg_rel}");
        if let Ok(src) = std::fs::read_to_string(&pkg_path) {
            for dep in parse_package_json(&src) {
                manifest_edges.push(manifest_edge(&project, &dep, &valid_from));
            }
        }
    }

    let manifest_count = manifest_edges.len();

    // ── ADR-mention pass ──────────────────────────────────────────────────────

    let mut adr_edges = Vec::new();

    let decisions_dir = format!("{root}/.rulebook/decisions");
    if let Ok(entries) = std::fs::read_dir(&decisions_dir) {
        let mut md_paths: Vec<std::path::PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        md_paths.sort(); // deterministic order

        for path in &md_paths {
            let body = match std::fs::read_to_string(path) {
                Ok(b) => b,
                Err(_) => continue,
            };

            // Derive decision key: leading numeric run before first '-',
            // formatted as ADR-{:03}. E.g. "018-phase18-foo.md" → "ADR-018".
            let decision_key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|stem| {
                    let numeric: String = stem
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    numeric.parse::<u32>().ok()
                })
                .map(|n| format!("ADR-{n:03}"));

            let key = match decision_key {
                Some(k) => k,
                None => continue, // no leading numeric run — skip
            };

            for (proj, ver) in scan_version_mentions(&body) {
                adr_edges.push(adr_mention_edge(&key, &proj, &ver, &valid_from));
            }
        }
    }

    let adr_count = adr_edges.len();

    // ── Dedup ─────────────────────────────────────────────────────────────────

    let mut all_edges = manifest_edges;
    all_edges.extend(adr_edges);
    let deduped = dedup_edges(all_edges);
    let deduped_total = deduped.len();

    // ── Write pass ────────────────────────────────────────────────────────────

    let mut written = 0usize;
    let mut errors = 0usize;

    if !dry_run {
        let (client, _nexus_url) = match connect_nexus(nexus.clone()) {
            Ok(c) => c,
            Err(code) => return code,
        };
        let runtime = match build_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        for edge in &deduped {
            let version_constraint = edge
                .props
                .get("version_constraint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let valid_from_prop = edge
                .props
                .get("valid_from")
                .and_then(|v| v.as_str())
                .unwrap_or(&valid_from)
                .to_string();

            let from_key_s = sanitize_literal(&edge.from_key);
            let to_key_s = sanitize_literal(&edge.to_key);
            let vc_s = sanitize_literal(&version_constraint);
            let vf_s = sanitize_literal(&valid_from_prop);

            let cypher = if edge.from_label == "Decision" {
                // MATCH-guarded: missing Decision → no-op (no orphan node).
                format!(
                    "MATCH (a:Decision {{ id: '{from_key_s}' }}) \
                     MERGE (b:Branch {{ id: '{to_key_s}' }}) \
                     MERGE (a)-[r:CROSS_PROJECT_REF]->(b) \
                     SET r.version_constraint = '{vc_s}', r.valid_from = '{vf_s}'"
                )
            } else {
                // Branch → Branch manifest edge.
                format!(
                    "MERGE (a:Branch {{ id: '{from_key_s}' }}) \
                     MERGE (b:Branch {{ id: '{to_key_s}' }}) \
                     MERGE (a)-[r:CROSS_PROJECT_REF]->(b) \
                     SET r.version_constraint = '{vc_s}', r.valid_from = '{vf_s}'"
                )
            };

            match runtime.block_on(async { client.sdk().execute_cypher(&cypher, None).await }) {
                Ok(_) => written += 1,
                Err(e) => {
                    eprintln!(
                        "WARN: backfill-cross-project: cypher error for {} -> {}: {e}",
                        edge.from_key, edge.to_key
                    );
                    errors += 1;
                }
            }
        }
    }

    // ── Report ────────────────────────────────────────────────────────────────

    let nexus_url = resolve_nexus_url(nexus);

    if json {
        let payload = serde_json::json!({
            "nexus_url": nexus_url,
            "project": project,
            "manifest_edges": manifest_count,
            "adr_edges": adr_count,
            "deduped_total": deduped_total,
            "written": written,
            "errors": errors,
            "dry_run": dry_run,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "cortex-ops backfill-cross-project @ {nexus_url}\n  project={project}  manifest_edges={manifest_count}  adr_edges={adr_count}  deduped_total={deduped_total}  written={written}  errors={errors}  dry_run={dry_run}"
        );
        if dry_run {
            for edge in &deduped {
                let vc = edge
                    .props
                    .get("version_constraint")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                println!("  {} -> {} ({})", edge.from_key, edge.to_key, vc);
            }
        }
    }

    if errors > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

// ── Helpers (mirrored from branch_cmd.rs) ────────────────────────────────────

fn connect_nexus(
    nexus: Option<String>,
) -> Result<(cortex_workers::graph::nexus_client::LiveNexusClient, String), ExitCode> {
    use cortex_workers::graph::config::GraphConfig;
    use cortex_workers::graph::nexus_client::LiveNexusClient;

    let nexus_url = nexus
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.nexus.nexus_url)
        })
        .unwrap_or_else(|| "http://127.0.0.1:17002".to_string());
    let cfg = GraphConfig {
        nexus_url: nexus_url.clone(),
        ..GraphConfig::default()
    };
    LiveNexusClient::new(cfg)
        .map(|c| (c, nexus_url.clone()))
        .map_err(|e| {
            eprintln!("ERROR: connect to Nexus at {nexus_url}: {e}");
            ExitCode::from(2)
        })
}

fn build_runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
    tokio::runtime::Runtime::new().map_err(|e| {
        eprintln!("ERROR: build tokio runtime: {e}");
        ExitCode::from(1)
    })
}

fn sanitize_literal(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\' | '\n' | '\r'))
        .collect()
}

/// Resolve the Nexus URL without constructing a live client (used in the
/// report line so dry-run runs can display it).
fn resolve_nexus_url(nexus: Option<String>) -> String {
    nexus
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.nexus.nexus_url)
        })
        .unwrap_or_else(|| "http://127.0.0.1:17002".to_string())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adr_key_derivation_from_filename() {
        // Simulate: stem = "018-phase18-something"
        // Expected: ADR-018
        let stem = "018-phase18-something";
        let numeric: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
        let n: u32 = numeric.parse().expect("should parse");
        assert_eq!(format!("ADR-{n:03}"), "ADR-018");
    }

    #[test]
    fn sanitize_literal_drops_quotes_backslashes_newlines() {
        assert_eq!(sanitize_literal("clean-id"), "clean-id");
        assert_eq!(sanitize_literal("a'b\"c\\d\ne\rf"), "abcdef");
    }
}
