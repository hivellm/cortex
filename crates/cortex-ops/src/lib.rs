//! Cortex operator library — shared logic for the `cortex-ops`
//! binary (plan + doctor subcommands). The doctor consistency
//! checker (phase4d) lives here so its modules are unit-testable
//! without booting the CLI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod doctor;

pub use doctor::{
    coverage_report, render_coverage_markdown, ArchiveProbe, ArchiveSummary, CoverageRow,
    DoctorReport, MeiliCoverageProbe, PartitionKey,
};
