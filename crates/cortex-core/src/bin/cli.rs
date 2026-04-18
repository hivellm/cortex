//! `cortex-core` CLI. Runs envelope + per-kind schema validation against a
//! JSON file (or stdin).

use clap::{Parser, Subcommand};
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "cortex-core", version, about = "Cortex core CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a JSON file (or stdin) against the envelope + per-kind schema.
    Validate {
        /// Path to the event JSON. Use `-` to read from stdin.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Print the canonical-JSON content hash of a payload (or stdin).
    Hash {
        /// Path to a JSON payload. Use `-` to read from stdin.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Emit a fresh ULID.
    NewId,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Validate { file } => {
            let value = load_json(&file)?;
            match cortex_core::validate_event(&value) {
                Ok(()) => {
                    println!("ok: event is valid");
                    Ok(())
                }
                Err(errors) => {
                    eprintln!("event failed validation:");
                    for err in errors {
                        eprintln!("  - {err}");
                    }
                    std::process::exit(2);
                }
            }
        }
        Command::Hash { file } => {
            let value = load_json(&file)?;
            let hash = cortex_core::content_hash(&value)?;
            println!("{hash}");
            Ok(())
        }
        Command::NewId => {
            println!("{}", cortex_core::event_id());
            Ok(())
        }
    }
}

fn load_json(path: &std::path::Path) -> anyhow::Result<Value> {
    let raw = if path.to_string_lossy() == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(path)?
    };
    Ok(serde_json::from_str(&raw)?)
}
