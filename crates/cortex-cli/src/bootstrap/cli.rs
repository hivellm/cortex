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

    /// Phase11e §5 — limit the replay to envelope kinds matching
    /// these family tokens. Comma-separated; recognised tokens are
    /// the spec-08 family suffixes (`decisions`, `turns`, `memory`,
    /// `analyses`, `laws`, `knowledge`, `learnings`, `code`,
    /// `docs`, `artifacts`). Unset / empty replays every kind
    /// (the legacy default).
    #[arg(long, value_name = "KIND[,KIND]", value_delimiter = ',')]
    pub kinds: Vec<String>,

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

    /// Workspace TOML enumerating multiple repos to bootstrap in
    /// one invocation. When set, the file's `[[repo]]` entries
    /// drive the run instead of (or alongside) the positional
    /// `repo_roots` args.
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,

    /// Re-run repos whose checkpoint already reports `status = done`.
    /// Without this flag, the orchestrator bypasses any repo whose
    /// checkpoint matches the current `HEAD` ref.
    #[arg(long)]
    pub force: bool,

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
    #[arg(
        long,
        default_value = ".cortex-bootstrap.state.json",
        value_name = "FILE"
    )]
    pub checkpoint: PathBuf,

    /// Structured logs format.
    #[arg(long, value_enum, default_value_t = LogFormat::Text)]
    pub log_format: LogFormat,

    /// Debug logging.
    #[arg(long)]
    pub verbose: bool,

    /// Phase11k §5.1 — run the static graph-extraction pass instead
    /// of the regular Synap-publishing bootstrap. Walks every
    /// selected repo, runs the per-language code analyzers and the
    /// markdown analyzer, builds [`GraphPatch`] entries via the
    /// resolver, and writes one canonical envelope per analyzed
    /// file to the zstd-NDJSON archive sink that
    /// `cortex-api::archive_loader` re-reads at boot. Synap, the
    /// runner, and the checkpoint logic are bypassed in this mode.
    ///
    /// [`GraphPatch`]: cortex_workers::graph::patch::GraphPatch
    #[arg(long)]
    pub graph_static: bool,

    /// Archive root the graph-static mode writes into. Required
    /// when [`Self::graph_static`] is set; ignored otherwise.
    #[arg(long, value_name = "PATH")]
    pub graph_archive_root: Option<PathBuf>,

    /// Phase11j §3.7 — run only the Meilisearch settings push, no
    /// repo walks, no Synap publish. PATCHes the baked-in
    /// `settings.v1.json` against every canonical global index in
    /// `cortex-storage::names::ALL_INDEXES` plus, when
    /// `--workspace` / positional repo roots are supplied, every
    /// per-repo `cortex-{slug}-{family}` uid the embedder + indexer
    /// would target. Settings PATCH is non-destructive: Meili
    /// applies new attributes additively without dropping documents
    /// or task history. The `version` field baked into
    /// `settings.v1.json` is stripped before forwarding so Meili
    /// accepts it.
    #[arg(long)]
    pub apply_settings_only: bool,

    /// Phase11j §3.7 — Meilisearch base URL the
    /// `--apply-settings-only` mode targets. Falls back to the
    /// `CORTEX_MEILI_URL` env var, then `http://127.0.0.1:17004`
    /// (the default `cortex-up` port). Ignored when
    /// `--apply-settings-only` is unset.
    #[arg(long, value_name = "URL", env = "CORTEX_MEILI_URL")]
    pub meili_url: Option<String>,

    /// Phase11j §3.7 — Meilisearch master key for the
    /// `--apply-settings-only` mode. Falls back to
    /// `CORTEX_MEILI_KEY`; passing `--apply-settings-only` against
    /// a server with auth enabled requires this to be set.
    #[arg(long, value_name = "KEY", env = "CORTEX_MEILI_KEY")]
    pub meili_api_key: Option<String>,
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
    /// invocation. `--resume` and `--workspace` both supply repo
    /// targets without positional args.
    /// `--apply-settings-only` runs against the global canonical
    /// index list when no repo roots are supplied, so it is also
    /// allowed without positional args.
    pub fn requires_repo_roots(&self) -> bool {
        !self.resume && self.workspace.is_none() && !self.apply_settings_only
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
    fn cli_parses_graph_static_flag() {
        let args = CliArgs::parse_from([
            "cortex-bootstrap",
            "Vectorizer/",
            "--graph-static",
            "--graph-archive-root",
            "/var/lib/cortex/archive",
        ]);
        assert!(args.graph_static);
        assert_eq!(
            args.graph_archive_root.as_deref(),
            Some(std::path::Path::new("/var/lib/cortex/archive"))
        );
    }

    #[test]
    fn cli_graph_static_defaults_off() {
        let args = CliArgs::parse_from(["cortex-bootstrap", "Vectorizer/"]);
        assert!(!args.graph_static);
        assert!(args.graph_archive_root.is_none());
    }

    #[test]
    fn clap_metadata_is_well_formed() {
        // Sanity: the command builds its help / version without panic.
        CliArgs::command().debug_assert();
    }

    #[test]
    fn cli_apply_settings_only_does_not_require_repo_roots() {
        // Phase11j §3.7 — `--apply-settings-only` walks the global
        // canonical index list, so it must parse without positional
        // repo roots and must opt out of `requires_repo_roots`.
        let args = CliArgs::parse_from(["cortex-bootstrap", "--apply-settings-only"]);
        assert!(args.apply_settings_only);
        assert!(args.repo_roots.is_empty());
        assert!(!args.requires_repo_roots());
    }

    #[test]
    fn cli_apply_settings_only_accepts_meili_overrides() {
        // The flag honours `--meili-url` + `--meili-api-key` for the
        // staging-deploy case where the operator points the push at a
        // non-default Meili. Both fields fall back to env vars in
        // production; the CLI still has to parse them when given.
        let args = CliArgs::parse_from([
            "cortex-bootstrap",
            "--apply-settings-only",
            "--meili-url",
            "http://10.0.0.4:7700",
            "--meili-api-key",
            "secret-token",
        ]);
        assert!(args.apply_settings_only);
        assert_eq!(args.meili_url.as_deref(), Some("http://10.0.0.4:7700"));
        assert_eq!(args.meili_api_key.as_deref(), Some("secret-token"));
    }

    #[test]
    fn cli_apply_settings_only_defaults_off() {
        let args = CliArgs::parse_from(["cortex-bootstrap", "Vectorizer/"]);
        assert!(!args.apply_settings_only);
    }
}
