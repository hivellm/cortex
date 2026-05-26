## 1. Crate scaffold
- [x] 1.1 `cortex-bootstrap` binary crate registered in `Cargo.toml` workspace members
- [x] 1.2 Clap CLI surface covering every option in spec 09 §CLI: positional `<REPO_ROOT>...` plus the include / exclude / `--since` / `--dry-run` / `--estimate` / `--resume` / `--parallelism` / `--synap-endpoint` / `--stream` / `--checkpoint` / `--log-format` / `--verbose` / `--config` flags
- [x] 1.3 Per-repo `cortex.toml` schema (exclude paths/extensions, chunking strategy, redaction overrides, git options, decision/law/memory promotion patterns) parsed via serde + `toml` workspace dep
- [x] 1.4 Global defaults applied when `cortex.toml` is missing (junk excludes, symbol-level code chunking, all commits, no PR enrichment, no extra redaction)

## 2. Walkers
- [x] 2.1 File walker using `ignore` crate (respects `.gitignore` + per-repo excludes; oversize files >10 MB dropped with the spec-09 oversize counter labelled `reason=oversize`)
- [x] 2.2 Git log walker (`git log --all --diff-filter=AMD --name-only --format='%H|%at|%ae|%s|%b'`); one event per commit; record framing via `\x1e` / `\x1f` so commit bodies with newlines still parse; merge commits emit a single record from the squash-style summary
- [x] 2.3 PR enrichment hook lives in the runner config (`include_prs`); the implementation degrades gracefully when `gh` is missing — wired up but currently inert until Phase-2 follow-up
- [x] 2.4 ADR / OpenSpec recognizer matches `cortex.decisions.promote_patterns` → `decision.imported`
- [x] 2.5 Law recognizer matches `cortex.laws.promote_patterns` → `law.imported`; memory recognizer matches `cortex.memories.import_files` → `memory.imported`

## 3. Synthetic event emission
- [x] 3.1 Envelope assembler producing spec-01-compliant events with `adapter="bootstrap"`, stream `cortex.events.bootstrap`, ULID `event_id`, ms-epoch `ts`
- [x] 3.2 `artifact.code` per accepted code file with `byte_range` + `git_ref` placeholders + language detection (downstream embedder handles symbol-level Tree-sitter splitting per spec 06)
- [x] 3.3 `artifact.doc` per `*.md` file with title derived from H1 or filename stem
- [x] 3.4 `turn.historical` per git commit with `evidence.{files_changed, diff_summary}` and timestamp pinned to the author's commit time
- [x] 3.5 `decision.imported` carries `title`/`status`/`supersedes`/`body`; `law.imported` carries `law_id`/`title`/`severity`/`detector`/`body`; `memory.imported` carries `title`/`body`
- [x] 3.6 Redaction pass via `cortex_core::redact` with the redactor's count surfaced on every event
- [x] 3.7 `content_hash = sha256(canonical_json(redacted_payload))` computed via `cortex_core::canonical_json::canonicalize`

## 4. Checkpoint + resume
- [x] 4.1 `.cortex-bootstrap.state.json` (configurable via `--checkpoint`) carries per-repo progress: `files_walked`/`files_total`, `commits_walked`/`commits_total`, `events_emitted`, `status`, `last_file`, `last_git_ref`
- [x] 4.2 Atomic write-rename in `checkpoint::write_atomic`; checkpoint write failure surfaces as a fatal `CheckpointError` (cannot guarantee resume correctness)
- [x] 4.3 `--resume` reads the checkpoint and the runner picks up after `last_file` / `last_git_ref`, never re-publishing already-emitted events

## 5. Operator ergonomics
- [x] 5.1 `--dry-run --estimate` prints the spec-09 sizing block (files after excludes, code chunks est, doc chunks est, commits, total events, redacted bytes, classifier in/out tokens, embedding storage, graph nodes + edges, fulltext index size, est runtime)
- [x] 5.2 Inclusion / exclusion / `--since <git-ref>` flags filter the repo set and commit window
- [x] 5.3 `--parallelism N` bounds concurrent repo walkers via a tokio semaphore (default 4)
- [x] 5.4 Stderr summary line per repo + JSON-line logs when `--log-format=json` (tracing-subscriber `json` feature wired in)

## 6. Publisher
- [x] 6.1 Synap publisher writing to `cortex.events.bootstrap` (overridable via `--stream`, `--synap-endpoint`); mirrors the `LiveSynapPublisher` shape from `cortex-fulltext`/`cortex-graph` for batch + retry parity
- [x] 6.2 Per-event retry with exponential backoff (3 attempts, 100/400/1600 ms); `MemoryPublisher` test double records every call for unit + integration coverage
- [x] 6.3 Graceful Ctrl-C: handler flips a shared `AtomicBool`, runner stops between repos, final checkpoint is written before exit

## 7. Observability
- [x] 7.1 Counters + histograms per spec 09 §Progress & telemetry (`bootstrap.files.walked`, the per-reason oversize / extension drop counters, `bootstrap.events.emitted{kind}`, `bootstrap.bytes.processed`, `bootstrap.commits.walked`, `bootstrap.repo.duration_s`, `bootstrap.errors{stage}`, `bootstrap.publish.latency_ms`)
- [x] 7.2 Per-repo summary printed on completion (events emitted, files dropped, wall-clock seconds)
- [x] 7.3 Per-batch tracing event with structured fields (`repo`, `events`, `files_dropped`, `commits_walked`, `duration_s`, `outcome`)

## 8. Tail (mandatory)
- [x] 8.1 Update or create documentation covering the implementation — `docs/specs/09-bootstrap-cli.md` flipped to 🟢 Implemented; `docs/specs/00-index.md` row updated to 🟢
- [x] 8.2 Write tests covering the new behavior — `tests/runner.rs` (11) covers `--estimate` no-write walk, end-to-end fixture emitting one event per recognised kind, idempotent replay via checkpoint resume, redaction leak test on a synthetic `.env` carrying an AWS-key-shaped secret, `--dry-run` publishes nothing, parallel walker over 4 fixture repos with no event loss, atomic checkpoint round-trip, oversize-file drop reasoning, `--since` propagation, glob-based promotion classification, and the format-estimate text block; lib unit tests (25) cover config parser, CLI surface, file classification + glob matcher, git-log parser, body-selection rule, doc-id rules, redaction count surface, and emitter shape per kind
- [x] 8.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy -p cortex-bootstrap --all-targets -- -D warnings`, `cargo test -p cortex-bootstrap` all green (36 tests)
