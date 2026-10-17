//! Native Claude Code hook transport.
//!
//! SessionStart, UserPromptSubmit and PreToolUse always await one JSON reply.
//! Publish-only events may use --fire-forget. Input, frame and reply sizes are
//! bounded; one deadline covers configuration, stdin, connection, write, read
//! and notice-state admission. Blocking filesystem work never delays shutdown.
//! Every failure exits zero. Only SessionStart/UserPromptSubmit can show the
//! fixed operator notice, once per actual session, never as additionalContext.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use cortex_adapter_claude_code::lifecycle::{ensure_started, should_autostart, StartDecision};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const DEFAULT_TIMEOUT_MS: u64 = 1500;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_REPLY_BYTES: usize = 64 * 1024;
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const NOTICE_BUDGET: Duration = Duration::from_millis(50);
const MAX_NOTICE_MARKERS: usize = 4096;
#[cfg(windows)]
const PIPE_DEFAULT: &str = r"\\.\pipe\cortex-adapter-claude";
const UNAVAILABLE_NOTICE: &str =
    "Cortex: local memory is unavailable. This prompt continues without saved Cortex memory.";
const START_FAILED_PREFIX: &str = "Cortex: could not start local memory (";
const RETRY_INTERVAL: Duration = Duration::from_millis(50);
const EARLY_EXIT_PROBE: Duration = Duration::from_millis(150);

#[derive(Debug)]
struct Args {
    event: String,
    fire_forget: bool,
    #[cfg_attr(not(windows), allow(dead_code))]
    pipe: Option<String>,
    #[cfg_attr(windows, allow(dead_code))]
    sock: Option<String>,
    timeout_ms: u64,
}

impl Args {
    fn parse(argv: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut argv = argv.into_iter();
        let mut event = None;
        let mut fire_forget = false;
        let mut pipe = None;
        let mut sock = None;
        let mut timeout_ms = DEFAULT_TIMEOUT_MS;
        while let Some(arg) = argv.next() {
            match arg.as_str() {
                "--fire-forget" => fire_forget = true,
                "--pipe" => pipe = Some(argv.next().ok_or("missing --pipe value")?),
                "--sock" => sock = Some(argv.next().ok_or("missing --sock value")?),
                "--timeout-ms" => {
                    timeout_ms = argv
                        .next()
                        .ok_or("missing --timeout-ms value")?
                        .parse()
                        .map_err(|_| "invalid --timeout-ms")?;
                    if !(1..=DEFAULT_TIMEOUT_MS).contains(&timeout_ms) {
                        return Err("--timeout-ms must be between 1 and 1500".into());
                    }
                }
                other if other.starts_with('-') => return Err("unknown flag".into()),
                other if event.is_none() => event = Some(other.to_string()),
                _ => return Err("unexpected argument".into()),
            }
        }
        let event = event.ok_or("missing event")?;
        if !matches!(
            event.as_str(),
            "SessionStart"
                | "UserPromptSubmit"
                | "PreToolUse"
                | "PostToolUse"
                | "Stop"
                | "SubagentStop"
                | "Notification"
        ) {
            return Err("unknown event".into());
        }
        // Context and enforcement cannot be silently discarded by a stale
        // packaged shim that still supplies --fire-forget.
        fire_forget &= !matches!(
            event.as_str(),
            "SessionStart" | "UserPromptSubmit" | "PreToolUse"
        );
        Ok(Self {
            event,
            fire_forget,
            pipe,
            sock,
            timeout_ms,
        })
    }
}

fn print_help() {
    println!(
        "cortex-hook {} — Claude Code hook shim\n\n\
         USAGE:\n  cortex-hook <event-name> [--fire-forget] [--pipe NAME] [--sock PATH] [--timeout-ms MS]\n\n\
         SessionStart, UserPromptSubmit and PreToolUse always wait for context or a decision.\n\
         --fire-forget applies only to publish-only events.\n\
         --timeout-ms is a total I/O budget, between 1 and {} ms.\n\n\
         CORTEX_ADAPTER_DISABLE=1 disables all hook I/O.\n\
         CORTEX_ADAPTER_PIPE / CORTEX_ADAPTER_SOCK override the local endpoint.\n\
         Stdin session_id and cwd take precedence over environment and process defaults.\n\
         CORTEX_HOOK_STATE_DIR overrides the private outage-marker directory.\n",
        env!("CARGO_PKG_VERSION"), DEFAULT_TIMEOUT_MS
    );
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return;
    }
    if argv.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("cortex-hook {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut args = match Args::parse(argv) {
        Ok(args) => args,
        Err(_) => {
            println!("{{}}");
            return;
        }
    };
    let started = Instant::now();
    let budget = Duration::from_millis(args.timeout_ms);
    let deadline = started + budget;
    let reserve = NOTICE_BUDGET.min(budget / 5);
    let io_deadline = deadline - reserve;
    // Preserve the documented kill switch even if TOML is corrupt or its
    // filesystem is unavailable. Other configuration errors also fail open.
    if cortex_config::adapter_disabled_by_environment() {
        println!("{{}}");
        return;
    }
    let cfg = match blocking_until(io_deadline, load_hook_config) {
        Some(Ok(cfg)) => cfg,
        _ => {
            println!("{{}}");
            return;
        }
    };
    if cfg.adapter.adapter_disable {
        println!("{{}}");
        return;
    }
    let autostart_enabled = cfg.adapter.hook_autostart.unwrap_or(true);
    let plugin_origin = cfg.adapter.hook_origin.as_deref() == Some("plugin");
    args.pipe = args.pipe.or(cfg.adapter.adapter_pipe);
    args.sock = args.sock.or(cfg.adapter.adapter_sock);
    if plugin_origin {
        let event = args.event.clone();
        let delivered_by_settings = blocking_until(io_deadline, move || {
            cortex_adapter_claude_code::legacy::settings_path(
                std::env::var_os("CLAUDE_CONFIG_DIR")
                    .map(PathBuf::from)
                    .as_deref(),
                home_dir().as_deref(),
            )
            .is_some_and(|path| {
                cortex_adapter_claude_code::legacy::read_legacy_events(&path).contains(&event)
            })
        });
        // Unknown (budget spent) also skips: legacy hooks may deliver it.
        if delivered_by_settings != Some(false) {
            let _ = blocking_until(io_deadline, || read_bounded(std::io::stdin().lock()));
            println!("{{}}");
            return;
        }
    }
    let event = args.event.clone();
    let frame = match blocking_until(io_deadline, move || {
        let input = read_bounded(std::io::stdin().lock()).ok()?;
        let cwd = std::env::current_dir().ok()?.display().to_string();
        build_frame(
            &event,
            &input,
            &std::env::var("CLAUDE_SESSION_ID").unwrap_or_default(),
            &cwd,
        )
    })
    .flatten()
    {
        Some(frame) => frame,
        None => {
            println!("{{}}");
            return;
        }
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            println!("{{}}");
            return;
        }
    };
    let attempt = |rt: &tokio::runtime::Runtime| {
        let remaining = io_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::ErrorKind::TimedOut);
        }
        rt.block_on(async {
            match tokio::time::timeout(
                remaining,
                transport_round_trip(&args, frame.line.as_bytes()),
            )
            .await
            {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(error.kind()),
                Err(_) => Err(io::ErrorKind::TimedOut),
            }
        })
    };
    let response = match attempt(&rt) {
        Ok(value) => value,
        Err(kind)
            if should_autostart(
                &args.event,
                explicit_endpoint(&args),
                autostart_enabled,
                kind,
            ) =>
        {
            autostart_then_retry(&rt, &attempt, cfg.dashboard.api_bind.clone(), io_deadline)
        }
        Err(_) if notice_event(&args.event) => json!({"systemMessage": UNAVAILABLE_NOTICE}),
        Err(_) => json!({}),
    };
    let response = filter_notice(
        &args.event,
        &frame.session_id,
        response,
        state_dir().as_deref(),
        deadline.min(Instant::now() + NOTICE_BUDGET),
    );
    println!("{response}");
}

struct Frame {
    line: String,
    session_id: String,
}

fn read_bounded(input: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input
        .take((MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(invalid_data("hook input exceeds limit"));
    }
    Ok(bytes)
}

/// Detached standard threads cannot hold a Tokio runtime open at shutdown.
/// A slow filesystem or unclosed stdin consumes only the remaining budget.
fn blocking_until<T: Send + 'static>(
    deadline: Instant,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    deadline.checked_duration_since(Instant::now())?;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let _worker = std::thread::Builder::new()
        .spawn(move || {
            let _ = send.send(work());
        })
        .ok()?;
    receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
}

fn load_hook_config() -> io::Result<cortex_config::Config> {
    let path = cortex_config::default_toml_path();
    match std::fs::metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES => {
            return Err(invalid_data(
                "hook configuration is not a bounded regular file",
            ))
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    cortex_config::Config::load_from(&path, |name| {
        let value = std::env::var(name).ok()?;
        // Legacy hook scripts use 0/1, while typed configuration uses bools.
        let boolean = matches!(name, "CORTEX_ADAPTER_DISABLE" | "CORTEX_HOOK_AUTOSTART");
        Some(match value.as_str() {
            "0" if boolean => "false".into(),
            "1" if boolean => "true".into(),
            _ => value,
        })
    })
    .map_err(|_| invalid_data("invalid hook configuration"))
}

fn valid_session(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn build_frame(event: &str, input: &[u8], env_session: &str, process_cwd: &str) -> Option<Frame> {
    let payload: Value = if input.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        serde_json::from_slice(input).ok()?
    };
    if !payload.is_object() {
        return None;
    }
    // A present invalid identity must not silently select an unrelated env
    // session or reach the daemon through its nested payload fallback.
    let session_id = match payload.get("session_id") {
        Some(value) => value.as_str().filter(|value| valid_session(value))?,
        None => {
            if valid_session(env_session) {
                env_session
            } else {
                ""
            }
        }
    }
    .to_string();
    let cwd = match payload.get("cwd") {
        Some(value) => value.as_str()?,
        None => process_cwd,
    };
    if cwd.len() > 32 * 1024 || cwd.chars().any(char::is_control) {
        return None;
    }
    let mut line = json!({"hook": event, "session_id": session_id, "cwd": cwd, "payload": payload})
        .to_string();
    line.push('\n');
    (line.len() <= MAX_INPUT_BYTES).then_some(Frame { line, session_id })
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

async fn read_reply(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Value> {
    let mut bytes = Vec::with_capacity(8192);
    let mut chunk = [0u8; 4096];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        let end = chunk[..count].iter().position(|byte| *byte == b'\n');
        let slice = &chunk[..end.unwrap_or(count)];
        if bytes.len().saturating_add(slice.len()) > MAX_REPLY_BYTES {
            return Err(invalid_data("hook reply exceeds limit"));
        }
        bytes.extend_from_slice(slice);
        if end.is_some() {
            break;
        }
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| invalid_data("invalid hook reply"))?;
    if !value.is_object() {
        return Err(invalid_data("hook reply must be an object"));
    }
    Ok(value)
}

async fn exchange(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    frame: &[u8],
    fire_forget: bool,
) -> io::Result<Value> {
    stream.write_all(frame).await?;
    stream.flush().await?;
    if fire_forget {
        Ok(json!({}))
    } else {
        read_reply(stream).await
    }
}

#[cfg(windows)]
async fn transport_round_trip(args: &Args, frame: &[u8]) -> io::Result<Value> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let pipe = args
        .pipe
        .as_deref()
        .filter(|pipe| !pipe.is_empty())
        .unwrap_or(PIPE_DEFAULT);
    let mut client = loop {
        match ClientOptions::new().open(pipe) {
            Ok(client) => break client,
            Err(error) if error.raw_os_error() == Some(231) => {
                // Busy is transient; outer deadline includes every retry.
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error),
        }
    };
    exchange(&mut client, frame, args.fire_forget).await
}

#[cfg(unix)]
async fn transport_round_trip(args: &Args, frame: &[u8]) -> io::Result<Value> {
    let path = args.sock.as_ref().map(PathBuf::from).unwrap_or_else(|| {
        home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".cortex/adapter-claude.sock")
    });
    let mut client = tokio::net::UnixStream::connect(path).await?;
    exchange(&mut client, frame, args.fire_forget).await
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let names = ["USERPROFILE", "HOME"];
    #[cfg(not(windows))]
    let names = ["HOME", "USERPROFILE"];
    names.iter().find_map(|name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

fn state_dir() -> Option<PathBuf> {
    std::env::var_os("CORTEX_HOOK_STATE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".cortex/hook-notices-v1")))
}

fn is_hook_notice(message: &str) -> bool {
    message == UNAVAILABLE_NOTICE || message.starts_with(START_FAILED_PREFIX)
}

fn start_failed_notice(reason: &str) -> String {
    format!("{START_FAILED_PREFIX}{reason}). This prompt continues without saved Cortex memory.")
}

/// An endpoint other than the platform default belongs to a runtime this hook
/// did not start (tests, side-by-side installs), so it is never autostarted.
fn explicit_endpoint(args: &Args) -> bool {
    #[cfg(windows)]
    {
        args.pipe
            .as_deref()
            .is_some_and(|pipe| !pipe.is_empty() && pipe != PIPE_DEFAULT)
    }
    #[cfg(unix)]
    {
        let default = home_dir().map(|home| home.join(".cortex/adapter-claude.sock"));
        args.sock
            .as_deref()
            .is_some_and(|sock| !sock.is_empty() && Some(PathBuf::from(sock)) != default)
    }
}

fn autostart_then_retry(
    rt: &tokio::runtime::Runtime,
    attempt: &impl Fn(&tokio::runtime::Runtime) -> Result<Value, io::ErrorKind>,
    api_bind: String,
    io_deadline: Instant,
) -> Value {
    let Some(directory) = cortex_config::cortex_home().map(|home| home.join("lifecycle")) else {
        return json!({"systemMessage": start_failed_notice("no Cortex home directory")});
    };
    let decision = blocking_until(io_deadline, move || {
        ensure_started(&directory, SystemTime::now(), || {
            spawn_runtime(&directory, &api_bind)
        })
    });
    match decision {
        None => json!({}),
        Some(StartDecision::Failed(reason)) => {
            json!({"systemMessage": start_failed_notice(reason)})
        }
        Some(StartDecision::Spawned | StartDecision::Starting | StartDecision::Busy) => loop {
            match attempt(rt) {
                Ok(value) => break value,
                Err(_) if Instant::now() + RETRY_INTERVAL >= io_deadline => break json!({}),
                Err(_) => std::thread::sleep(RETRY_INTERVAL),
            }
        },
    }
}

/// Launch `cortex-api` (unless its port already answers) and the adapter
/// daemon from the directory holding this `cortex-hook`, detached from the
/// hook's console and job, logging into the lifecycle directory. A child
/// that exits within [`EARLY_EXIT_PROBE`] counts as a failed start.
fn spawn_runtime(log_dir: &Path, api_bind: &str) -> Result<(), &'static str> {
    let bin_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .ok_or("cannot locate cortex-hook")?;
    let binary = |name: &str| bin_dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let adapter = binary("cortex-adapter-claude");
    let api = binary("cortex-api");
    if !adapter.is_file() {
        return Err("cortex-adapter-claude not found next to cortex-hook");
    }
    let mut children = Vec::new();
    if !api_answers(api_bind) {
        if !api.is_file() {
            return Err("cortex-api not found next to cortex-hook");
        }
        children.push(launch(&api, &[], &log_dir.join("api.log"))?);
    }
    children.push(launch(&adapter, &["daemon"], &log_dir.join("adapter.log"))?);
    std::thread::sleep(EARLY_EXIT_PROBE);
    if children
        .iter_mut()
        .any(|child| !matches!(child.try_wait(), Ok(None)))
    {
        return Err("runtime exited immediately; see the lifecycle logs");
    }
    Ok(())
}

fn api_answers(api_bind: &str) -> bool {
    let Ok(mut address) = api_bind.parse::<std::net::SocketAddr>() else {
        return false;
    };
    if address.ip().is_unspecified() {
        address.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
    }
    std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
}

fn launch(binary: &Path, args: &[&str], log: &Path) -> Result<std::process::Child, &'static str> {
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|_| "cannot open runtime log")?;
    let stderr = stdout.try_clone().map_err(|_| "cannot open runtime log")?;
    let mut command = std::process::Command::new(binary);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    if let Some(parent) = log.parent().and_then(Path::parent) {
        command.current_dir(parent);
    }
    detached_spawn(&mut command).map_err(|_| "cannot launch runtime process")
}

#[cfg(windows)]
fn detached_spawn(command: &mut std::process::Command) -> io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    // Leave the client's job object when allowed so closing the session does
    // not kill the runtime; jobs that forbid breakaway reject the flag.
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
    command.spawn().or_else(|_| {
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        command.spawn()
    })
}

#[cfg(unix)]
fn detached_spawn(command: &mut std::process::Command) -> io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    command.process_group(0).spawn()
}

fn notice_event(event: &str) -> bool {
    matches!(event, "SessionStart" | "UserPromptSubmit")
}

fn filter_notice(
    event: &str,
    session_id: &str,
    mut response: Value,
    directory: Option<&Path>,
    deadline: Instant,
) -> Value {
    if response
        .get("systemMessage")
        .and_then(Value::as_str)
        .is_some_and(is_hook_notice)
    {
        let claimed = notice_event(event)
            && valid_session(session_id)
            && directory.is_some_and(|dir| {
                let directory = dir.to_owned();
                let session = session_id.to_owned();
                blocking_until(deadline, move || {
                    claim_notice(&directory, &session, MAX_NOTICE_MARKERS)
                })
                .unwrap_or(false)
            });
        if !claimed {
            response
                .as_object_mut()
                .map(|object| object.remove("systemMessage"));
        }
    }
    response
}

/// Empty SHA-256-named markers reveal no session IDs. Exclusive creation
/// arbitrates concurrent hook processes. Never reset on resume, recovery or
/// cwd changes; never expire automatically, since that would repeat notices
/// for the same actual session. State errors suppress notices, not prompts.
/// A worker that outlives its deadline may claim a notice that was never
/// displayed. This is at-most-once admission, not guaranteed visibility.
/// Admission stops after 4096 observed markers; no unbounded directory scan
/// or automatic deletion is performed. Only a newly created marker emits.
fn claim_notice(directory: &Path, session_id: &str, limit: usize) -> bool {
    if !valid_session(session_id) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        if std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .is_err()
        {
            return false;
        }
    }
    #[cfg(not(unix))]
    if std::fs::create_dir_all(directory).is_err() {
        return false;
    }
    let metadata = match std::fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return false;
        }
    }
    let digest = format!("{:x}", Sha256::digest(session_id.as_bytes()));
    let marker = directory.join(format!("{digest}.warned"));
    // Common repeat path avoids enumerating the directory on every prompt.
    if marker.exists() {
        return false;
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    if entries.take(limit).count() >= limit {
        return false;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(marker).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice_deadline() -> Instant {
        Instant::now() + Duration::from_secs(1)
    }

    #[test]
    fn blocked_worker_times_out_without_waiting_for_shutdown() {
        let (release, wait) = std::sync::mpsc::channel();
        let started = Instant::now();
        let result = blocking_until(started + Duration::from_millis(20), move || {
            wait.recv().unwrap();
            true
        });
        let elapsed = started.elapsed();
        let _ = release.send(());
        assert_eq!(result, None);
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn expired_notice_deadline_preserves_real_context_without_claiming_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("private-markers");
        let reply = json!({"systemMessage":UNAVAILABLE_NOTICE, "hookSpecificOutput":{"hookEventName":"UserPromptSubmit", "additionalContext":"real saved context"}});
        let filtered = filter_notice(
            "UserPromptSubmit",
            "actual-session",
            reply,
            Some(&dir),
            Instant::now() - Duration::from_millis(1),
        );
        assert!(filtered.get("systemMessage").is_none());
        assert_eq!(
            filtered["hookSpecificOutput"]["additionalContext"],
            "real saved context"
        );
        assert!(!dir.exists());
    }

    #[test]
    fn payload_identity_and_cwd_override_process_defaults_without_rewriting_payload() {
        let payload =
            json!({"session_id":"actual-session", "cwd":"/actual checkout", "prompt":"keep me"});
        let frame = build_frame(
            "UserPromptSubmit",
            payload.to_string().as_bytes(),
            "stale-env",
            "/wrong",
        )
        .unwrap();
        let value: Value = serde_json::from_str(&frame.line).unwrap();
        assert_eq!(frame.session_id, "actual-session");
        assert_eq!(value["session_id"], "actual-session");
        assert_eq!(value["cwd"], "/actual checkout");
        assert_eq!(value["payload"], payload);
        let frame = build_frame("Stop", b"{}", "fallback-session", "/fallback").unwrap();
        assert_eq!(frame.session_id, "fallback-session");
        assert!(build_frame("Stop", b"[]", "fallback", "/fallback").is_none());
        assert!(build_frame("Stop", b"broken", "fallback", "/fallback").is_none());
        assert!(build_frame(
            "Stop",
            br#"{"session_id":42}"#,
            "wrong-session",
            "/fallback"
        )
        .is_none());
    }

    #[test]
    fn input_frame_and_argument_limits_fail_open_without_truncating_context() {
        assert!(read_bounded(vec![b'x'; MAX_INPUT_BYTES + 1].as_slice()).is_err());
        assert_eq!(read_bounded(b"{}".as_slice()).unwrap(), b"{}");
        for argv in [
            vec!["Stop", "--timeout-ms", "20000"],
            vec!["Stop", "--timeout-ms", "0"],
            vec!["Stop", "--pipe"],
        ] {
            assert!(Args::parse(argv.into_iter().map(String::from)).is_err());
        }
        for event in ["SessionStart", "UserPromptSubmit", "PreToolUse"] {
            let args = Args::parse([event, "--fire-forget"].into_iter().map(String::from)).unwrap();
            assert!(!args.fire_forget);
        }
    }

    #[test]
    fn notice_is_shared_across_events_and_never_resets_or_enters_model_context() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("private-markers");
        let reply = json!({"systemMessage":UNAVAILABLE_NOTICE, "hookSpecificOutput":{"hookEventName":"SessionStart", "additionalContext":"real saved context"}});
        let first = filter_notice(
            "SessionStart",
            "actual-session",
            reply.clone(),
            Some(&dir),
            notice_deadline(),
        );
        assert_eq!(first["systemMessage"], UNAVAILABLE_NOTICE);
        let repeat = filter_notice(
            "UserPromptSubmit",
            "actual-session",
            reply.clone(),
            Some(&dir),
            notice_deadline(),
        );
        assert!(repeat.get("systemMessage").is_none());
        assert_eq!(
            repeat["hookSpecificOutput"]["additionalContext"],
            "real saved context"
        );
        assert!(filter_notice(
            "SessionStart",
            "actual-session",
            reply.clone(),
            Some(&dir),
            notice_deadline()
        )
        .get("systemMessage")
        .is_none());
        assert_eq!(
            filter_notice(
                "UserPromptSubmit",
                "different-session",
                reply.clone(),
                Some(&dir),
                notice_deadline(),
            )["systemMessage"],
            UNAVAILABLE_NOTICE
        );
        assert!(filter_notice(
            "SessionStart",
            "",
            reply.clone(),
            Some(&dir),
            notice_deadline()
        )
        .get("systemMessage")
        .is_none());
        assert!(filter_notice(
            "Stop",
            "third-session",
            reply,
            Some(&dir),
            notice_deadline()
        )
        .get("systemMessage")
        .is_none());
        let unrelated = json!({"systemMessage":"another component's message"});
        assert_eq!(
            filter_notice(
                "Stop",
                "actual-session",
                unrelated.clone(),
                Some(&dir),
                notice_deadline()
            ),
            unrelated
        );
    }

    #[test]
    fn start_failure_notice_is_shown_once_per_session_and_never_as_context() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("private-markers");
        let notice = start_failed_notice("cortex-api not found next to cortex-hook");
        let reply = json!({"systemMessage": notice});
        let first = filter_notice(
            "SessionStart",
            "boot-session",
            reply.clone(),
            Some(&dir),
            notice_deadline(),
        );
        assert_eq!(first["systemMessage"], notice);
        assert!(first.get("hookSpecificOutput").is_none());
        for event in ["UserPromptSubmit", "SessionStart"] {
            assert!(filter_notice(
                event,
                "boot-session",
                reply.clone(),
                Some(&dir),
                notice_deadline()
            )
            .get("systemMessage")
            .is_none());
        }
        assert!(filter_notice(
            "PreToolUse",
            "other-session",
            reply,
            Some(&dir),
            notice_deadline()
        )
        .get("systemMessage")
        .is_none());
    }

    #[test]
    fn only_the_default_endpoint_is_autostarted() {
        let parse = |argv: &[&str]| Args::parse(argv.iter().map(|a| a.to_string())).unwrap();
        assert!(!explicit_endpoint(&parse(&["SessionStart"])));
        #[cfg(windows)]
        {
            assert!(!explicit_endpoint(&parse(&[
                "SessionStart",
                "--pipe",
                PIPE_DEFAULT
            ])));
            assert!(explicit_endpoint(&parse(&[
                "SessionStart",
                "--pipe",
                r"\\.\pipe\other"
            ])));
        }
        #[cfg(unix)]
        assert!(explicit_endpoint(&parse(&[
            "SessionStart",
            "--sock",
            "/tmp/other.sock"
        ])));
    }

    #[test]
    fn concurrent_claims_emit_once_and_marker_policy_is_bounded_and_private() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("private-markers");
        let claims: Vec<_> = (0..8)
            .map(|_| {
                let dir = dir.clone();
                std::thread::spawn(move || claim_notice(&dir, "same-session", 16))
            })
            .collect();
        assert_eq!(
            claims
                .into_iter()
                .filter_map(|claim| claim.join().ok())
                .filter(|claimed| *claimed)
                .count(),
            1
        );
        assert!(!claim_notice(&dir, "same-session", 16));
        assert!(!claim_notice(&dir, "another-session", 1));
        assert!(!claim_notice(&dir, "bad\nsession", 16));
        assert!(!claim_notice(&dir, &"s".repeat(257), 16));
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_name().to_string_lossy().len(), 71);
        assert_eq!(entries[0].metadata().unwrap().len(), 0);
        assert!(!claim_notice(&entries[0].path(), "new-session", 16));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                entries[0].metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn response_stops_at_first_line_without_waiting_for_eof() {
        let (mut reader, mut writer) = tokio::io::duplex(128);
        writer
            .write_all(b"{\"hookSpecificOutput\":{\"additionalContext\":\"saved\"}}\n")
            .await
            .unwrap();
        let response = tokio::time::timeout(Duration::from_millis(100), read_reply(&mut reader))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response["hookSpecificOutput"]["additionalContext"], "saved");
        drop(writer);
    }

    #[tokio::test]
    async fn reply_size_shape_and_utf8_are_validated_and_write_is_inside_deadline() {
        for bytes in [
            b"[]\n".to_vec(),
            b"not json\n".to_vec(),
            vec![0xff, b'\n'],
            vec![b'x'; MAX_REPLY_BYTES + 1],
        ] {
            let mut reader = bytes.as_slice();
            assert!(read_reply(&mut reader).await.is_err());
        }
        let (mut stream, _peer_not_reading) = tokio::io::duplex(8);
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            exchange(&mut stream, &[b'x'; 128], false)
        )
        .await
        .is_err());
        let (mut stream, mut peer) = tokio::io::duplex(128);
        let task = tokio::spawn(async move {
            let mut frame = [0; 3];
            peer.read_exact(&mut frame).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            exchange(&mut stream, b"{}\n", false)
        )
        .await
        .is_err());
        task.abort();
    }
}
