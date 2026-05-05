use std::process::ExitCode;
use super::helpers::home_dir;

/// Phase9h — `cortex-ops memory-consolidate`. Embeds, clusters, and
/// merges Claude Code's auto-memory directory. Today's binding wires
/// the deterministic in-process embedder + rule-based merger so the
/// CLI runs offline; the production Sonnet driver lands when the
/// classifier surface exposes a streaming agent client.
pub(super) fn memory_consolidate(
    project: Option<String>,
    threshold: f32,
    drift_floor: f32,
    max_clusters: Option<usize>,
    apply: bool,
    memory_dir: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_cli::ops::memory_consolidate::{
        memory_dir_for, resolve_project_slug, run, ClusterOutcome, HashingEmbedder, Plan,
        RuleMerger,
    };

    // phase11v §6 — bookkeeping anchor.
    let started_at = chrono::Utc::now();

    let dir: std::path::PathBuf = match memory_dir {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let slug = match project {
                Some(s) => s,
                None => {
                    let cwd = match std::env::current_dir() {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("memory-consolidate: cwd: {e}");
                            return ExitCode::FAILURE;
                        }
                    };
                    resolve_project_slug(&cwd)
                }
            };
            let home = match home_dir() {
                Some(h) => h,
                None => {
                    eprintln!("memory-consolidate: HOME / USERPROFILE unset");
                    return ExitCode::FAILURE;
                }
            };
            memory_dir_for(&home, &slug)
        }
    };

    let plan = Plan {
        now: chrono::Utc::now(),
        threshold,
        drift_floor,
        max_clusters,
        apply,
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let embedder = HashingEmbedder::default();
    let merger = RuleMerger;
    let report = match runtime.block_on(run(&dir, &plan, &embedder, &merger)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("memory-consolidate: {e}");
            return ExitCode::FAILURE;
        }
    };

    if json {
        let payload = serde_json::json!({
            "memory_dir": dir.display().to_string(),
            "files_in": report.files_in,
            "files_out": report.files_out,
            "applied": report.applied,
            "archive_dir": report.archive_dir.as_ref().map(|p| p.display().to_string()),
            "warnings": report
                .warnings
                .iter()
                .map(|(p, msg)| {
                    serde_json::json!({
                        "path": p.display().to_string(),
                        "reason": msg,
                    })
                })
                .collect::<Vec<_>>(),
            "clusters": report
                .clusters
                .iter()
                .map(|c| {
                    let outcome = match &c.outcome {
                        ClusterOutcome::Singleton => serde_json::json!({"kind": "singleton"}),
                        ClusterOutcome::Merged {
                            consolidated_filename,
                            frontmatter,
                        } => serde_json::json!({
                            "kind": "merged",
                            "consolidated_filename": consolidated_filename,
                            "frontmatter": {
                                "name": frontmatter.name,
                                "description": frontmatter.description,
                                "type": frontmatter.kind.as_str(),
                            },
                        }),
                        ClusterOutcome::SkippedDrift { min_cosine } => {
                            serde_json::json!({
                                "kind": "skipped_drift",
                                "min_cosine": min_cosine,
                            })
                        }
                        ClusterOutcome::SkippedAgentError { reason } => {
                            serde_json::json!({
                                "kind": "skipped_agent_error",
                                "reason": reason,
                            })
                        }
                    };
                    serde_json::json!({
                        "type": c.kind.as_str(),
                        "members": c.members,
                        "outcome": outcome,
                    })
                })
                .collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops memory-consolidate");
        println!("memory_dir:  {}", dir.display());
        println!("files_in:    {}", report.files_in);
        println!("files_out:   {}", report.files_out);
        println!("applied:     {}", report.applied);
        if let Some(p) = &report.archive_dir {
            println!("archive_dir: {}", p.display());
        }
        if !report.warnings.is_empty() {
            println!("warnings:");
            for (p, msg) in &report.warnings {
                println!("  {} — {msg}", p.display());
            }
        }
        if report.clusters.is_empty() {
            println!("clusters:   (none)");
        } else {
            println!("clusters:");
            for c in &report.clusters {
                let detail = match &c.outcome {
                    ClusterOutcome::Singleton => "singleton".to_string(),
                    ClusterOutcome::Merged {
                        consolidated_filename,
                        ..
                    } => format!("merged → {consolidated_filename}"),
                    ClusterOutcome::SkippedDrift { min_cosine } => {
                        format!("skipped (drift, min_cosine={min_cosine:.2})")
                    }
                    ClusterOutcome::SkippedAgentError { reason } => {
                        format!("skipped (agent: {reason})")
                    }
                };
                println!(
                    "  [{}] {} files: {}",
                    c.kind.as_str(),
                    c.members.len(),
                    detail
                );
                for m in &c.members {
                    println!("    - {m}");
                }
            }
        }
    }
    let mut extras = serde_json::Map::new();
    extras.insert("files_in".into(), report.files_in.into());
    extras.insert("files_out".into(), report.files_out.into());
    extras.insert("applied".into(), report.applied.into());
    super::record_sweep_run(
        "memory_consolidate",
        started_at,
        "success",
        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
            records_demoted: report.applied as u64,
            extras,
            ..Default::default()
        },
    );
    ExitCode::SUCCESS
}
