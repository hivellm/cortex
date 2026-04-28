# Proposal: phase8c_version_coherence

## Why

The 2026-04-28 incident's hardest moment was diagnosing why the
`\n\n{asst}` Stop-turn fix didn't take effect: the source had the fix,
but the running `cortex-api.exe` had been built before the commit.
There was no way to ask the running daemon "what git SHA were you built
from?" — we had to compare binary mtime against `git log` to deduce it.

This is a recurring footgun on Cortex's multi-binary stack. After every
`cargo build`, you have to manually kill and restart each affected
binary; if you forget, the production process keeps running stale code
and silent regressions reappear. There's no system-level guard.

Embedding `git_sha` and `build_ts` into every binary closes the loop:
`/healthz` reports them, `cortex doctor versions` diffs the running
SHAs against the workspace HEAD, and CI can refuse to merge if the
embedded SHA doesn't match the commit being merged.

## What Changes

1. NEW workspace `build.rs` script (or shared `cortex-build` crate)
   that emits at compile time:
   - `CORTEX_GIT_SHA` (full + short)
   - `CORTEX_BUILD_TS` (ISO-8601)
   - `CORTEX_GIT_DIRTY` (bool — were there uncommitted changes?)
   - `CORTEX_BUILD_PROFILE` (debug/release)

   Each crate's `lib.rs` (or `main.rs`) re-exports these as
   `pub const VERSION_INFO`.

2. Every `/healthz` endpoint (phase8a) MUST include `extras.version`:
   ```json
   { "git_sha": "0115122abc...", "git_sha_short": "0115122",
     "build_ts": "2026-04-28T16:34:10Z", "git_dirty": false,
     "profile": "release" }
   ```

3. NEW `cortex-api /v1/health/versions` aggregator returns every
   running binary's version block plus the workspace's current
   `git rev-parse HEAD`. Computes:
   - `running_shas: HashSet<String>` (should be size 1 ideally)
   - `head_sha: String`
   - `drift: Vec<{ binary, running_sha, expected_sha, behind_by_commits }>`

4. NEW `scripts/doctor-versions.bat` calls the endpoint and prints a
   table; exit code 1 if any binary is behind workspace HEAD.

5. CI gate: `.github/workflows/version-coherence.yml` rejects PRs
   where `target/release/*.exe` mtimes are older than the touched
   `crates/<x>/src/` files (catches "forgot to rebuild before
   committing").

## Impact

- Affected specs: NEW `specs/version_coherence/spec.md`.
- Affected code:
  - NEW `crates/cortex-build/` (or root `build.rs` shared via path dep)
  - `crates/*/build.rs` (one-line include of cortex-build)
  - Every `/healthz` handler updated to include version block
  - NEW `crates/cortex-api/src/health/versions.rs`
  - NEW `scripts/doctor-versions.bat`
  - NEW `.github/workflows/version-coherence.yml`
- Breaking change: NO (additive).
- User benefit: the "running binary != source" footgun becomes
  impossible to miss — the GUI's Health view goes red, CI fails,
  `scripts/doctor-versions.bat` prints the offender.
