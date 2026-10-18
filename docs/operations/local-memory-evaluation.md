# Local memory evaluation

The standalone evaluation separates storage/provenance correctness, fixed-corpus retrieval quality, latency, installed-surface checks and optional provider quality. A green **contract/non-regression gate does not mean natural-language readiness**. The current held-out natural-language baseline is zero, and the separate readiness result is false.

All normal evaluation inputs are public fictional fixtures. Runners create temporary homes and never import into the operator's real store. Historical events in this corpus are invented examples; they are not the real historical archive verified during [phase36 acceptance](../analysis/local-first-core-2026-10/05-implementation-evidence.md).

## Run the offline gate

Use Python 3.13 and the repository Rust toolchain. Build/dependency acquisition may need network access; the evaluation itself requires no Docker, application HTTP service, provider key or external model. Classification contract tests start an ephemeral loopback HTTP fixture.

From the repository root, using the default Cargo target directory:

```powershell
cargo build --release --locked -p cortex-local --bin cortex
$env:CORTEX_EVAL_BINARY = (Resolve-Path target/release/cortex.exe).Path
python -W error::ResourceWarning -m unittest discover -s scripts/tests -p "test_*eval*.py"
cargo test --locked -p cortex-local --test classification_eval
python scripts/gate_local_memory.py --binary "$env:CORTEX_EVAL_BINARY" --report scripts/eval-report.json
```

POSIX shell equivalent:

```bash
cargo build --release --locked -p cortex-local --bin cortex
export CORTEX_EVAL_BINARY="$PWD/target/release/cortex"
python -W error::ResourceWarning -m unittest discover -s scripts/tests -p 'test_*eval*.py'
cargo test --locked -p cortex-local --test classification_eval
python scripts/gate_local_memory.py --binary "$CORTEX_EVAL_BINARY" --report scripts/eval-report.json
```

If `CARGO_TARGET_DIR` is set, substitute its release path consistently. `CORTEX_EVAL_BINARY` enables real-binary Python integration tests; omitting it skips those optional tests and does not constitute full integration validation. Some Windows hosts cannot create symbolic links without additional privilege; the relevant test explicitly reports that limitation.

The gate uses [local-eval-v1.json](../../crates/cortex-local/baselines/local-eval-v1.json), builds fresh isolated stores, runs CLI/MCP quality and integrity checks, then measures twenty sequential samples in each of five latency series. Exit zero means this fixed contract and non-regression policy passed. Inspect the report's separate `natural_language_ready` assessment before drawing a usefulness conclusion. Failed reports retain diagnostics and samples; do not delete failures or change judgments to obtain green output.

The script also accepts an explicit pair of `--quality-report` and `--latency-report` inputs for historical assessment. Such a result assesses supplied claims rather than performing fresh execution. CI uses neither flag. Policy changes require review and rationale; no successful artifact automatically becomes the next baseline.

## What the corpus measures

[eval-v1](../../crates/cortex-local/tests/fixtures/eval-v1/README.md) contains eighteen memories, eight archived turn/tool events, four graph records, twenty retrieval queries and three graph questions under CC0-1.0. Exact repositories `Atlas`, `atlas` and `AtlasLab` deliberately overlap in text. The manifest pins source bytes, original/imported identities, source hashes and expected evidence. Committed compressed archives avoid a runtime compression dependency.

Queries and relevance judgments were frozen before retrieval execution. Twelve queries are development and eight held-out; held-out means excluded from tuning, not independently sourced external validation. Corpus changes belong in a reviewed new version, retaining prior results. The frozen identities are:

| Artifact | SHA-256 |
| --- | --- |
| Corpus | `dd4c38d60a66f36004aa440a68279adb1043dd72c0723caeef13b510c0ad1521` |
| Manifest | `a661af9be17b6148a19b65fd5568f4787c6b3681d24e2caa3c08005a47b90734` |
| Gate policy | `7a82c121075cf0f0c2a6be1ebc7115044fc77c80055e76af7e563efb6a65dcc5` |

Construction imports through production CLI, repeats imports and reindexes two independent temporary homes. Sources must remain byte-identical. Complete canonical records must remain unchanged within each home; cross-home comparison excludes generated timestamps, record hashes and validated absolute locator prefixes. Full source/metadata and readable archive body projections are checked, so recomputing a hash cannot legitimize injected foreign content.

Scope checks alternate repositories in one persistent MCP process and verify identities, recall text, statistics and ranking stability after foreign insertions. Graph checks compare exact nodes and directed typed edges, including reverse traversal, bounds, truncation, inactive/missing roots and dangling links. This is bounded bidirectional BFS, not a shortest-path API. CLI graph limits are depth 0..5 and 1..200 nodes; MCP permits 1..100 nodes.

## Observed retrieval quality

The baseline below was measured with accepted standalone release SHA-256 `ce68013c14018cdd1df45878f29a2e5750590d6aefca1c33b82f68f023fedb8f`. Future candidates may have a different hash; quality and latency evidence must identify the same candidate executable.

| Split | Answerable queries | Empty expectations | Recall@5 | MRR@10 |
| --- | ---: | ---: | ---: | ---: |
| Development | 10 | 2 | 0.8625 | 1.0000 |
| Held-out | 7 | 1 | 0.0000 | 0.0000 |
| Combined | 17 | 3 | 0.5073529412 | 0.5882352941 |

All three empty expectations returned no hits. All seven answerable natural-language held-out questions also returned no hits: this is a measured limitation of literal AND retrieval, not successful semantic recall.

Recall@5 divides unique valid relevant hits in the first five positions by **all** expected relevant identities. A query with eight expected records can therefore score at most 5/8. MRR@10 averages the reciprocal position of the first valid relevant hit among the first ten results. Duplicate or invalid positions do not move later results forward. Empty expectations use separate success counts and have null recall/MRR.

The gate blocks loss on each frozen query and aggregate, plus integrity, scope, graph, provenance and empty-result failures. Existing zero-credit held-out queries cannot reveal further recall loss. Separate natural-language targets require held-out Recall@5 and MRR@10 of at least 0.5; both are currently unmet. Passing this gate does not establish those targets, broad productivity gains or model consumption. The two cited memories observed in phase36 agent sessions remain a distinct, narrower result.

## Latency baseline and limits

Frozen baseline: Windows 11 build 26200, AMD Ryzen 9 7950X3D, 32 logical CPUs, Python 3.13.9, twenty samples per series, concurrency one. Values below come from the versioned policy rather than a later incidental run.

| Series | Observed p50 (ms) | Observed p95 (ms) | Blocking p95 ceiling (ms) |
| --- | ---: | ---: | ---: |
| CLI, new process, warmed store | 26.58 | 34.09 | 2000 |
| MCP, new process and first request | 24.89 | 26.12 | 2000 |
| MCP, persistent after warmup | 3.85 | 4.47 | 500 |
| SessionStart, direct hook process | 29.58 | 38.64 | 2000 |
| UserPromptSubmit, direct hook process | 53.16 | 55.02 | 2000 |

These are evaluator-observed wall times including transport, process polling, JSON parsing and semantic validation. New-process MCP includes spawn and initialization; cleanup is outside timing. "Cold" means a fresh process only: OS/filesystem caches are not evicted. Hooks are direct executable calls, excluding launcher/client scheduling. UserPromptSubmit includes publication of a fictional event; SessionStart is read-only. Prepared context is not proof of model use.

P50/p95 use nearest rank over successful samples, with every attempted, failed and unmeasured sample reported. Acceptance requires all planned samples, valid hook context, unchanged original records/sources, and no setup/cleanup failure. The two-second/500-millisecond ceilings are broad small-corpus responsiveness guards for heterogeneous CI. They do not detect modest slowdowns relative to this workstation baseline, establish production SLOs or measure a large historical store.

For latency diagnostics alone:

```bash
python scripts/evaluate_local_latency.py --binary target/release/cortex --report scripts/latency-report.json --samples 20
```

Use `.exe` on Windows. Lower diagnostic sample counts do not satisfy the gate's minimum.

## Check an installed executable

The installed smoke compares an installed binary with an explicit reference build. Matching hashes/build-info are required; this command does not install or update either executable.

```powershell
python scripts/smoke_installed_local.py --installed-binary "$env:USERPROFILE/.cortex/bin/cortex.exe" --reference-binary target/release/cortex.exe --report scripts/installed-smoke-report.json
```

```bash
python scripts/smoke_installed_local.py --installed-binary "$HOME/.cortex/bin/cortex" --reference-binary target/release/cortex --report scripts/installed-smoke-report.json
```

Setup creates only a new fictional temporary home. The current smoke compares fifteen advertised/implemented/documented tools and exercises eleven source-preserving tools, plus explicit retention preview without apply. Recall and SessionStart may publish local audit receipts. Original memory records/segments, queue JSON and source bytes must remain identical; audit publication is separate from those sources. Derived SQLite/lock bytes are excluded because readers may modify them. A reference build and installed executable must match before interpreting results.

The historical phase28 CE680 acceptance exercised eleven tools, eight read-only operations and fourteen passing checks before audit receipts were added. Its SessionStart path did not write receipts. Those results remain evidence for that binary, not a claim that an older installed executable already provides phase31 tools or that an active client loaded a new plugin.

Phase31 final acceptance passes on installed release `3959770b`. The elevated
full workspace all-feature run exits 0 across 263 targets: 4,280 tests pass,
none fail and nine pre-existing checks/examples are ignored (180 standalone,
4,100 legacy). It uses `--test-threads=1`; all tests and internal concurrency
exercises remain enabled, but this does not prove default parallel stability.
Formatting, type-check, strict clippy and Ruff pass. Python runs 162 cases with
real CLI/MCP integration and strict ResourceWarning handling: 161 pass and one
Windows symlink-privilege case is skipped. The parser and aggregate final
acceptance report pass.

Desktop 1440x1000 and mobile 390x844 GUI acceptance verifies captured-citation
labels, exact revision 1 after revision 2 exists, literal imported markup without
injected DOM, and no horizontal overflow. The temporary server is stopped, port
16464 closed and token invalidated; no test-target or Cargo process remains.
Installed smoke passes fourteen checks over fifteen tools in an isolated home.
Runtime acceptance preserves four canonical files through restart/reindex, with
an actual CLI follow-up captured through the direct adapter remaining unknown
without native attribution. Historical CLI/MCP checks preserve the May 5/May 9
events and matching graph results. Plugin caches contain 0.4.1; active clients
still need restarting, and fixture acceptance is not proof of client reload.

Preservation verifies all 1,657 canonical files and 69,634 archive files
(319,238,316 bytes) with unchanged hashes. Evidence has been moved to
`C:/Users/Bolado/.cortex/backups/phase31-audit-3959770b/evidence/`: 198 files,
51 directories and 6,902,784 bytes, verified by before/after manifests with
matching sizes and hashes. Failed runs and temporary acceptance helpers remain
in that backup alongside successful reports.

Earlier attempts remain failures: missing plugin documentation/version checks,
legacy Bash timeout, a Windows SQLite sidecar snapshot race, retention-canary
orphan, sandbox token ACL denial and cache HTTP 503 responses. Test-only
corrections preserve source assertions and production deadlines. Isolated
reruns did not establish causes for the intermittent retention/cache failures;
the passing serial gate is not evidence that default parallel timing is fixed.
The task design preserves detailed counts, corrections and review evidence.

Frozen corpus, judgments and policy remain unchanged. Seven answerable held-out
natural-language questions still return zero and `natural_language_ready=false`.
Current real-home inspection is also limited: exact repository `Cortex` contains
26 records/revisions (16 events, ten memories, one link), but audit matching_records
is zero, without prepared-delivery or captured-citation evidence. The whole-home
106,960 records across 37 repositories is a separate inventory. Neither that
inventory nor fixture/direct-adapter acceptance establishes actual-client
consumption or causal productivity from the new audit feature.

## Classification: offline versus live

[classification-eval-v1](../../crates/cortex-local/tests/fixtures/classification-eval-v1/README.md) has six separate fictional category judgments. Production typed decoding and queue behavior run against synthetic loopback responses, including deliberate mistakes, unknown outcomes, malformed responses, HTTP 429 and timeout. Mocked correctness is 4/6 with two unknown answers. This checks scoring, probability validation, accounting and source-authority preservation; it is **not measured Jev accuracy**. Synthetic token counts are not billed usage.

Live evaluation is separate and defaults to preview:

```bash
python scripts/evaluate_jev_live.py --binary target/release/cortex --report scripts/jev-preview.json
```

The preview records exact requests and `payload_sha256`. It sends no provider request. Use `.exe` on Windows. Review the payload, destination, pricing assumptions and budget; obtain separate authorization before any live call. Ordinary offline evaluation does not authorize outbound classification.

Only after that review and authorization, with the credential supplied securely in `TYPESAFE_API_KEY`, the explicit invocation is:

```bash
python scripts/evaluate_jev_live.py --binary target/release/cortex --report scripts/jev-live.json --live --reviewed-payload-sha256 REVIEWED_HASH --budget-usd 0.02
```

`REVIEWED_HASH` must match the previewed request set. The example budget is a chosen limit, not authorization or a charge estimate. The runner pins `jev-1.13.0`, allows at most six cases and one attempt per case, uses a ten-second provider timeout, and checks a conservative reservation before sending. Pricing assumptions reviewed on 2026-10-18 reserve 65,536 input tokens per call at USD 0.042/million, with free output: USD 0.002752512 per call, or USD 0.016515072 for six. Assumptions expire after thirty days and require review. This guard is not a provider-native billing cap; missing usage retains the reservation, and timeout does not prove zero cost.

No new live Jev quality request was executed for this evaluation phase. The earlier phase36 live integration example remains historical evidence, not classification-accuracy validation.

## CI and evidence boundaries

[eval.yml](../../.github/workflows/eval.yml) runs on push, pull request and manual dispatch for Ubuntu and Windows. It builds the release, tests offline classification, enables real-binary Python tests and runs fresh evaluation. Reports upload with `always()` for ninety days, including gate failures. Build failures may produce no report and remain visible in job logs. GitHub-hosted success is not claimed before actual runs.

The [phase28 design record](../../.rulebook/archive/2026-10-18-phase28_retrieval-eval-gate-live/design.md) preserves checkpoint results, failed-run lessons and enforcement rationale. Final phase28 validation passes 131 unique standalone and 4,100 legacy Rust tests, with nine pre-existing ignored live checks/examples. Python runs 148 cases: 147 pass and one Windows symlink privilege check is skipped. Formatting, workspace all-target/all-feature check, strict clippy and Python lint/format pass; fresh evaluation and fourteen installed-smoke checks also pass. Earlier Windows sandbox ACL failures and the Python cleanup failure remain recorded alongside the successful reruns.

Earlier phase28 service-stack evidence remains in commit `2ba3d2f502f3f3955166ac272ae5d8b28c2aea05`: enabling the reranker increased p95 from 102 to 2,224 ms (a +2,122 ms delta), the phantom-link rate was 0.50, and the nightly workflow omitted retrieval. These were unresolved legacy findings, not a passing standalone baseline. See the [task proposal](../../.rulebook/archive/2026-10-18-phase28_retrieval-eval-gate-live/proposal.md) and [backlog review](../analysis/local-first-core-2026-10/02-backlog-review.md) for the change in scope.

Machine-specific reports and logs are preserved in the local evidence backup `~/.cortex/backups/phase28-evaluation-ce68013c/`, with batch manifests verifying unchanged SHA-256 values. Failed reports remain alongside successful reruns. Versioned public fixtures and policy stay in the repository.
