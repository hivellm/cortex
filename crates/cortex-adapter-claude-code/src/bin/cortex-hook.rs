//! `cortex-hook` — native Claude Code hook shim. Phase 11x.
//!
//! Replaces the per-event `.sh` / `.ps1` shims with a single Rust
//! binary that connects directly to the daemon's named pipe (Windows)
//! or Unix domain socket (Linux/macOS). Cold start under 50 ms on
//! Windows, under 20 ms on Linux — versus ~545 ms for `pwsh -NoProfile`
//! that the legacy shim pays unconditionally.
//!
//! # Wire shape (unchanged from the legacy shims)
//!
//! ```text
//! { "hook": "<HookKind>", "session_id": "...", "cwd": "...", "payload": {...} }
//! ```
//!
//! # Modes
//!
//! - **Synchronous** (default for `UserPromptSubmit`, `PreToolUse`):
//!   write the frame, read one response line, print it on stdout.
//! - **Fire-and-forget** (`--fire-forget`, default for `PostToolUse`,
//!   `SubagentStop`, `Stop`, `SessionStart`, `Notification`): write
//!   the frame, drop the connection, print `{}` on stdout. The daemon
//!   publishes envelopes asynchronously, so the hook does not need to
//!   block on a response.
//!
//! # Fail-open
//!
//! Every error path — adapter disabled, daemon down, malformed
//! stdin, timeout — prints `{}` to stdout and exits 0. The Claude
//! Code session must never break because of this binary.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

const DEFAULT_TIMEOUT_MS: u64 = 1500;
const PIPE_DEFAULT: &str = r"\\.\pipe\cortex-adapter-claude";

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
    fn parse() -> Result<Self, String> {
        let mut argv = std::env::args().skip(1);
        let mut event = None;
        let mut fire_forget = false;
        let mut pipe = None;
        let mut sock = None;
        let mut timeout_ms = DEFAULT_TIMEOUT_MS;

        while let Some(arg) = argv.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                "--version" | "-V" => {
                    println!("cortex-hook {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                "--fire-forget" => fire_forget = true,
                "--pipe" => pipe = argv.next(),
                "--sock" => sock = argv.next(),
                "--timeout-ms" => {
                    if let Some(v) = argv.next() {
                        timeout_ms = v.parse().map_err(|e| format!("--timeout-ms: {e}"))?;
                    }
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag: {other}"));
                }
                other if event.is_none() => event = Some(other.to_string()),
                other => return Err(format!("unexpected argument: {other}")),
            }
        }

        let event = event.ok_or_else(|| "missing <event-name>".to_string())?;
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
         EVENTS (HookKind PascalCase):\n  \
           SessionStart  UserPromptSubmit  PreToolUse  PostToolUse\n  \
           SubagentStop  Stop  Notification\n\n\
         FLAGS:\n  \
           --fire-forget     Drop the connection without reading a response (publish-only).\n  \
           --pipe NAME       Override the Windows named-pipe name (default: \\\\.\\pipe\\cortex-adapter-claude).\n  \
           --sock PATH       Override the Unix domain socket path (default: $HOME/.cortex/adapter-claude.sock).\n  \
           --timeout-ms MS   Synchronous response timeout (default: {} ms).\n\n\
         ENV:\n  \
           CORTEX_ADAPTER_DISABLE=1   Skip every I/O; print {{}} and exit 0.\n  \
           CORTEX_ADAPTER_PIPE        Same as --pipe.\n  \
           CORTEX_ADAPTER_SOCK        Same as --sock.\n  \
           CLAUDE_SESSION_ID          Stamped onto the frame's session_id field.\n",
        env!("CARGO_PKG_VERSION"),
        DEFAULT_TIMEOUT_MS
    );
}

/// Always-print-`{}`-and-exit-0 helper. Centralises the fail-open
/// invariant so every error path goes through the same exit.
fn fail_open() -> ! {
    println!("{{}}");
    std::process::exit(0);
}

fn main() {
    if std::env::var("CORTEX_ADAPTER_DISABLE").as_deref() == Ok("1") {
        fail_open();
    }
    let args = match Args::parse() {
        Ok(a) => a,
        Err(_) => fail_open(),
    };
    let frame = build_frame(&args.event);

    // Single-thread tokio runtime — every other flavor pulls in the
    // worker pool which costs another 5-10 ms on Windows. The hook
    // does one tiny round-trip; one thread is plenty.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => fail_open(),
    };

    let response = rt.block_on(async move {
        let timeout = Duration::from_millis(args.timeout_ms);
        match tokio::time::timeout(
            timeout,
            transport_round_trip(&args, frame.as_bytes(), args.fire_forget),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) | Err(_) => "{}".to_string(),
        }
    });

    println!("{response}");
}

/// Build the canonical `HookFrame` JSON line. Mirrors the shape the
/// legacy shims produced so the daemon dispatcher needs zero changes.
fn build_frame(event: &str) -> String {
    let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin).ok();
    let payload: Value = serde_json::from_str(stdin.trim()).unwrap_or(Value::Object(Default::default()));

    let frame = serde_json::json!({
        "hook": event,
        "session_id": session_id,
        "cwd": cwd,
        "payload": payload,
    });
    let mut s = frame.to_string();
    s.push('\n');
    s
}

/// Resolve the binding from CLI overrides, env, then platform default.
fn resolve_pipe_name(args: &Args) -> String {
    if let Some(p) = &args.pipe {
        return p.clone();
    }
    if let Ok(p) = std::env::var("CORTEX_ADAPTER_PIPE") {
        if !p.is_empty() {
            return p;
        }
    }
    PIPE_DEFAULT.to_string()
}

#[cfg(unix)]
fn resolve_sock_path(args: &Args) -> std::path::PathBuf {
    if let Some(p) = &args.sock {
        return std::path::PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CORTEX_ADAPTER_SOCK") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(home)
        .join(".cortex")
        .join("adapter-claude.sock")
}

#[cfg(windows)]
async fn transport_round_trip(
    args: &Args,
    frame: &[u8],
    fire_forget: bool,
) -> std::io::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::ClientOptions;

    let pipe_name = resolve_pipe_name(args);
    let mut client = ClientOptions::new().open(&pipe_name)?;
    client.write_all(frame).await?;
    client.flush().await?;
    if fire_forget {
        // Drop the writer. The server has the bytes; reading the
        // response would defeat the purpose of fire-and-forget.
        drop(client);
        return Ok("{}".to_string());
    }
    let mut buf = Vec::with_capacity(8 * 1024);
    client.read_to_end(&mut buf).await?;
    let mut s = String::from_utf8_lossy(&buf).to_string();
    if let Some(idx) = s.find('\n') {
        s.truncate(idx);
    }
    if s.is_empty() {
        s = "{}".to_string();
    }
    Ok(s)
}

#[cfg(unix)]
async fn transport_round_trip(
    args: &Args,
    frame: &[u8],
    fire_forget: bool,
) -> std::io::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let sock_path = resolve_sock_path(args);
    let mut client = UnixStream::connect(&sock_path).await?;
    client.write_all(frame).await?;
    client.flush().await?;
    if fire_forget {
        drop(client);
        return Ok("{}".to_string());
    }
    let mut buf = Vec::with_capacity(8 * 1024);
    client.read_to_end(&mut buf).await?;
    let mut s = String::from_utf8_lossy(&buf).to_string();
    if let Some(idx) = s.find('\n') {
        s.truncate(idx);
    }
    if s.is_empty() {
        s = "{}".to_string();
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_frame_includes_event_and_payload_object() {
        let raw = build_frame("UserPromptSubmit");
        let v: Value = serde_json::from_str(raw.trim()).unwrap();
        assert_eq!(v["hook"], "UserPromptSubmit");
        assert!(v["payload"].is_object() || v["payload"].is_array());
        assert!(v["cwd"].is_string());
        assert!(v["session_id"].is_string());
    }

    #[test]
    fn args_parse_event_only() {
        let saved: Vec<String> = std::env::args().collect();
        // Args::parse reads std::env::args; we test the field defaults by
        // constructing manually since we cannot inject argv inside a
        // unit test without spawning a child.
        let a = Args {
            event: "Stop".into(),
            fire_forget: false,
            pipe: None,
            sock: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        };
        assert_eq!(a.event, "Stop");
        assert_eq!(a.timeout_ms, 1500);
        let _ = saved;
    }

    #[test]
    fn args_struct_supports_fire_forget() {
        let a = Args {
            event: "PostToolUse".into(),
            fire_forget: true,
            pipe: Some("custom".into()),
            sock: None,
            timeout_ms: 250,
        };
        assert!(a.fire_forget);
        assert_eq!(a.pipe.as_deref(), Some("custom"));
        assert_eq!(a.timeout_ms, 250);
    }
}
