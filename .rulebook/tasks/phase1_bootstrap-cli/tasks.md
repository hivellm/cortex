## 1. Crate scaffold
- [ ] 1.1 `cortex-bootstrap` binary crate with Clap CLI (args per spec 09 §CLI)
- [ ] 1.2 Per-repo `cortex.toml` schema (exclude paths / extensions, chunking strategy, redaction overrides, git options, decision / law / memory promotion patterns)
- [ ] 1.3 Global defaults applied when `cortex.toml` is missing

## 2. Walkers
- [ ] 2.1 File walker using `ignore` crate (respects `.gitignore` + per-repo excludes; drops >10 MB files with counter)
- [ ] 2.2 Git log walker (`git log --all --diff-filter=AMD --name-only --format`); one event per commit; PR enrichment via `gh api` when authed
- [ ] 2.3 ADR / OpenSpec recognizer → `decision.imported`
- [ ] 2.4 Law recognizer → `law.imported`; memory recognizer → `memory.imported`

## 3. Synthetic event emission
- [ ] 3.1 Envelope assembler populating `adapter="bootstrap"`, stream header = `cortex.events.bootstrap`
- [ ] 3.2 Emission rules per source type (code symbol-level via chunker prefilter; doc section-level; turn per commit)
- [ ] 3.3 Redaction pass via `cortex-core::redact` (repo-specific extras merged)
- [ ] 3.4 Content-hash computed over the redacted payload

## 4. Checkpoint + resume
- [ ] 4.1 `.cortex-bootstrap.state.json` with per-repo progress (files walked, commits walked, status, last_file, last_git_ref)
- [ ] 4.2 Atomic write-rename every 5 s; fail-fast on write error
- [ ] 4.3 `--resume` reads checkpoint and continues from last progress

## 5. Operator ergonomics
- [ ] 5.1 `--dry-run --estimate` prints sizing block (files, chunks, bytes, Haiku cost estimate, embedding storage, graph nodes, Meili index size, one-time runtime)
- [ ] 5.2 Inclusion / exclusion / `--since <git-ref>` / `--parallelism N` flags per spec 09 §CLI
- [ ] 5.3 Stderr progress bar (TTY) + JSON logs (`--log-format json`)

## 6. Publisher
- [ ] 6.1 HTTP client to `cortex-core /v1/events/batch` with batching + retry
- [ ] 6.2 Graceful Ctrl-C: flush in-flight batch, write final checkpoint
- [ ] 6.3 Parallel repo walkers with bounded concurrency

## 7. Observability
- [ ] 7.1 Counters + histograms per spec 09 §Progress & telemetry
- [ ] 7.2 Per-repo summary printed on completion

## 8. Tail (mandatory)
- [ ] 8.1 Update `docs/specs/09-bootstrap-cli.md` status flag to 🟢 + index row
- [ ] 8.2 Integration tests: estimate mode on Vectorizer repo; end-to-end Vectorizer bootstrap (events observable in all four backends); idempotent replay (near-zero new writes); inclusion + exclusion + `--since` filters; SIGINT + resume correctness; ADR + law + memory promotion; parallel walk zero event loss; Tree-sitter-missing language routed through fallback; redaction leak test on synthetic `.env`
- [ ] 8.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
