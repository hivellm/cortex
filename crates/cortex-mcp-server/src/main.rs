//! Binary entry — clap dispatcher for the three sub-commands the spec
//! exposes:
//!
//! - `serve`: stdio JSON-RPC bridge.
//! - `validate <plugin-dir>`: lint the plugin asset tree.
//! - `print-tool-descriptors`: dump the three MCP descriptors as
//!   formatted JSON for debugging.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use cortex_mcp_server::tools::ToolContext;
use cortex_mcp_server::{validate, Server, ToolRegistry};

#[derive(Parser, Debug)]
#[command(
    name = "cortex-mcp-server",
    version,
    about = "Cortex MCP server (spec 18)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Override the Cortex API URL. Defaults to `$CORTEX_API_URL` or
    /// `http://127.0.0.1:17000` (the workspace's canonical
    /// `CORTEX_API_PORT`).
    #[arg(long, global = true)]
    api_url: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the JSON-RPC stdio bridge.
    Serve,
    /// Lint a `packages/cortex-claude-plugin/` directory.
    Validate {
        /// Path to the plugin directory.
        plugin_dir: PathBuf,
    },
    /// Print the three MCP tool descriptors as pretty JSON.
    PrintToolDescriptors,
}

fn resolve_api_url(cli_override: Option<String>) -> String {
    cli_override
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.dashboard.api_url)
        })
        .unwrap_or_else(|| cortex_mcp_server::DEFAULT_API_URL.to_string())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    init_tracing();

    match cli.cmd {
        Cmd::Serve => {
            let api_url = resolve_api_url(cli.api_url);
            tracing::info!(api_url = %api_url, "cortex-mcp-server starting on stdio");
            let server = Arc::new(Server::new(ToolContext::new(api_url)));
            cortex_mcp_server::transport_stdio::run(server).await
        }
        Cmd::Validate { plugin_dir } => {
            let report = validate::validate_plugin(&plugin_dir);
            let code = validate::print_report(&report, &plugin_dir);
            std::process::exit(code);
        }
        Cmd::PrintToolDescriptors => {
            let registry = ToolRegistry::default_set();
            let pretty = serde_json::to_string_pretty(&registry.descriptors())
                .expect("serialise descriptors");
            println!("{pretty}");
            Ok(())
        }
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_level(true)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .try_init();
}
