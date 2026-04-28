//! Cortex operator library — shared logic for the `cortex-ops`
//! binary (plan + doctor subcommands). The doctor consistency
//! checker (phase4d) lives here so its modules are unit-testable
//! without booting the CLI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod doctor;
pub mod probe;

pub use doctor::{
    coverage_report, coverage_report_full, render_coverage_markdown, ArchiveProbe, ArchiveSummary,
    CoverageOptions, CoverageRow, DoctorReport, LiveNexusCoverageProbe,
    LiveVectorizerCoverageProbe, MeiliCoverageProbe, MemoryNexusCoverageProbe,
    MemoryVectorizerCoverageProbe, NexusCounts, NexusCoverageScan, PartitionKey,
    VectorizerCounts, VectorizerCoverageScan,
};
pub use probe::{
    render_query_markdown, run_query_probes, JaccardObservation, MemoryQueryProbe, QueryProbe,
    QueryReport,
};
