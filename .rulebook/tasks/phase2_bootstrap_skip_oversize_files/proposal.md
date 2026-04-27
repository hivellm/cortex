# Proposal: phase2_bootstrap_skip_oversize_files

## Why

The 2026-04-27 bootstrap of the 17 Hive repos failed mid-run on `Tml`:

```
[bootstrap] Tml: failed: publish failed for docs/docs.json:
  synap publish: Server error:
  Failed to buffer the request body: length limit exceeded
```

`Tml/docs/docs.json` is a single ~12 MB JSON blob. The Synap publisher rejects bodies above its 10 MB limit (the embedder/fulltext workers have a `max_body_bytes: 10485760` config — same ceiling). When `cortex-bootstrap` hits a single oversized file, the entire repo run aborts and the next 14 repos still on the CLI line continue, but `Tml` is left half-indexed.

The walker today has size limits but they apply to a different bucket (line-count? extension?). Whatever the existing filter is, it does not stop a 12 MB JSON file from being shipped wholesale.

Two repos have been observed to have this problem (`Tml`, possibly `TmlDocs`'s vendored maps). Likely more in the wild as projects accumulate generated artifacts.

Source: 2026-04-27 bootstrap log captured during the reindex audit.

## What Changes

- `cortex-bootstrap`'s walker (`crates/cortex-bootstrap/src/walker.rs` plus `cortex.toml` config schema) gains a `max_file_bytes` ceiling. Default 8 MB (under the 10 MB Synap limit, with headroom for redaction-induced expansion). Override per-repo via `cortex.toml`'s `[<repo>.exclude]` block — e.g.
  ```toml
  [tml.exclude]
  max_file_bytes = 4194304   # 4 MB for repos with vendored blobs
  ```
- Files above the ceiling become `WalkEntry::Dropped { reason: "oversize_<bytes>" }` instead of `Accepted`. Metrics increment `cortex_bootstrap_files_dropped{reason="oversize"}`. Log line at `INFO`: `dropped oversize file <path> (<size> > <limit>)`.
- The publisher path stays at 10 MB for now — the walker is the right place to filter (decision lives with the source-of-truth that knows the file size already, and we avoid the round-trip cost).
- Repo run no longer aborts on one publish failure for a single envelope. The runner switches from `?` propagation to per-event error counters: continue past a single failure, increment `errors{reason}`, and only abort the repo when the failure rate exceeds 5% of attempted publishes (prevents masking systemic failures like Synap being down).

## Impact

- Affected specs: spec-09 (bootstrap walker — add `max_file_bytes` to the per-repo config schema).
- Affected code:
  - `crates/cortex-bootstrap/src/walker.rs` (apply ceiling)
  - `crates/cortex-bootstrap/src/config.rs` (parse `max_file_bytes` per repo)
  - `crates/cortex-bootstrap/src/runner.rs` (per-event error counters; replace `?` with bounded retry → drop)
  - `cortex-bootstrap.toml` example doc with the override
- Breaking change: NO — the new field is optional with a sane default.
- User benefit: bootstrap completes 17/17 repos in one run; oversize files are reported, not silently fatal; one bad blob in a repo no longer wastes the rest of the index work.
