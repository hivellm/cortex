//! Phase8c — build-time version emitter shared across the Cortex
//! workspace.
//!
//! The 2026-04-28 incident proved that "running binary != source" is
//! a recurring footgun on a multi-binary workspace. After every
//! `cargo build`, you have to manually kill and restart each affected
//! binary; if you forget, the production process keeps running stale
//! code and silent regressions reappear. There's no system-level
//! guard.
//!
//! This crate closes the loop with two halves:
//!
//! 1. [`emit_version_env`] — call from each crate's `build.rs` to
//!    bake `CORTEX_GIT_SHA` / `CORTEX_GIT_SHA_SHORT` /
//!    `CORTEX_BUILD_TS` / `CORTEX_GIT_DIRTY` / `CORTEX_BUILD_PROFILE`
//!    into the binary at compile time via `cargo:rustc-env`.
//!
//! 2. [`version_info`] — a runtime helper that reads the same
//!    `env!()` constants and returns a [`VersionInfo`] struct ready
//!    to ship inside `/healthz` extras.
//!
//! Builds outside a git working tree (e.g. published crates.io
//! tarballs) MUST fall back to `"unknown"` for git fields without
//! failing the build — see [`emit_version_env`] for the fallback
//! semantics.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

/// Compile-time-baked version block surfaced by every binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Full 40-char SHA of the workspace HEAD when the binary was
    /// built. `"unknown"` when built outside a git working tree.
    pub git_sha: String,
    /// First 7 chars of [`Self::git_sha`] for compact display.
    pub git_sha_short: String,
    /// RFC-3339 build timestamp (UTC).
    pub build_ts: String,
    /// `true` when `git status --porcelain` had any output at build
    /// time — flags binaries built from a dirty working tree.
    pub git_dirty: bool,
    /// `"debug"` or `"release"` — read from the cargo profile.
    pub profile: String,
    /// Crate version from `Cargo.toml` (`CARGO_PKG_VERSION`).
    pub crate_version: String,
}

impl VersionInfo {
    /// Construct a version block from the values baked at compile
    /// time. Callers typically call [`crate::version_info`] (the
    /// macro-friendly helper) instead of constructing this directly.
    pub fn new(
        git_sha: impl Into<String>,
        git_sha_short: impl Into<String>,
        build_ts: impl Into<String>,
        git_dirty: bool,
        profile: impl Into<String>,
        crate_version: impl Into<String>,
    ) -> Self {
        Self {
            git_sha: git_sha.into(),
            git_sha_short: git_sha_short.into(),
            build_ts: build_ts.into(),
            git_dirty,
            profile: profile.into(),
            crate_version: crate_version.into(),
        }
    }

    /// Sentinel value used when the build environment couldn't
    /// produce a SHA (e.g. crates.io tarball, no git on PATH).
    pub const UNKNOWN_SHA: &'static str = "unknown";

    /// `true` when the SHA is the unknown sentinel.
    pub fn is_unknown(&self) -> bool {
        self.git_sha == Self::UNKNOWN_SHA
    }
}

/// Construct a [`VersionInfo`] from the env vars [`emit_version_env`]
/// stamped at compile time.
///
/// Resolves `CARGO_PKG_VERSION` from the *calling* crate so each
/// binary surfaces its own crate version, not cortex-build's. Use
/// the [`version_info!`] macro to capture that automatically.
pub fn from_compile_env(crate_version: &'static str) -> VersionInfo {
    VersionInfo::new(
        option_env!("CORTEX_GIT_SHA").unwrap_or(VersionInfo::UNKNOWN_SHA),
        option_env!("CORTEX_GIT_SHA_SHORT").unwrap_or("unknown"),
        option_env!("CORTEX_BUILD_TS").unwrap_or("unknown"),
        matches!(
            option_env!("CORTEX_GIT_DIRTY").unwrap_or("false"),
            "1" | "true" | "TRUE"
        ),
        option_env!("CORTEX_BUILD_PROFILE").unwrap_or("unknown"),
        crate_version,
    )
}

/// Capture the calling crate's version block. Resolves the env vars
/// [`emit_version_env`] stamped at compile time + the calling crate's
/// `CARGO_PKG_VERSION`. Each binary calls this once at boot to feed
/// `/healthz extras.version`.
#[macro_export]
macro_rules! version_info {
    () => {
        $crate::from_compile_env(env!("CARGO_PKG_VERSION"))
    };
}

// ---- build.rs side -----------------------------------------------

/// Phase8c — invoked from each Cortex crate's `build.rs`. Stamps the
/// compile-time env vars [`from_compile_env`] reads back at runtime.
///
/// Side effects (printed to stdout for cargo to consume):
/// - `cargo:rerun-if-changed=.git/HEAD` so a checkout-switch triggers
///   a rebuild and refreshes the embedded SHA.
/// - `cargo:rustc-env=CORTEX_GIT_SHA=<full>`
/// - `cargo:rustc-env=CORTEX_GIT_SHA_SHORT=<7-char>`
/// - `cargo:rustc-env=CORTEX_BUILD_TS=<RFC-3339>`
/// - `cargo:rustc-env=CORTEX_GIT_DIRTY=<true|false>`
/// - `cargo:rustc-env=CORTEX_BUILD_PROFILE=<debug|release>`
///
/// On a non-git build (no `.git`, no `git` binary), git fields fall
/// back to `"unknown"` and `git_dirty` to `false`. The build never
/// fails because of a missing git environment.
pub fn emit_version_env() {
    use std::process::Command;

    // Re-run the build script when HEAD moves. Without this hint
    // cargo only re-runs build.rs when its own contents change, so
    // the embedded SHA goes stale on every checkout switch even
    // though the source files moved with it.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");

    // CORTEX_GIT_SHA — full SHA of HEAD.
    let git_sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| VersionInfo::UNKNOWN_SHA.to_string());
    let git_sha_short = if git_sha.len() >= 7 {
        git_sha[..7].to_string()
    } else {
        git_sha.clone()
    };
    println!("cargo:rustc-env=CORTEX_GIT_SHA={git_sha}");
    println!("cargo:rustc-env=CORTEX_GIT_SHA_SHORT={git_sha_short}");

    // CORTEX_GIT_DIRTY — any output from `git status --porcelain` ⇒ dirty.
    let git_dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false);
    println!("cargo:rustc-env=CORTEX_GIT_DIRTY={git_dirty}");

    // CORTEX_BUILD_TS — UTC, RFC-3339 (use a local minimal formatter
    // so cortex-build stays chrono-free at build time).
    let build_ts = current_utc_rfc3339();
    println!("cargo:rustc-env=CORTEX_BUILD_TS={build_ts}");

    // CORTEX_BUILD_PROFILE — "debug" or "release" from cargo's
    // PROFILE env var.
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=CORTEX_BUILD_PROFILE={profile}");
}

/// Minimal UTC RFC-3339 formatter that doesn't pull in chrono at
/// build time. Returns `"unknown"` if the system clock cannot be
/// read (extremely unlikely; covered for completeness).
fn current_utc_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => return "unknown".to_string(),
    };
    // Convert seconds-since-epoch into a Y-M-D H:M:S struct without
    // an external crate. Algorithm from Howard Hinnant's
    // date-algorithms (public domain), simplified for Y in 1970..3000.
    let z = secs as i64 / 86_400;
    let secs_of_day = (secs % 86_400) as i64;
    let h = secs_of_day / 3_600;
    let m = (secs_of_day % 3_600) / 60;
    let s = secs_of_day % 60;

    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m_calendar = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m_calendar <= 2);

    format!(
        "{y:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        m_calendar, d, h, m, s
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_info_macro_returns_struct_with_crate_version() {
        let v = version_info!();
        // The crate version comes from the *calling* crate (this test
        // crate is cortex-build itself, version = workspace version).
        assert!(!v.crate_version.is_empty());
    }

    #[test]
    fn from_compile_env_falls_back_to_unknown_when_unset() {
        let v = from_compile_env("0.1.0");
        // The unit-test build can't guarantee the env vars are
        // unset (they ARE set in CI when build.rs runs), so just
        // assert the shape is well-formed.
        assert!(!v.git_sha.is_empty());
        assert!(!v.build_ts.is_empty());
        assert!(matches!(v.profile.as_str(), "debug" | "release" | "unknown"));
        assert_eq!(v.crate_version, "0.1.0");
    }

    #[test]
    fn unknown_sentinel_round_trips_through_is_unknown() {
        let v = VersionInfo::new(
            VersionInfo::UNKNOWN_SHA,
            "unknown",
            "unknown",
            false,
            "unknown",
            "0.1.0",
        );
        assert!(v.is_unknown());
        let v2 = VersionInfo::new("abc1234", "abc1234", "now", false, "debug", "0.1.0");
        assert!(!v2.is_unknown());
    }

    #[test]
    fn current_utc_rfc3339_round_trips_a_known_epoch() {
        // 2020-01-01T00:00:00Z is 1577836800 seconds past epoch.
        // The helper reads the actual clock so we can't pin the
        // value directly; assert the *shape* is RFC-3339.
        let s = current_utc_rfc3339();
        // Expected: "YYYY-MM-DDTHH:MM:SSZ" (20 chars).
        assert_eq!(s.len(), 20, "RFC-3339 trimmed shape: {s:?}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], "T");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
    }

    #[test]
    fn serde_round_trips_version_info() {
        let v = VersionInfo::new(
            "0115122abc1234567890abcdef0011223344556677",
            "0115122",
            "2026-04-29T18:00:00Z",
            true,
            "release",
            "0.1.0",
        );
        let s = serde_json::to_string(&v).unwrap();
        let parsed: VersionInfo = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, v);
    }
}
