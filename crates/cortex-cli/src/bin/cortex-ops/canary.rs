use std::process::ExitCode;

/// Phase8f — `cortex-ops canary`. Sends a synthetic frame through
/// the daemon's IPC and polls the archive for the marker. Exit
/// codes match `CanaryOutcome::exit_code()`.
pub(super) fn canary(
    hook: String,
    ipc: Option<String>,
    api_url: Option<String>,
    deadline_secs: u64,
    json: bool,
) -> ExitCode {
    use cortex_api::canary::{run_canary_once, CanaryConfig};
    let mut cfg = CanaryConfig {
        deadline_secs,
        ipc_path: ipc,
        ..CanaryConfig::default()
    };
    if let Some(url) = api_url {
        cfg.api_url = url;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = runtime.block_on(run_canary_once(&cfg, &hook));
    if json {
        match serde_json::to_string_pretty(&outcome) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize outcome: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops canary --hook={hook}");
        println!("{}", outcome.describe());
    }
    match outcome.exit_code() {
        0 => ExitCode::SUCCESS,
        2 => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}
