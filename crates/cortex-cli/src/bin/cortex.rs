//! `cortex` — install and update the daily Cortex runtime.
//!
//! Subcommands:
//! - `setup` — write `<Cortex home>/config.toml` and `api-token`.
//! - `install` — copy the daily runtime binaries into `<Cortex home>/bin`.
//! - `doctor` — report installed binaries that are missing, shadowed or stale.
//! - `update` — pull, build, install and restart from the source checkout.
//!
//! Every command is a dry run unless `--apply` is passed.

#![forbid(unsafe_code)]

// Submodules live under `src/bin/cortex/`; a bin crate root resolves
// `mod <name>;` next to itself, so each module names its path explicitly.
#[path = "cortex/install_bins.rs"]
mod install_bins;
#[path = "cortex/setup.rs"]
mod setup;
#[path = "cortex/update.rs"]
mod update;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "cortex",
    version,
    about = "Install, check and update the daily Cortex runtime"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write `<Cortex home>/config.toml` and `api-token` (dry run unless
    /// `--apply`), optionally migrating an existing runtime TOML.
    Setup {
        /// Write the planned files (default: print the plan only).
        #[arg(long)]
        apply: bool,
        /// Existing runtime TOML to migrate, e.g.
        /// `.cortex-state/daily-runtime/runtime.toml`.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Target Cortex home (default: `CORTEX_HOME`, else `~/.cortex`).
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Copy the daily runtime binaries from a release build into a stable user
    /// directory (dry run unless `--apply`).
    Install {
        /// Install the binaries (default: print the plan only).
        #[arg(long)]
        apply: bool,
        /// Release build directory holding the binaries, e.g. `target/release`.
        #[arg(long)]
        from: PathBuf,
        /// Destination directory (default: `<Cortex home>/bin`).
        #[arg(long)]
        to: Option<PathBuf>,
    },
    /// Report daily runtime binaries that are missing from PATH, shadowed by
    /// another PATH entry, or stale. Exit 2 on any finding.
    Doctor {
        /// Installed binary directory (default: `<Cortex home>/bin`).
        #[arg(long)]
        bin_dir: Option<PathBuf>,
        /// Optional build directory the installed binaries must match.
        #[arg(long)]
        build: Option<PathBuf>,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Update the installed Cortex from its source checkout: git pull, release
    /// build, install into the bin dir, stop the old runtime, refresh the
    /// plugins (dry run unless `--apply`).
    Update {
        /// Run the update (default: print the plan only).
        #[arg(long)]
        apply: bool,
        /// Cortex checkout to build (default: the workspace holding the build
        /// directory recorded in `<bin dir>/install-manifest.json`).
        #[arg(long)]
        source: Option<PathBuf>,
        /// Build the checkout as it is, without `git pull --ff-only`.
        #[arg(long)]
        no_pull: bool,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Setup { apply, from, home } => setup::setup(apply, from, home),
        Command::Install { apply, from, to } => install_bins::install_binaries(apply, from, to),
        Command::Doctor {
            bin_dir,
            build,
            json,
        } => install_bins::doctor_install(bin_dir, build, json),
        Command::Update {
            apply,
            source,
            no_pull,
        } => update::update(source, no_pull, apply),
    }
}
