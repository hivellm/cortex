//! Binary entry — clap dispatcher for the three sub-commands the spec
//! exposes:
//!
//! - `serve`: stdio JSON-RPC bridge.
//! - `validate <plugin-dir>`: lint the plugin asset tree.
//! - `print-tool-descriptors`: dump the configured profile's MCP descriptors as
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
    /// `CORTEX_API_PORT`). An empty value is ignored, so a plugin can pass
    /// an unset user option through unchanged.
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
    /// Print the configured profile's MCP tool descriptors as pretty JSON.
    PrintToolDescriptors,
}

fn tool_context(cli_override: Option<String>, config: cortex_config::Config) -> ToolContext {
    let api_url = cli_override
        .filter(|url| !url.is_empty())
        .or(config.dashboard.api_url)
        .unwrap_or_else(|| cortex_mcp_server::DEFAULT_API_URL.to_string());
    let context = ToolContext::new(api_url);
    match config.dashboard.api_token.filter(|token| !token.is_empty()) {
        Some(token) => context.with_api_key(token),
        None => context,
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    init_tracing();

    match cli.cmd {
        Cmd::Serve => {
            let config = cortex_config::Config::load_client().map_err(std::io::Error::other)?;
            let profile = config.runtime.profile;
            let registry = ToolRegistry::for_profile(profile);
            let context = tool_context(cli.api_url, config);
            tracing::info!(api_url = %context.api_url, runtime_profile = ?profile, tools = registry.len(), "cortex-mcp-server starting on stdio");
            let server = Arc::new(Server::with_registry(context, registry));
            cortex_mcp_server::transport_stdio::run(server).await
        }
        Cmd::Validate { plugin_dir } => {
            let report = validate::validate_plugin(&plugin_dir);
            let code = validate::print_report(&report, &plugin_dir);
            std::process::exit(code);
        }
        Cmd::PrintToolDescriptors => {
            let config = cortex_config::Config::load().map_err(std::io::Error::other)?;
            let registry = ToolRegistry::for_profile(config.runtime.profile);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_context_forwards_configured_token_and_preserves_url_override() {
        let mut config = cortex_config::Config::default();
        config.dashboard.api_url = Some("http://127.0.0.1:17001".into());
        config.dashboard.api_token = Some("synthetic-token".into());
        let context = tool_context(Some("http://127.0.0.1:17002".into()), config);
        assert_eq!(context.api_url, "http://127.0.0.1:17002");
        assert_eq!(context.api_key.as_deref(), Some("synthetic-token"));
    }

    #[test]
    fn empty_cli_url_defers_to_configuration() {
        let mut config = cortex_config::Config::default();
        config.dashboard.api_url = Some("http://127.0.0.1:17003".into());
        assert_eq!(
            tool_context(Some(String::new()), config).api_url,
            "http://127.0.0.1:17003"
        );
    }

    #[test]
    fn absent_or_empty_configured_token_keeps_anonymous_transport() {
        for token in [None, Some(String::new())] {
            let mut config = cortex_config::Config::default();
            config.dashboard.api_token = token;
            let context = tool_context(None, config);
            assert_eq!(context.api_url, cortex_mcp_server::DEFAULT_API_URL);
            assert!(context.api_key.is_none());
        }
    }
}
