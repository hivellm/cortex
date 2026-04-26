//! Clap CLI surface — every option in `docs/specs/09-bootstrap-cli.md` §CLI.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// `cortex-bootstrap` — walk existing Hive repos and republish their
/// content as synthetic events on `cortex.events.bootstrap` (spec 09).
#[derive(Debug, Clone, Parser)]
#[command(name = "cortex-bootstrap", version, about)]
pub struct CliArgs {
    /// Repo roots to walk. At least one required (unless `--resume`
    /// pulls them from the checkpoint, in which case CLI args may be
    /// empty).
    pub repo_roots: Vec<PathBuf>,

    /// Global config override (defaults to `./cortex-bootstrap.toml`).
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Include only the listed repos (by `id` after `cortex.toml`
    /// override or directory name). Comma-separated.
    #[arg(long, value_name = "NAME[,NAME]", value_delimiter = ',')]
    pub only: Vec<String>,

    /// Exclude the listed repos. Comma-separated.
    #[arg(long, value_name = "NAME[,NAME]", value_delimiter = ',')]
    pub skip: Vec<String>,

    /// Only re-index changes since this git ref (incremental mode).
    #[arg(long, value_name = "GIT-REF")]
    pub since: Option<String>,

    /// No writes; print plan.
    #[arg(long)]
    pub dry_run: bool,

    /// Implies --dry-run; print sizing block (files, chunks, bytes,
    /// est. cost).
    #[arg(long)]
    pub estimate: bool,

    /// Resume from the last checkpoint.
    #[arg(long)]
    pub resume: bool,

    /// Number of concurrent repo walkers.
    #[arg(long, default_value_t = 4, value_name = "N")]
    pub parallelism: usize,

    /// Override Synap connection URL.
    #[arg(long, value_name = "URL")]
    pub synap_endpoint: Option<String>,

    /// Destination Synap stream.
    #[arg(long, default_value = "cortex.events.bootstrap", value_name = "NAME")]
    pub stream: String,

    /// Checkpoint file path.
    #[arg(long, default_value = ".cortex-bootstrap.state.json", value_name = "FILE")]
    pub checkpoint: PathBuf,

    /// Structured logs format.
    #[arg(long, value_enum, default_value_t = LogFormat::Text)]
    pub log_format: LogFormat,

    /// Debug logging.
    #[arg(long)]
    pub verbose: bool,
}

/// Logging output mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    /// Stderr progress bar (TTY) plus human-readable lines.
    Text,
    /// One JSON object per log line (machine-readable).
    Json,
}

impl CliArgs {
    /// Convenience helper: `--estimate` always implies `--dry-run`.
    pub fn is_dry_run(&self) -> bool {
        self.dry_run || self.estimate
    }

    /// Whether the `repo_roots` argument is mandatory for this
    /// invocation. `--resume` is the only mode where it can be empty.
    pub fn requires_repo_roots(&self) -> bool {
        !self.resume
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_parses_minimum_invocation() {
        let args = CliArgs::parse_from(["cortex-bootstrap", "Vectorizer/"]);
        assert_eq!(args.repo_roots, vec![PathBuf::from("Vectorizer/")]);
        assert!(!args.is_dry_run());
        assert_eq!(args.parallelism, 4);
        assert_eq!(args.stream, "cortex.events.bootstrap");
        assert_eq!(args.log_format, LogFormat::Text);
    }

    #[test]
    fn cli_parses_estimate_implies_dry_run() {
        let args = CliArgs::parse_from(["cortex-bootstrap", "Vectorizer/", "--estimate"]);
        assert!(args.estimate);
        assert!(args.is_dry_run());
    }

    #[test]
    fn cli_parses_filters() {
        let args = CliArgs::parse_from([
            "cortex-bootstrap",
            "Vectorizer/",
            "Nexus/",
            "--only",
            "Vectorizer,Nexus",
            "--skip",
            "Lexum",
            "--since",
            "HEAD~200",
            "--parallelism",
            "8",
            "--log-format",
            "json",
        ]);
        assert_eq!(args.only, vec!["Vectorizer", "Nexus"]);
        assert_eq!(args.skip, vec!["Lexum"]);
        assert_eq!(args.since.as_deref(), Some("HEAD~200"));
        assert_eq!(args.parallelism, 8);
        assert_eq!(args.log_format, LogFormat::Json);
    }

    #[test]
    fn cli_resume_does_not_require_repo_roots() {
        let args = CliArgs::parse_from(["cortex-bootstrap", "--resume"]);
        assert!(args.repo_roots.is_empty());
        assert!(!args.requires_repo_roots());
    }

    #[test]
    fn clap_metadata_is_well_formed() {
        // Sanity: the command builds its help / version without panic.
        CliArgs::command().debug_assert();
    }
}
