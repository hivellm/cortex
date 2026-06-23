//! `cortex-ops migrate-classification` — backfill `class_level` /
//! `class_compartments` on events that predate phase21.
//!
//! In dry-run mode (default) the command walks the archive, counts
//! events missing classification, and reports anomalies. In live mode
//! (`--no-dry-run`) it records the imputation count (graph write is
//! wired in phase21 §3.2).

use std::path::PathBuf;
use std::process::ExitCode;

use cortex_storage::archive::walk_envelopes;
use cortex_workers::graph::classification_migration::{
    analyse_envelope, ClassificationMigrationReport, MAX_VALID_LEVEL,
};

/// `cortex-ops migrate-classification` entry point.
pub(super) fn migrate_classification(
    archive_root: Option<String>,
    project: Option<String>,
    default_level: u8,
    dry_run: bool,
    json_output: bool,
) -> ExitCode {
    if default_level > MAX_VALID_LEVEL {
        eprintln!(
            "error: --default-level {default_level} exceeds the maximum valid level ({MAX_VALID_LEVEL}).",
        );
        return ExitCode::FAILURE;
    }

    let archive = resolve_archive_root(archive_root);
    if !archive.exists() {
        eprintln!(
            "error: archive root `{}` does not exist.",
            archive.display()
        );
        return ExitCode::FAILURE;
    }

    let project_slug = project
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "_all".to_string());

    let mut report = ClassificationMigrationReport {
        project: project_slug.clone(),
        dry_run,
        ..Default::default()
    };

    let project_filter = project.clone().map(|p| p.to_ascii_lowercase());
    let envelopes = walk_envelopes(&archive, |env| {
        // Apply optional project filter.
        if let Some(ref slug) = project_filter {
            let env_repo = env
                .context
                .repo
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase();
            if &env_repo != slug {
                return false;
            }
        }
        true
    });

    match envelopes {
        Ok(envs) => {
            for env in &envs {
                analyse_envelope(env, default_level, dry_run, &mut report);
            }
        }
        Err(e) => {
            eprintln!("error walking archive: {e:#}");
            return ExitCode::FAILURE;
        }
    }

    if json_output {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error serialising report: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        let mode = if dry_run { "dry-run" } else { "live" };
        println!("migrate-classification ({mode}) — project: {project_slug}");
        println!("  examined : {}", report.total_events);
        println!("  missing  : {}", report.events_missing_class_level);
        println!("  imputed  : {}", report.events_imputed);
        println!("  anomalies: {}", report.anomalies.len());
        for a in &report.anomalies {
            println!("    {} — {:?}", a.event_id, a.reason);
        }
    }

    if report.anomalies.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

fn resolve_archive_root(override_path: Option<String>) -> PathBuf {
    if let Some(p) = override_path {
        return PathBuf::from(p);
    }
    if let Ok(cfg) = cortex_config::Config::load() {
        if let Some(r) = cfg.ingestion.archive_root {
            return PathBuf::from(r);
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".cortex").join("archive")
}
