# cortex-bootstrap

> Spec: [`docs/specs/09-bootstrap-cli.md`](../../docs/specs/09-bootstrap-cli.md)

One-shot + incremental backfill CLI for Cortex. Walks an existing
Hive repo and republishes its content — files, commits, ADRs, laws,
memories, audits — as envelope-compliant events on
`cortex.events.bootstrap`, where `cortex-classifier-worker` picks
them up and fans them out to the embedder, graph, and full-text
workers.

```
repo on disk ──▶ walker ──▶ emitter ──▶ Synap (cortex.events.bootstrap)
              (.gitignore)  (synthetic                ▲
              (cortex.toml)  envelopes)               │
                                                      ▼
                                          cortex-classifier-worker
                                                      ▼
                                          cortex.events.enriched
```

## Per-repo configuration (`cortex.toml`)

The walker reads `cortex.toml` from the repo root. Missing or
incomplete files fall back to spec-09 defaults — common-junk
excludes, symbol chunking for code, section chunking for docs, all
git commits included, no PR enrichment.

```toml
[cortex]
id = "Cortex"

[cortex.exclude]
paths = ["target/", "node_modules/", ".git/"]
extensions = ["lock", "log", "png"]

[cortex.chunking]
code_strategy = "auto"
doc_strategy = "section"

[cortex.git]
include_commits = true
include_prs = false

[cortex.decisions]
promote_patterns = ["docs/decisions/*.md", ".rulebook/decisions/**/*.md"]

[cortex.laws]
promote_patterns = [".rulebook/laws/*.yaml", ".claude/rules/*.md"]

[cortex.analyses]
promote_patterns = ["docs/analysis/**/*.md", "docs/analyses/**/*.md"]

[cortex.memories]
import_files = ["CLAUDE.md", "AGENTS.md", ".rulebook/memory/**/*.md"]
```

A canonical `.rulebook/*` layout (decisions, knowledge, learnings,
specs, handoffs, PLANS.md / STATE.md) is rescued automatically even
when the repo ships no `cortex.toml` of its own — sibling Hive repos
(Nexus, Vectorizer, Rulebook, Synap) benefit from the same discovery
defaults.

## File classification

| `FileClass` | Bootstrap kind         | Default routing                                               |
|-------------|------------------------|----------------------------------------------------------------|
| `Code`      | `artifact.code`        | `cortex-{repo}-code` (Vectorizer + Meili)                      |
| `Doc`       | `artifact.doc`         | `cortex-{repo}-docs`                                           |
| `Decision`  | `decision.imported`    | `cortex-{repo}-decisions` + `(:Decision)` in Nexus             |
| `Law`       | `law.imported`         | `cortex-{repo}-governance`                                     |
| `Memory`    | `memory.imported`      | `cortex-{repo}-misc`                                           |
| `Analysis`  | `analysis.imported`    | `cortex-{repo}-analyses` + `(:Analysis)-[:ANALYZES]->(:Repo)`  |

## CLI

```bash
# walk the current repo and publish to the local Synap stack
cortex-bootstrap .

# multi-repo walk (subset)
cortex-bootstrap repos/Vectorizer repos/Nexus

# only re-emit since a git ref (incremental)
cortex-bootstrap . --since main

# dry run + sizing block
cortex-bootstrap . --dry-run --estimate

# resume from the last checkpoint
cortex-bootstrap --resume
```

| Flag                    | Notes                                                          |
|-------------------------|----------------------------------------------------------------|
| `--config <FILE>`       | Override `./cortex-bootstrap.toml` global config.              |
| `--only <NAME[,…]>`     | Whitelist by repo `id`.                                        |
| `--skip <NAME[,…]>`     | Blacklist by repo `id`.                                        |
| `--since <GIT-REF>`     | Incremental since a ref.                                       |
| `--dry-run`             | No writes; print the plan.                                     |
| `--estimate`            | Implies `--dry-run`; prints the spec-09 sizing block.          |
| `--resume`              | Resume from the last `.cortex-bootstrap.state.json` checkpoint.|
| `--parallelism <N>`     | Concurrent repo walkers (default 4).                           |
| `--synap-endpoint <URL>`| Override Synap connection URL.                                 |
| `--stream <NAME>`       | Override the destination stream.                               |

## Checkpoint

Every successful repo walk is recorded in
`.cortex-bootstrap.state.json` — files walked, commits walked, events
emitted, last-seen file, last git ref. `--resume` reads that state
and skips already-completed repos.

## Tests

```bash
cargo test -p cortex-bootstrap
```

Unit tests cover the walker classifier, the rescue-walk gate, the
emitter shapes (every kind), and the YAML / Markdown front-matter
parsers used by `decision.imported`, `law.imported`, and
`analysis.imported`.

## Stability

Pre-1.0. The synthetic event shape mirrors spec 01 envelopes — when
the envelope schema bumps, this crate bumps with it.
