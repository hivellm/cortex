# Spec: Version coherence

## ADDED Requirements

### Requirement: build-time version emission

Every Cortex binary MUST embed at compile time the following environment
variables via `cargo:rustc-env`:
- `CORTEX_GIT_SHA` — full 40-char SHA of the workspace HEAD when built
- `CORTEX_GIT_SHA_SHORT` — first 7 chars of the SHA
- `CORTEX_BUILD_TS` — RFC-3339 build timestamp (UTC)
- `CORTEX_GIT_DIRTY` — `"true"` if `git status --porcelain` had any output, else `"false"`
- `CORTEX_BUILD_PROFILE` — `"debug"` or `"release"`

Builds outside a git working tree (e.g. published crates.io tarballs) MUST
fall back to `"unknown"` for git fields without failing the build.

#### Scenario: dirty build is flagged
Given the workspace has uncommitted changes
When `cargo build --release -p cortex-api` runs
Then the resulting binary's `version_info().git_dirty` MUST equal `true`.

### Requirement: /healthz reports version

Every `/healthz` endpoint (phase8a) MUST include an `extras.version`
object containing all five fields from the build-time emission.

#### Scenario: stale binary reveals itself
Given a binary was built from `git_sha = abc1234`
And the workspace HEAD is now at `def5678`
When a client calls `GET /healthz` on the binary
Then the response's `extras.version.git_sha` MUST equal `abc1234`.

### Requirement: cortex-api version drift aggregator

`GET /v1/health/versions` on cortex-api MUST return a payload of shape:
```
{
  "head_sha": "<workspace HEAD>",
  "head_sha_short": "<7-char>",
  "running_binaries": [
    { "name": "cortex-api", "git_sha": "...", "build_ts": "...", "git_dirty": false }
  ],
  "drift": [
    { "binary": "cortex-api", "running_sha": "...", "expected_sha": "...", "behind_by_commits": 3 }
  ],
  "all_in_sync": false
}
```

`drift` MUST list only binaries whose `running_sha != head_sha`.
`all_in_sync` MUST be `true` iff `drift` is empty.
`behind_by_commits` is the count returned by
`git rev-list <running_sha>..HEAD --count`; if the running SHA is not
reachable (force-pushed branch), the value is `null` and a
`note: "running sha unreachable from HEAD"` field is included.

#### Scenario: aggregator detects drift
Given cortex-api is running from `running_sha = abc1234`
And cortex-adapter-claude is running from `running_sha = def5678`
And workspace HEAD is `def5678`
When `GET /v1/health/versions` is called
Then `drift` MUST contain exactly one entry where `binary = "cortex-api"`
     AND `all_in_sync` MUST be `false`.

### Requirement: CI gate refuses stale binaries

A GitHub Actions workflow named `version-coherence` MUST run on every PR
and MUST fail when any `target/release/<binary>.exe` mtime is older than
the most-recent mtime of `crates/<owning-crate>/src/**/*.rs`.

The error message MUST identify both the binary and the newer source
file so the contributor knows what to rebuild.

#### Scenario: PR rebuilds are enforced
Given a PR modifies `crates/cortex-api/src/dashboard.rs`
And `target/release/cortex-api.exe` was committed but its mtime is older
When the `version-coherence` workflow runs
Then the workflow MUST fail with a message naming both files
     AND the commit MUST NOT merge until the binary is rebuilt.
