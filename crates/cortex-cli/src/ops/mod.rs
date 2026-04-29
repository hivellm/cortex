//! Cortex operator CLI library (was crate `cortex-ops`).
//!
//! Plan + doctor subcommands. The doctor consistency checker
//! (phase4d) lives here so its modules are unit-testable without
//! booting the CLI.

#![warn(missing_docs)]

pub mod doctor;
pub mod log_rotate;
pub mod memory_consolidate;
pub mod probe;

pub use log_rotate::{rotate_if_needed, LogRotateOpts, LogRotateOutcome};

pub use doctor::{
    coverage_report, coverage_report_full, render_coverage_markdown, scan_hash_coverage,
    ArchiveProbe, ArchiveSummary, CoverageOptions, CoverageRow, DoctorReport,
    HashCoverageSummary, LiveNexusCoverageProbe, LiveVectorizerCoverageProbe, MeiliCoverageProbe,
    MemoryNexusCoverageProbe, MemoryVectorizerCoverageProbe, NexusCounts, NexusCoverageScan,
    PartitionKey, VectorizerCounts, VectorizerCoverageScan, HASH_COVERAGE_THRESHOLD,
    HASH_COVERAGE_WINDOW_HOURS,
};
pub use probe::{
    render_query_markdown, run_query_probes, JaccardObservation, MemoryQueryProbe, QueryProbe,
    QueryReport,
};
