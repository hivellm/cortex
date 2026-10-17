//! Phase35 §1.5 — app-managed start of the daily runtime from `cortex-hook`.
//!
//! When a synchronous hook finds the default adapter endpoint absent, it asks
//! [`ensure_started`] to launch the runtime. There is no OS autostart (Run
//! key, scheduled task, Startup entry or service); the first session after a
//! reboot brings the runtime back.
//!
//! Coordination uses two files in a private state directory:
//! - `spawn.lock`, created exclusively for the few milliseconds a hook spends
//!   deciding and spawning. A concurrent hook that finds it returns
//!   [`StartDecision::Busy`]. A lock older than [`STALE_LOCK`] belonged to a
//!   killed hook and is replaced.
//! - `last-spawn`, holding the attempt time and outcome. Within
//!   [`SPAWN_GRACE`] of a successful attempt the runtime is assumed to be
//!   booting ([`StartDecision::Starting`]); after the grace and until
//!   [`SPAWN_BACKOFF`], or after a failed attempt, the start is reported as
//!   failed without spawning again. Past the backoff a new attempt is made.

use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Time a freshly spawned runtime gets to create its endpoint.
pub const SPAWN_GRACE: Duration = Duration::from_secs(10);
/// Minimum time between two spawn attempts.
pub const SPAWN_BACKOFF: Duration = Duration::from_secs(60);
/// Age after which a leftover `spawn.lock` is considered abandoned.
pub const STALE_LOCK: Duration = Duration::from_secs(10);

const LOCK_FILE: &str = "spawn.lock";
const STAMP_FILE: &str = "last-spawn";

/// Outcome of one [`ensure_started`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartDecision {
    /// This call launched the runtime.
    Spawned,
    /// A recent attempt succeeded; the runtime is still booting.
    Starting,
    /// Another hook holds the spawn lock right now.
    Busy,
    /// The runtime could not be started; the reason is safe to show.
    Failed(&'static str),
}

/// Whether a hook should try to start the runtime after a failed exchange:
/// only synchronous events, only for the default endpoint (an explicit pipe or
/// socket names a runtime this hook does not own), only when the endpoint is
/// absent rather than slow, and never when autostart is disabled
/// (`adapter.hook_autostart = false`, env `CORTEX_HOOK_AUTOSTART=0`).
#[must_use]
pub fn should_autostart(
    event: &str,
    explicit_endpoint: bool,
    autostart_enabled: bool,
    error: ErrorKind,
) -> bool {
    matches!(event, "SessionStart" | "UserPromptSubmit" | "PreToolUse")
        && !explicit_endpoint
        && autostart_enabled
        && matches!(error, ErrorKind::NotFound | ErrorKind::ConnectionRefused)
}

/// Start the runtime through `spawn` unless another hook is doing it or the
/// backoff forbids a new attempt. Never blocks on other hooks.
pub fn ensure_started(
    directory: &Path,
    now: SystemTime,
    spawn: impl FnOnce() -> Result<(), &'static str>,
) -> StartDecision {
    if create_private_dir(directory).is_err() {
        return StartDecision::Failed("lifecycle state directory is unavailable");
    }
    let lock = directory.join(LOCK_FILE);
    if !acquire(&lock, now) {
        return StartDecision::Busy;
    }
    let decision = decide_and_spawn(directory, now, spawn);
    let _ = std::fs::remove_file(&lock);
    decision
}

fn decide_and_spawn(
    directory: &Path,
    now: SystemTime,
    spawn: impl FnOnce() -> Result<(), &'static str>,
) -> StartDecision {
    let stamp = directory.join(STAMP_FILE);
    if let Some((attempted, succeeded)) = read_stamp(&stamp) {
        if let Ok(age) = now.duration_since(attempted) {
            if succeeded && age < SPAWN_GRACE {
                return StartDecision::Starting;
            }
            if age < SPAWN_BACKOFF {
                return StartDecision::Failed("runtime did not start; retrying after backoff");
            }
        }
    }
    let result = spawn();
    let millis = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let outcome = if result.is_ok() { "ok" } else { "failed" };
    let _ = std::fs::write(&stamp, format!("{millis} {outcome}\n"));
    match result {
        Ok(()) => StartDecision::Spawned,
        Err(reason) => StartDecision::Failed(reason),
    }
}

fn acquire(lock: &Path, now: SystemTime) -> bool {
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock)
        {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                return true;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let abandoned = std::fs::metadata(lock)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age > STALE_LOCK);
                if !abandoned || std::fs::remove_file(lock).is_err() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    false
}

fn read_stamp(path: &Path) -> Option<(SystemTime, bool)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut parts = raw.split_whitespace();
    let millis: u64 = parts.next()?.parse().ok()?;
    let succeeded = parts.next()? == "ok";
    Some((UNIX_EPOCH + Duration::from_millis(millis), succeeded))
}

fn create_private_dir(directory: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    #[test]
    fn absent_runtime_is_spawned_then_reported_starting_within_grace() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("lifecycle");
        let spawns = AtomicUsize::new(0);
        let spawn = || {
            spawns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        assert_eq!(ensure_started(&dir, at(0), spawn), StartDecision::Spawned);
        assert_eq!(
            ensure_started(&dir, at(5), || panic!("must not respawn")),
            StartDecision::Starting
        );
        assert_eq!(spawns.load(Ordering::SeqCst), 1);
        assert!(!dir.join(LOCK_FILE).exists());
    }

    #[test]
    fn backoff_is_honored_after_grace_and_after_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("lifecycle");
        assert_eq!(
            ensure_started(&dir, at(0), || Ok(())),
            StartDecision::Spawned
        );
        assert!(matches!(
            ensure_started(&dir, at(30), || panic!("inside backoff")),
            StartDecision::Failed(_)
        ));
        assert_eq!(
            ensure_started(&dir, at(61), || Ok(())),
            StartDecision::Spawned
        );

        assert_eq!(
            ensure_started(&dir, at(200), || Err("adapter binary not found")),
            StartDecision::Failed("adapter binary not found")
        );
        assert!(matches!(
            ensure_started(&dir, at(201), || panic!("failed attempt backs off")),
            StartDecision::Failed(_)
        ));
        assert_eq!(
            ensure_started(&dir, at(261), || Ok(())),
            StartDecision::Spawned
        );
    }

    #[test]
    fn clock_moving_backwards_allows_a_new_attempt() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("lifecycle");
        assert_eq!(
            ensure_started(&dir, at(100), || Ok(())),
            StartDecision::Spawned
        );
        assert_eq!(
            ensure_started(&dir, at(0), || Ok(())),
            StartDecision::Spawned
        );
    }

    #[test]
    fn concurrent_hooks_spawn_exactly_once() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Arc::new(tmp.path().join("lifecycle"));
        let spawns = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (dir, spawns, barrier) = (dir.clone(), spawns.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    ensure_started(&dir, at(0), || {
                        spawns.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                })
            })
            .collect();
        let decisions: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(spawns.load(Ordering::SeqCst), 1);
        assert_eq!(
            decisions
                .iter()
                .filter(|d| **d == StartDecision::Spawned)
                .count(),
            1
        );
        assert!(decisions.iter().all(|d| matches!(
            d,
            StartDecision::Spawned | StartDecision::Starting | StartDecision::Busy
        )));
    }

    #[test]
    fn held_lock_is_busy_and_abandoned_lock_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("lifecycle");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOCK_FILE), "1\n").unwrap();
        let written = std::fs::metadata(dir.join(LOCK_FILE))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            ensure_started(&dir, written + Duration::from_secs(1), || panic!("locked")),
            StartDecision::Busy
        );
        assert!(dir.join(LOCK_FILE).exists());
        assert_eq!(
            ensure_started(&dir, written + STALE_LOCK + Duration::from_secs(1), || Ok(
                ()
            )),
            StartDecision::Spawned
        );
        assert!(!dir.join(LOCK_FILE).exists());
    }

    #[test]
    fn unusable_state_directory_fails_without_spawning() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-directory");
        std::fs::write(&file, "").unwrap();
        assert!(matches!(
            ensure_started(&file.join("lifecycle"), at(0), || panic!("no state")),
            StartDecision::Failed(_)
        ));
    }

    #[test]
    fn present_endpoint_failures_never_autostart_a_running_daemon() {
        // A successful exchange never reaches `should_autostart`; these are
        // the errors a daemon that exists but is slow, busy, restarting or
        // owned by someone else produces. None of them may spawn a second
        // runtime next to it.
        let present = [
            ErrorKind::TimedOut,
            ErrorKind::WouldBlock,
            ErrorKind::BrokenPipe,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::UnexpectedEof,
            ErrorKind::InvalidData,
            ErrorKind::PermissionDenied,
            ErrorKind::AddrInUse,
            ErrorKind::Interrupted,
            ErrorKind::Other,
        ];
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "SubagentStop",
            "Notification",
        ] {
            for error in present {
                assert!(
                    !should_autostart(event, false, true, error),
                    "{event} {error:?}"
                );
            }
        }
    }

    #[test]
    fn recent_successful_spawn_leaves_the_booting_runtime_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("lifecycle");
        assert_eq!(
            ensure_started(&dir, at(0), || Ok(())),
            StartDecision::Spawned
        );
        let stamp = std::fs::read(dir.join(STAMP_FILE)).unwrap();
        for second in [0, 1, 9] {
            assert_eq!(
                ensure_started(&dir, at(second), || panic!("runtime is booting")),
                StartDecision::Starting
            );
        }
        assert_eq!(std::fs::read(dir.join(STAMP_FILE)).unwrap(), stamp);
        assert!(!dir.join(LOCK_FILE).exists());
    }

    #[test]
    fn autostart_only_for_absent_default_endpoint_on_synchronous_events() {
        for event in ["SessionStart", "UserPromptSubmit", "PreToolUse"] {
            assert!(should_autostart(event, false, true, ErrorKind::NotFound));
            assert!(should_autostart(
                event,
                false,
                true,
                ErrorKind::ConnectionRefused
            ));
        }
        assert!(!should_autostart("Stop", false, true, ErrorKind::NotFound));
        assert!(!should_autostart(
            "SessionStart",
            true,
            true,
            ErrorKind::NotFound
        ));
        assert!(!should_autostart(
            "SessionStart",
            false,
            false,
            ErrorKind::NotFound
        ));
        assert!(!should_autostart(
            "SessionStart",
            false,
            true,
            ErrorKind::TimedOut
        ));
        assert!(!should_autostart(
            "SessionStart",
            false,
            true,
            ErrorKind::InvalidData
        ));
    }
}
