//! Cortex operator library — shared logic for the `cortex-ops`
//! binary (plan + doctor subcommands). The doctor consistency
//! checker (phase4d) lives here so its modules are unit-testable
//! without booting the CLI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod doctor;
pub mod probe;

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
