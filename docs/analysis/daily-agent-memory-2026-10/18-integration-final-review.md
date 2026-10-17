# Integration final review — §1.12

Reviewed on 2026-10-16. Scope: daily pre-thinking transport and scope derivation,
adapter dispatcher/publisher/startup and packaged lifecycle hooks, MCP daily
capture/read/client attribution/error paths, and launcher process ownership and
failure cleanup. This is source review by the integration agent, not an external
model review. The reviewer authored earlier launcher/probe work; those artifacts
are not independent evidence of their own correctness. No Cargo or provider/model
calls were made for this review. The root agent owns Rust quality gates and
independent model reviews.

## P1 — Basename fallback aliases unrelated daily-memory repositories

Before correction, `repo_from_cwd` in
`crates/cortex-pre-thinking/src/scope.rs:140` used the nearest Git root's basename
when no valid `cortex.id` existed. `read_memory` in
`crates/cortex-pre-thinking/src/pipeline.rs:563` passed that fallback to the daily
API as an exact repository identity. Two unrelated roots named `app` consequently
read the same accepted/pinned `app` corpus. Server-side exact scope checks cannot
distinguish these callers because both requests already claim the same identity.

Trigger: a shared local API containing an applicable accepted memory under `app`,
with two unrelated Git checkouts whose directory name is `app` and neither has
an explicit valid `[cortex] id` at its nearest Git root. A prompt or SessionStart
in the second checkout can inject memory belonging to the first.

### Concrete pre-fix reproduction

Used the existing `target/debug/cortex-mcp-server.exe` with an isolated loopback
HTTP stub and public synthetic text. No real Cortex memories, external models,
credentials or provider endpoints were used. The stub supplied a valid prepared
response representing an accepted pinned memory in repository `app`; optional
search returned HTTP 503. The two fixture roots were `a/app` and `b/app`, each
with `.git` and no `cortex.toml`.

Both MCP `cortex_pre_thinking` calls posted `repo="app"` to
`/v1/memory/query`. Both returned the exact same `public-pinned-app` context and
`memory_outcome="prepared"`; observed call latencies were 11 ms and 4 ms. This
reproduces the client-side identity collision, not a production database or
native-model consumption claim. Evidence:

`C:\Users\Bolado\AppData\Local\Temp\cortex-basename-repro-9bl65a0s\reproduction.json`

Pre-fix SHA-256:

| Source | SHA-256 |
| --- | --- |
| `scope.rs` | `3742C6C959A58A9CAA6BBCF9E866F5A9BCB743860972EAB3A07BBDF2A4DFED1E` |
| `pipeline.rs` | `6506317DDF0BB2DA0F3694E54AB155A923ECB694AD8990A1B8EFEEAEA981C606` |

### Authorized correction

`DerivedScope.memory_repo` now carries only a valid explicit `cortex.id` from
the nearest Git root. Daily lookup refuses basename-only identity with
`memory_outcome="scope_identity_required"` before invoking its memory transport.
The adapter maps that outcome to the existing once-per-session unavailable
notice; MCP exposes the precise outcome. Legacy optional full-stack search and
`repo_from_cwd` retain their basename contract. No new repository identity is
invented, no existing records are migrated, and explicit `cortex_memory_read`
continues to use its caller-supplied exact identity subject to API authorization.

Nearest-root discovery matters: a nested Git repository cannot inherit an outer
repository's explicit identity. Malformed, padded, oversized or non-string IDs
do not authorize daily lookup. An explicit valid ID remains an operator-owned
scope: unrelated repositories must receive distinct IDs. This checkout already
has `cortex.toml` with `id = "Cortex"`.

Regression coverage added:

- `same_basename_roots_require_explicit_identity_before_reading_daily_memory`:
  both roots initially skip the memory reader; optional basename scope remains
  compatible; an explicit `app` ID enables only the configured root, including
  the SessionStart memory-only path.
- `daily_identity_comes_only_from_valid_configuration_at_nearest_git_root`:
  nested-root separation and invalid configuration cases.
- `missing_explicit_repository_identity_surfaces_memory_warning_on_session_start`:
  the adapter exposes its existing unavailable notice for this refusal.

`docs/operations/daily-memory-runtime.md` documents the explicit-ID requirement.
Standalone rustfmt and targeted `git diff --check` passed. Rust compilation,
Clippy and test execution remain with the root's coordinated gates; these tests
are not claimed as executed here.

Basename-correction SHA-256 before root quality gates and subsequent CWD hardening:

| Source | SHA-256 |
| --- | --- |
| `scope.rs` | `91E8991253D834A02C93370C855AA44217F1167F3924B81A367B7B426241132B` |
| `pipeline.rs` | `80C96EBB93B7438CD42C7583EF8C4FF86AF84CE41972664B87572EF2AF17199B` |

## P1 — Unknown hook origin inherits the daemon's repository

The root's Astra cross-review identified this separate case. Integration review
confirmed that `SyncClient::context` previously used `cwd.unwrap_or(".")` in
`crates/cortex-adapter-claude-code/src/sync_paths.rs:271`. When the adapter daemon
ran from a repository with a valid explicit `cortex.id`, a frame without `cwd`
could consequently look up that repository's daily memories even though the
client's origin was unknown. Explicit-ID enforcement alone cannot prevent this:
the selected ID is valid but belongs to the daemon, not the unknown source.
An empty CWD could similarly affect the parallel SessionStart task resolver.

Root follow-up review also identified `cwd="."` and other relative paths as the
same ambiguity: a supplied path still resolves under the daemon if it is not
absolute. The authorized correction requires an absolute observed hook CWD
before daily or optional context lookup. Missing, empty, whitespace-only and
relative CWD returns an empty bundle with
`fail_open=true` and `memory_outcome="scope_unresolved"`; the dispatcher maps
that outcome to its existing deduplicated unavailable notice. `task_repo` also
requires an absolute origin before filesystem work or active-work lookup. Explicit
absolute CWD preserves the existing flow. Optional context is skipped for this
unknown-origin case as well: searching under daemon CWD would carry the same
unproven repository assumption.

Regression
`unknown_hook_cwd_never_uses_daemon_repository_for_memory_or_tasks` covers missing,
empty, whitespace-only, `.`, `relative/project` and Windows drive-relative
`C:relative\project` through direct sync results and both SessionStart and
UserPromptSubmit frames. It asserts the unresolved outcome, no context, an
operator notice and zero HTTP requests, including memory, optional search and
active-work. A prepared public synthetic daemon-only memory response is mounted
with an expectation of zero reads. No real stored memory or provider is used.
Source rustfmt and targeted diff-check passed. Rust execution remains owned by
the root; no Cargo command was run by this agent.

Shared pre-thinking scope derivation also withholds `memory_repo` for relative
CWD, even when it resolves a valid explicit configuration. This closes daily
lookup through MCP `cortex_pre_thinking` while preserving that tool's legacy
optional-search scope. Regression
`relative_cwd_cannot_authorize_daily_memory_even_with_explicit_id` creates a real
valid config reachable by both relative and absolute paths without changing
process CWD. It asserts no daily read for the relative path, preserves the
optional explicit scope and verifies absolute derivation remains authorized.

Current correction points:
`crates/cortex-adapter-claude-code/src/sync_paths.rs:223`,
`crates/cortex-adapter-claude-code/src/dispatcher.rs:124` and `:329`, and
`crates/cortex-adapter-claude-code/tests/dispatcher.rs:102`.

### Full-gate fixture correction — 2026-10-17

Root's `%TEMP%/cortex-phase34-step112-final-v2-test.log` records a failure in
`prethink_health_live_it::healthz_latency_p95_stays_under_200ms_under_normal_session_load`:
the latency sample count was 0 rather than 50. The load fixture still passed
`cwd="/repo"`. On Windows this is a drive-root-relative path, not an absolute
observed origin, so the new guard correctly returned before pipeline metrics.
This was a stale test fixture, not a reason to weaken the origin guard or record
fake pipeline latency samples for rejected calls.

The load test now creates a real temporary Git repository with an explicit
`cortex.id = "Latency/Repo"`, passes its absolute path and asserts all 50 calls
reach mocked optional HTTP retrieval. Existing assertions remain unchanged:
exactly 50 latency samples and p95 below 200 ms. The upstream remains hermetic;
no real repository memory or provider is used. Standalone rustfmt and targeted
diff-check passed; Cargo execution remains with root's coordinated gates.

Follow-up fixture audit found two unit tests in `sync_paths.rs` that still used
Windows-incomplete `/repo` or `/r` paths. Both now use an absolute temporary Git
root with an explicit ID. The unreachable-endpoint test preserves its connection
failure scenario. The first full run showed Windows can exhaust the unchanged
50 ms budget before its closed-port connect reports refusal. The test therefore
accepts transport outcomes `unavailable` or `timeout`, while rejecting the
scope-guard outcomes; optional retrieval need not run after deadline exhaustion.
The cache-failure test mocks HTTP 503 and requires two
requests to each of the memory and optional-query routes across repeated calls,
empty bundles and zero cache hits/misses. Their 50 ms timeouts are unchanged.
HTTP-listener and dispatcher-fuzz fixtures also use real absolute temporary
roots so valid frame branches reach context processing on Windows. Production
guards and metrics are unchanged. These edits passed rustfmt/diff-check; root
owns subsequent test execution.

The final candidate run also exposed an existing dashboard overview test that
assumed the adapter health port was closed. The now-active installed adapter
returned its real assembly p95 (809 ms), conflicting with the fixture's expected
zero. Overview response construction now accepts an adapter health snapshot;
the production handler still gathers real subsystem health, while unit tests
inject absent or synthetic health without network calls or global environment
changes. The zero assertion remains exact. A separate regression injects
assembly p95 809 and a different probe latency to verify the correct metric is
serialized. Runtime services remain active; root owns Rust execution.

## Other reviewed paths and limits

No further confirmed P0–P2 finding beyond those above. In particular:

- The pipeline's local lookup precedes optional retrieval/breaker handling,
  reserves wrapper bytes before preparing context, rejects over-budget or
  cross-scope response metadata, and never clips the cited memory section after
  receipt preparation. There is no stale client-side daily bundle cache.
- HTTP daily reads bound response bodies to 16 KiB; MCP daily responses count
  both raw response and escaped tool-result size. MCP capture defaults external
  classification to false and records observed MCP client names as attribution,
  not authentication.
- Daily adapter startup selects the disabled publisher before opening or
  replaying the archival WAL. Enabled publishing releases queue lock before HTTP
  send/retry while serializing batch flushes. SessionStart without resolved
  task scope skips active-work lookup rather than requesting an unfiltered list.
- Packaged SessionStart/UserPromptSubmit scripts invoke the native transport
  synchronously. Native transport owns bounded stdin/frame/reply I/O, actual
  payload session identity and the shared outage-notice policy.
- Launcher source verifies PID, executable path and creation time before cleanup,
  checks listener ownership after health confirmation, and restores inherited
  environment. This supplemental inspection is not independent review of the
  reviewer's earlier launcher implementation.

This review does not replace storage/API authorization review, native harness
acceptance, independent model review, or the final full quality gates. Earlier
public synthetic probes and real-model acceptance are recorded separately in
analysis11–15; they are not counted as independent source-review evidence here.
