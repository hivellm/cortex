# 09 — Bootstrap CLI (`cortex-bootstrap`)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 04, 05, 06, 07, 08

## Goal

Ship a one-shot + incremental CLI that walks an existing Hive repo (or a list of repos), synthesizes events for its source code, docs, specs, git history, and rule/law files, and drives them through the live processing pipeline (`cortex.events.bootstrap` stream → classifier → embedder → graph writer → full-text indexer). This is what gives the very first `pre_change_context` query something to retrieve.

## Scope

**In:**
- `cortex-bootstrap` crate: Rust binary, Clap CLI.
- Per-repo configuration (`cortex.toml`) with sensible defaults.
- Source discovery: git metadata, file walker, doc parser, ADR recognizer.
- Synthetic event emitters (envelope matches spec 01; kind is one of the new `artifact.*`, `turn.historical`, `decision.imported`, `law.imported`, `memory.imported`).
- Publication to `cortex.events.bootstrap` Synap stream.
- Progress reporting (ETA, per-repo counters).
- Resume support via a checkpoint file.
- `--dry-run --estimate` mode (no writes; prints sizing).
- `--only <repo[,repo,...]>` / `--skip <repo[,...]>` filters.

**Out:**
- The processing pipeline itself (specs 05–08).
- Filesystem watcher / git-hook tailing (separate Phase-2 spec).
- Webhook receiver (Phase 2).
- CI-agent integration (Phase 3).

## Inputs / Outputs

### CLI

```
cortex-bootstrap [OPTIONS] <REPO_ROOT>...

OPTIONS:
  --config <file>           Global config override (default: ./cortex-bootstrap.toml)
  --only <name[,name]>      Include only listed repos
  --skip <name[,name]>      Exclude listed repos
  --since <git-ref>          Only re-index changes since this ref (incremental mode)
  --dry-run                  No writes; print plan
  --estimate                 Implies --dry-run; print sizing (files, chunks, bytes, est. cost)
  --resume                   Resume from the last checkpoint
  --parallelism <N>          Number of concurrent repo walkers (default: 4)
  --synap-endpoint <url>     Override Synap connection
  --stream <name>            Destination Synap stream (default: cortex.events.bootstrap)
  --checkpoint <file>        Checkpoint file path (default: .cortex-bootstrap.state.json)
  --log-format <json|text>   Structured logs
  --verbose                  Debug logging
```

### Per-repo configuration (`cortex.toml` inside a repo, optional)

```toml
[cortex]
id = "Vectorizer"                            # override repo name (default: basename)

[cortex.exclude]
paths = ["target/", "node_modules/", "dist/", "tmp/"]
extensions = ["lock", "log", "pack", "png", "jpg", "pdf"]

[cortex.chunking]
code_strategy = "symbol"                     # symbol | window | auto (default: auto)
doc_strategy = "section"                     # section | window (default: section)

[cortex.redaction]
extra_patterns = [
  { name = "internal_token", regex = "HIVE_TOKEN_[A-Z0-9]{24}" }
]

[cortex.git]
include_commits = true
include_prs = true                           # requires `gh` CLI and auth
since = "2019-01-01"                         # ignore ancient commits (default: all)

[cortex.decisions]
promote_patterns = [
  "docs/decisions/*.md",
  "openspec/**/proposal.md",
  "ADR-*.md"
]

[cortex.laws]
promote_patterns = [
  "rulebook/laws/*.yaml",
  ".cursor/rules/*.md"
]

[cortex.analyses]
promote_patterns = [
  "docs/analysis/**/*.md",
  "docs/analyses/**/*.md"
]

[cortex.memories]
import_files = ["CLAUDE.md", "AGENTS.md", ".rulebook/memory/**/*.md"]
```

Files matching `[cortex.analyses].promote_patterns` are emitted as
`analysis.imported` events with payload `{ title, status, body,
source_path }`. They route to a dedicated per-repo
`cortex-{repo}-analyses` Meili index + Vectorizer collection, and the
graph writer materialises them as `(:Analysis)-[:ANALYZES]->(:Repo)`.
This is the path audit / deep-analysis (spec 15) reports take to reach
the dashboard's Analysis surface.

If `cortex.toml` is missing, defaults apply:

- exclude common junk (`target/`, `node_modules/`, `.git/`, `dist/`, binary extensions)
- symbol chunking for code, section chunking for docs
- all git commits included, no PR enrichment
- no extra redaction patterns

### Synthetic event shape

Each source produces envelope-compliant events (spec 01). Examples:

**Code artifact (one event per top-level declaration after Tree-sitter split):**

```jsonc
{
  "event_id": "01HXY...", "ts": 1713369600000,
  "kind": "artifact.code",
  "adapter": "bootstrap",
  "source": { "repo": "Vectorizer", "path": "src/index/hnsw/mod.rs", "symbol": "hnsw_search",
              "byte_range": [120, 840], "git_ref": "abc123" },
  "redacted_payload": { "text": "pub fn hnsw_search(...) { ... }", "language": "rust" },
  "content_hash": "sha256:..."
}
```

**Historical turn (one event per git commit):**

```jsonc
{
  "event_id": "01HXZ...", "ts": 1710000000000,
  "kind": "turn.historical",
  "adapter": "bootstrap",
  "source": { "repo": "Vectorizer", "git_ref": "abc123", "author": "a@b.c" },
  "redacted_payload": {
    "role": "developer",
    "message": "fix: raise ef_search default for 1M-vector benchmarks",
    "evidence": { "files_changed": ["src/index/hnsw/mod.rs"], "diff_summary": "+40 / -12" }
  },
  "content_hash": "sha256:..."
}
```

**Decision (promoted from ADR / OpenSpec proposal):**

```jsonc
{
  "event_id": "01HY0...", "ts": 1712000000000,
  "kind": "decision.imported",
  "adapter": "bootstrap",
  "source": { "repo": "Vectorizer", "path": "docs/decisions/0042-hnsw-ef-default.md" },
  "redacted_payload": {
    "title": "Raise HNSW ef_search default to 128",
    "status": "accepted", "supersedes": null,
    "body": "...markdown content..."
  },
  "content_hash": "sha256:..."
}
```

**Law (imported from existing rule files):**

```jsonc
{
  "event_id": "01HY1...", "ts": 1713000000000,
  "kind": "law.imported",
  "adapter": "bootstrap",
  "source": { "repo": "Rulebook", "path": "rulebook/laws/LAW-007.yaml" },
  "redacted_payload": {
    "law_id": "LAW-007", "title": "...", "severity": "critical",
    "detector": "hook:pre_commit_no_skip", "body": "...markdown..."
  },
  "content_hash": "sha256:..."
}
```

### Checkpoint file

```jsonc
{
  "version": 1,
  "started_at": "2026-04-17T20:30:00Z",
  "repos": {
    "Vectorizer": {
      "files_walked": 4821, "files_total": 4821,
      "commits_walked": 3124, "commits_total": 3124,
      "events_emitted": 9573,
      "status": "done",
      "last_git_ref": "abc123"
    },
    "Nexus": {
      "files_walked": 1204, "files_total": 5200,
      "status": "in_progress",
      "last_file": "src/query/parser.rs"
    }
  }
}
```

Written atomically (write-rename) every 5 s while running. On `--resume`, the CLI picks up from `last_file` / `last_git_ref`.

## Design

### Pipeline (per repo, parallel across repos)

```
 ┌─────────────────┐
 │  Repo loader    │  reads cortex.toml, applies defaults
 └────────┬────────┘
          ▼
 ┌─────────────────┐     ┌──────────────────┐     ┌────────────────────────┐
 │  File walker    │ ──▶ │  Chunker prefilter│ ──▶ │  Synthetic event emit  │
 │ (ignore list)   │     │  (quick size/ext) │     │  (envelope assembler)  │
 └─────────────────┘     └──────────────────┘     └───────────┬────────────┘
                                                              │
 ┌─────────────────┐                                          │
 │  Git log walker │ ──▶ turn.historical events   ────────────▶
 └─────────────────┘                                          │
                                                              ▼
 ┌─────────────────┐                             ┌────────────────────────┐
 │  ADR / OpenSpec │ ──▶ decision.imported ────▶ │  Redactor (cortex-core) │
 │  recognizer     │                             │  (static patterns)      │
 └─────────────────┘                             └───────────┬────────────┘
                                                              ▼
 ┌─────────────────┐                             ┌────────────────────────┐
 │  Law / memory   │ ──▶ law.imported +         ─▶│ Publisher → Synap      │
 │  recognizer     │      memory.imported         │ cortex.events.bootstrap │
 └─────────────────┘                             └────────────────────────┘
```

Everything downstream (classifier, embedder, graph, full-text) is the same as live ingestion — the CLI does **not** touch those systems directly.

### File walker

- Uses `ignore` crate (respects `.gitignore` by default, plus `cortex.toml` excludes).
- Emits `artifact.code` for source files (language detected by extension or Tree-sitter), `artifact.doc` for `*.md`.
- Files > 10 MB are skipped with a warning (same rule as spec 08 truncation, but here we skip entirely because they likely aren't source).

### Git log walker

- `git log --all --diff-filter=AMD --name-only --format='%H|%at|%ae|%s|%b'`.
- One event per commit. PR body (via `gh api`) is appended if `cortex.git.include_prs = true` and a PR number is derivable from the commit message.
- Merge commits (multiple parents) are emitted once; their diffs are the **squashed** diffs from the nearest non-merge parent.

### Redaction

Every synthetic event passes through `cortex-core`'s redactor (spec 04) **before** publication. Repo-specific extra patterns merge into the global catalog for the duration of that repo's walk.

### Idempotency

- Each event carries `content_hash = sha256(canonical_json(redacted_payload))`.
- Downstream writers (spec 06 embedder, spec 08 indexer) are keyed on `content_hash` — re-running bootstrap is a no-op at the storage layer.
- The checkpoint file prevents **re-publishing** unchanged events to Synap, saving classifier cost.

### Progress & telemetry

Progress is surfaced three ways:

1. **Stderr progress bar** (one line per repo) when `--log-format=text` and TTY.
2. **Structured logs** (JSON lines) when `--log-format=json`.
3. **Metrics** emitted to `cortex.metrics`:

```
cortex.bootstrap.files.walked     counter, labels: repo
cortex.bootstrap.files.skipped    counter, labels: repo, reason
cortex.bootstrap.events.emitted   counter, labels: repo, kind
cortex.bootstrap.bytes.processed  counter, labels: repo
cortex.bootstrap.commits.walked   counter, labels: repo
cortex.bootstrap.repo.duration_s  histogram, labels: repo
cortex.bootstrap.errors           counter, labels: repo, stage
```

### `--estimate` mode

Walks the repo without emitting events; prints:

```
Repo: Vectorizer
  Files (after excludes):   4 821
  Code chunks (est):       25 400   [Tree-sitter symbol-level]
  Doc chunks (est):         3 100   [Markdown section-level]
  Commits:                  3 124
  Est. events:            31 624
  Est. redacted bytes:     48.2 MB
  Est. Haiku classifier cost (input tokens): 12.1 M  → ~$1.06
  Est. Haiku classifier cost (output tokens): 4.3 M  → ~$0.22
  Est. embedding storage:   71 MB
  Est. graph nodes:        28 900 (+ 52 000 edges)
  Est. full-text index:    94 MB
  Est. one-time runtime:   32 min (at 500 eps)
```

Numbers are back-of-envelope; real cost depends on classifier cache hit rate.

### Failure modes

| Failure                         | Handling                                                                     |
|---------------------------------|------------------------------------------------------------------------------|
| Synap unreachable               | Fail fast                                                                     |
| Repo path does not exist        | Fail with a clear error listing the provided paths                            |
| `.git` missing                  | Walk source files only; skip git log; warn                                    |
| Tree-sitter grammar missing     | Fall back to fixed-window chunking (same rule as spec 06); warn               |
| Oversize file (>10 MB)          | Skip; counter `bootstrap.files.skipped{reason=oversize}`                      |
| Checkpoint write failure         | Fail fast (can't guarantee resume correctness)                                |
| `gh` CLI missing                 | PR enrichment disabled; warn once per run                                     |
| Partial repo run interrupted (Ctrl-C) | Checkpoint still valid; `--resume` picks up                                |

## Rulebook artifact indexing

The walker promotes the canonical `.rulebook/` layout into typed
events on every repo, regardless of whether the repo ships a
per-repo `cortex.toml`. This closes the 2026-04-27 audit's
`decisions = 0` / `laws_active = 0` finding for spec-driven
projects: the institutional knowledge already exists on disk,
the indexer just needed to know where to look.

### Path → kind mapping

| Repo-rooted path                          | Walker class         | Emitter event                                   | Notes |
|-------------------------------------------|----------------------|-------------------------------------------------|-------|
| `.rulebook/decisions/**/*.md`             | `FileClass::Decision` | `decision.imported` (one per file)              | ADR-style; first H1 → title, `Status:` → status. |
| `.rulebook/specs/**/*.md`                 | `FileClass::Law`      | `law.imported` (**fan-out** — one per `## ` heading) | Spec doc → addressable laws. See "Spec splitter" below. |
| `.rulebook/knowledge/patterns/**/*.md`    | `FileClass::Memory`   | `memory.imported`                               | Reusable patterns; surfaced in `/v1/dashboard/memory`. |
| `.rulebook/knowledge/anti-patterns/**/*.md` | `FileClass::Memory` | `memory.imported`                               | Anti-patterns; same surface as patterns. |
| `.rulebook/learnings/**/*.md`             | `FileClass::Memory`   | `memory.imported`                               | Implementation insights — clipped excerpts in the Memory view. |
| `.rulebook/handoff/**/*.md`               | `FileClass::Memory`   | `memory.imported`                               | Cross-session hand-off snapshots; per-project Handoffs view. |
| `.rulebook/{PLANS,STATE,COMPACT_CONTEXT}.md` | `FileClass::Memory` | `memory.imported`                               | Loose top-level memory files. |

The patterns are hardcoded in `walker::RULEBOOK_DECISION_GLOBS`,
`RULEBOOK_LAW_GLOBS`, and `RULEBOOK_MEMORY_GLOBS` so a sibling
repo (Nexus / Vectorizer / Synap / etc.) without its own
`cortex.toml` still gets full discovery coverage. Per-repo
`cortex.toml` `[cortex.{decisions,laws,analyses,memories}]`
patterns continue to win when set — explicit promotion always
overrides the canonical defaults.

### Spec splitter (`emit_spec_laws_imported`)

A spec doc like `.rulebook/specs/RULEBOOK.md` typically carries
many distinct rules under `## ` headings. Treating the whole doc
as one law collapses every rule under one un-addressable id.
The splitter solves this by:

1. Walking lines, treating each `## ` at column 0 as a section
   boundary.
2. Synthesising `law_id = LAW-{filename-stem}-{section-index}-{section-slug}`
   for every section so operators can grep for the prefix to find
   every law from a given spec.
3. Stamping the section heading text as `title`.
4. Carrying the section body (heading + content up to the next
   `## ` or EOF) as `body`.
5. Emitting one `law.imported` event per section.

Preamble text (before the first `## `) is intentionally dropped
— the dashboard already surfaces the full doc as a `memory`
entry; this path only materialises the *addressable* laws.

When a spec doc has zero `## ` boundaries, the splitter falls
back to `emit_law_imported`'s single-law-per-file shape so
flat rule files keep working.

### Downstream contract

- **Classifier worker** (`cortex-classifier-worker::kinds`):
  `law.imported` normalises to `Kind::LawViolation` (the canonical
  Kind enum has no separate `Law` variant today; "imported law"
  and "law violation" share storage but never collide because
  imported laws never carry a `violation_id`).
- **Fulltext worker**: routes the event to
  `cortex-{repo-slug}-governance`. Each spec section becomes its
  own Meili doc, searchable by `law_id` / `title` / `body`.
- **`/v1/dashboard/laws`** + **`laws_active` overlay**: read every
  governance hit, populate `LawRef` entries when `extras.law_id`
  is present. The orchestrator's `derive_laws` already reads this
  field — no changes needed downstream.

### Verification path

After re-bootstrapping a repo:
1. `cortex-bootstrap <repo>` walks the `.rulebook/specs/**` tree
   and emits N `law.imported` events per spec doc.
2. `cortex-fulltext-worker` indexes each into
   `cortex-{slug}-governance`.
3. `cortex-api` boot's `meili_loader` projects them onto the
   keyword lane with `extras.law_id` stamped.
4. `curl http://127.0.0.1:17000/v1/dashboard/laws` shows the
   per-section laws under `LAW-{stem}-NN-{slug}` ids.

## Acceptance criteria

- [ ] `cortex-bootstrap Vectorizer/ --estimate` completes on a cold cache, prints the sizing block, writes no events.
- [ ] `cortex-bootstrap Vectorizer/` end-to-end produces events across all four downstream writers (Vectorizer, Nexus, Meilisearch); verified via `cortex query` (spec 11 stub) or direct queries to each service.
- [ ] Re-running the same `cortex-bootstrap Vectorizer/` produces near-zero new chunks/nodes/docs (cache hits + content-hash dedup).
- [ ] `--only Vectorizer,Nexus` restricts to two repos and ignores others passed on CLI.
- [ ] `--since HEAD~200` emits events only for commits within the last 200.
- [ ] Checkpoint + resume: SIGINT mid-run; `--resume` picks up without duplicate events.
- [ ] ADR recognition: a file matching `cortex.decisions.promote_patterns` produces `decision.imported` events and ends up as a `Decision` node in Nexus.
- [ ] Law recognition: a file matching `cortex.laws.promote_patterns` produces `law.imported` events and ends up as a `Law` node in Nexus (spec 13 seeding).
- [x] Analysis recognition: a file matching `cortex.analyses.promote_patterns` produces `analysis.imported` events, lands in `cortex-{repo}-analyses` (Meili + Vectorizer), and surfaces as `(:Analysis)-[:ANALYZES]->(:Repo)` in Nexus.
- [ ] Redaction: a committed `.env` file with synthetic secrets is detected + patterns are stripped before publication; unit test asserts no secret bytes reach Synap.
- [ ] Parallel walk: `--parallelism 4` against 4 small repos finishes in ~1/4 the wall-clock of sequential; no event loss.
- [ ] Tree-sitter-missing language: Elixir files land with `source=fallback_window` chunks downstream.
- [ ] `--dry-run` writes no events, prints the plan.
- [ ] Telemetry counters non-zero at end of a real run; bootstrap run for the 17-repo corpus finishes within the 4–8 h envelope (architecture §6.5).

## Decisions

1. **Synthetic events flow through the live pipeline.** No "bootstrap mode" shortcuts. If a bug shows up in the classifier, it shows up for live events too — one system of record.
2. **Separate Synap stream.** `cortex.events.bootstrap` is distinct from `cortex.events.raw` so we can throttle or pause bootstrap without starving live capture.
3. **File-level checkpointing.** Per-commit / per-file granularity; resume is trivially correct. Sacrifices some resume speed in exchange for no-duplicate guarantee.
4. **Respect `.gitignore` by default.** Surprising developers by indexing `target/` bytes is worse than missing something. Explicit opt-in via `cortex.toml`.
5. **PR enrichment is optional.** `gh` auth is not always available. Degrade gracefully.
6. **No reindex triggers from the CLI.** The CLI writes events; it does not reach into Vectorizer/Nexus/Meilisearch. Everything downstream is idempotent, so a re-run is the refresh mechanism.

## Open questions

1. **Cross-repo symbol resolution.** When the same function name appears in multiple repos, do we emit `SIMILAR_TO` hints here or defer to query-time derivation? Deferring to spec 11 for now.
2. **Incremental bootstrap scheduling.** Should `--since` default to the checkpoint's `last_git_ref` on subsequent runs, making continuous bootstrap trivial? Yes; tracked as a follow-up once the Phase-2 tailing spec lands.

## References

- Architecture §6 (bootstrap), §6.3 (repo priority list), §6.5 (sizing).
- Spec 01 — Event schema.
- Spec 04 — Cortex Core (redactor, ingestion router).
- Spec 05 — Classifier (downstream consumer).
- Spec 06 — Embedder (symbol-level chunking).
- Spec 07 — Graph writer (consumes decision/law events).
- Spec 08 — Full-text indexer.
- Spec 10 — Claude Code adapter (not this spec, but a sibling producer of events).
- `ignore` crate: https://docs.rs/ignore (gitignore-aware walker).
- `gh` CLI: https://cli.github.com (optional PR enrichment).
