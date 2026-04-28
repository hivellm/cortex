//! Cortex bootstrap CLI library — walks existing Hive repos and
//! republishes their content as synthetic events on
//! `cortex.events.bootstrap`. Mirrors the operational shape of the
//! Phase-1 worker crates (`cortex-embedder`, `cortex-graph`,
//! `cortex-fulltext`) so operations look identical across all four
//! Phase-1 binaries.
//!
//! See `docs/specs/09-bootstrap-cli.md` for the full contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod checkpoint;
pub mod cli;
pub mod config;
pub mod emitter;
pub mod estimate;
pub mod git;
pub mod metrics;
pub mod publisher;
pub mod runner;
pub mod walker;
pub mod workspace;

pub use checkpoint::{load_or_default as load_checkpoint, write_atomic, Checkpoint, RepoProgress};
pub use cli::{CliArgs, LogFormat};
pub use config::{
    load_for_repo, load_or_default as load_config, ChunkingConfig, CortexSection, CortexToml,
    ExcludeConfig, ExtraPattern, GitConfig, MemoriesConfig, PromoteConfig, RedactionConfig,
};
pub use emitter::{
    emit_artifact_code, emit_artifact_doc, emit_decision_imported, emit_for_file,
    emit_for_file_multi, emit_law_imported, emit_memory_imported, emit_spec_laws_imported,
    emit_turn_historical, BootstrapEvent, BOOTSTRAP_STREAM,
};
pub use estimate::{estimate_repo, format_estimate, Estimate};
pub use git::{current_head_sha, parse_log, walk_commits, CommitRecord, GitWalkError};
pub use metrics::Metrics;
pub use publisher::{LiveSynapPublisher, MemoryPublisher, Publisher, SynapHandle};
pub use runner::{count_classes, run_repo, run_repos_parallel, RepoRunReport, RunnerConfig};
pub use walker::{classify_path, matches_any, walk_repo, FileClass, WalkEntry, MAX_FILE_BYTES};
pub use workspace::{
    load_workspace, preflight as preflight_workspace, WorkspaceConfig, WorkspaceError,
    WorkspaceRepo,
};
