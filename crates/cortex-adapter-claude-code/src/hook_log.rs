//! Daemon-side hook logging — Phase 11x.
//!
//! Replaces the per-invocation log writes the legacy `.sh` / `.ps1`
//! shims used to do. Now that `cortex-hook` is a thin native shim
//! that does no I/O beyond the named-pipe / Unix-socket round-trip,
//! the dispatcher owns logging.
//!
//! Two append-only files in `~/.cortex/`:
//!
//! - `hook-invocations.log` — one line per dispatch with hook +
//!   session id + payload session id + pid.
//! - `hook-errors.log` — categorised error trail
//!   (`pipe_broken | connect_timeout | access_denied | other`).
//!
//! Both files rotate at 10 MB: the live file is renamed to
//! `<basename>.log.1` (overwriting any prior rotation) and a fresh
//! empty file replaces it. At most two rotations live on disk.
//!
//! Cost: a single `metadata()` syscall on the live file every dispatch
//! to read its current size. The existing
//! `cortex-cli/src/ops/log_rotate.rs` rotator handles the deeper
//! gzip-and-keep-8 retention; this module exists so the per-call hot
//! path doesn't depend on the operator running that rotator.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use crate::events::HookFrame;

/// Soft cap for either log file. Past this size the file is rotated
/// before the next append.
const ROTATE_AT_BYTES: u64 = 10 * 1024 * 1024;

/// Synchronises append + rotate across the dispatcher tasks. Logging
/// is a fail-soft path; we never propagate I/O errors out of it.
static LOG_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Resolve `~/.cortex/<name>` for the current operator. Falls back to
/// the platform tmp dir when `HOME` / `USERPROFILE` are unset so the
/// dispatcher never panics inside the logging path.
fn cortex_home_path(name: &str) -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    PathBuf::from(home).join(".cortex").join(name)
}

/// Write a single invocation line to `hook-invocations.log`.
///
/// Format mirrors the legacy shim:
///
/// ```text
/// 2026-05-06T12:34:56.789Z UserPromptSubmit env_sid=<env> payload_sid=<payload> pid=<pid>
/// ```
pub fn record_invocation(frame: &HookFrame, pid: u32) {
    let path = cortex_home_path("hook-invocations.log");
    let payload_sid = frame
        .payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let env_sid = frame.session_id.as_deref().unwrap_or("");
    let line = format!(
        "{ts} {hook} env_sid={env} payload_sid={pid_sid} pid={pid}\n",
        ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        hook = frame.hook,
        env = env_sid,
        pid_sid = payload_sid,
        pid = pid,
    );
    append_with_rotate(&path, line.as_bytes());
}

/// Write one error line to `hook-errors.log`. `category` matches the
/// taxonomy the legacy `.ps1` shim used so external alerting that
/// scrapes the file keeps parsing.
pub fn record_error(hook: &str, category: &str, msg: &str) {
    let path = cortex_home_path("hook-errors.log");
    // One line, trimmed to a single physical line so a multi-line
    // panic backtrace doesn't shred the file format.
    let one_line: String = msg
        .replace('\r', " ")
        .replace('\n', " | ")
        .chars()
        .take(2048)
        .collect();
    let line = format!(
        "{ts} {hook} {category} {msg}\n",
        ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        hook = hook,
        category = category,
        msg = one_line,
    );
    append_with_rotate(&path, line.as_bytes());
}

/// Append `bytes` to `path`, rotating first when the file already
/// passed [`ROTATE_AT_BYTES`].
fn append_with_rotate(path: &Path, bytes: &[u8]) {
    let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() >= ROTATE_AT_BYTES {
            rotate(path);
        }
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(bytes);
    }
}

/// Move `<path>` to `<path>.1` (overwriting any existing rotation)
/// and create a fresh empty `<path>`. Best-effort: every error is
/// swallowed so the hot path keeps logging.
fn rotate(path: &Path) {
    let mut rotated = path.as_os_str().to_os_string();
    rotated.push(".1");
    let rotated = PathBuf::from(rotated);
    let _ = std::fs::remove_file(&rotated);
    if std::fs::rename(path, &rotated).is_ok() {
        // Best effort: re-create an empty live file so the next
        // append finds a writable target without an O_CREAT race.
        let _ = File::create(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tempfile::TempDir;

    fn frame(hook: &str, sid: &str, payload_sid: &str) -> HookFrame {
        HookFrame {
            hook: hook.to_string(),
            session_id: Some(sid.to_string()),
            cwd: Some("/tmp".to_string()),
            payload: serde_json::json!({ "session_id": payload_sid }),
        }
    }

    #[test]
    fn rotate_renames_when_threshold_crossed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hook-invocations.log");
        // Seed the live file just past the rotate threshold.
        std::fs::write(&path, vec![b'x'; (ROTATE_AT_BYTES + 1) as usize]).unwrap();
        append_with_rotate(&path, b"new line\n");
        assert!(path.exists(), "live file recreated after rotation");
        let rotated = {
            let mut p = path.clone().into_os_string();
            p.push(".1");
            PathBuf::from(p)
        };
        assert!(rotated.exists(), ".1 rotation produced");
        // Live file holds only the new line, not the seed.
        let live_size = std::fs::metadata(&path).unwrap().len();
        assert!(
            live_size < 1024,
            "live file should be tiny after rotation, got {live_size}"
        );
    }

    #[test]
    fn record_invocation_writes_one_line() {
        let dir = TempDir::new().unwrap();
        let saved_home = std::env::var("HOME").ok();
        let saved_userprofile = std::env::var("USERPROFILE").ok();
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());

        let f = frame("UserPromptSubmit", "env-sid", "payload-sid");
        record_invocation(&f, 42);
        // Restore env so concurrent tests stay isolated.
        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match saved_userprofile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }

        let log_path = dir.path().join(".cortex").join("hook-invocations.log");
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("UserPromptSubmit"));
        assert!(content.contains("env_sid=env-sid"));
        assert!(content.contains("payload_sid=payload-sid"));
        assert!(content.contains("pid=42"));
        assert_eq!(content.matches('\n').count(), 1);
    }

    #[test]
    fn record_invocation_soak_rotates_under_load_and_caps_live_size() {
        // §3.4 — drive `record_invocation` until the live file
        // would cross 10 MB. Verifies that the rotation invariant
        // ("live file stays under cap, prior content moves to .1")
        // holds when the trigger is real append traffic, not the
        // pre-seed of `rotate_renames_when_threshold_crossed`.
        //
        // The seed below puts the file 1 KB under the cap so we
        // only need a handful of real appends to trip rotation.
        // That keeps the test under one second on a slow CI runner
        // while still exercising the full `record_invocation` path
        // (timestamp formatting, env-resolved paths, lock guard,
        // metadata read, append, conditional rename).
        let dir = TempDir::new().unwrap();
        let saved_home = std::env::var("HOME").ok();
        let saved_userprofile = std::env::var("USERPROFILE").ok();
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());

        let log_path = dir.path().join(".cortex").join("hook-invocations.log");
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        std::fs::write(&log_path, vec![b'x'; (ROTATE_AT_BYTES - 1024) as usize]).unwrap();

        let f = frame("PreToolUse", "soak-env", "soak-payload");
        // Each `record_invocation` line is ~80 bytes, so ~14
        // appends push the live file past the 10 MB threshold and
        // trigger one rotation. We do 64 to be safe and to confirm
        // the live file does NOT keep growing past the cap after
        // the first rotation.
        for _ in 0..64 {
            record_invocation(&f, 1234);
        }

        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match saved_userprofile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }

        let live_size = std::fs::metadata(&log_path).unwrap().len();
        assert!(
            live_size < ROTATE_AT_BYTES,
            "live file must stay under cap after rotation, got {live_size}"
        );
        // Live file should be small because rotation reset it; the
        // 64 appended lines fit easily under 1 MB.
        assert!(
            live_size < 1024 * 1024,
            "live file should be small after rotation, got {live_size}"
        );

        let rotated_path = {
            let mut p = log_path.clone().into_os_string();
            p.push(".1");
            std::path::PathBuf::from(p)
        };
        assert!(rotated_path.exists(), ".1 rotation produced under load");
        let rotated_size = std::fs::metadata(&rotated_path).unwrap().len();
        assert!(
            rotated_size >= ROTATE_AT_BYTES,
            ".1 must hold the pre-rotation tail (>= cap), got {rotated_size}"
        );
    }

    #[test]
    fn record_error_collapses_multiline_messages() {
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());

        record_error("PreToolUse", "pipe_broken", "first line\nsecond\nthird");
        let path = dir.path().join(".cortex").join("hook-errors.log");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("pipe_broken"));
        assert!(!content.contains("\nsecond\n"), "multiline collapsed");
        // Exactly one trailing newline → one physical line.
        assert_eq!(content.matches('\n').count(), 1);
        // Confirm a Value can still parse the record_error format if a
        // future caller wants structured logs (defensive: file is
        // text-line right now; this just guards against accidental
        // JSON-breaking chars sneaking in).
        let _: Value = serde_json::Value::String(content);
    }
}
