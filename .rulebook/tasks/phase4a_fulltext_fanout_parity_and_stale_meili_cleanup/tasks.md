## 1. Diagnostic — pin the fan-out gap before patching
- [ ] 1.1 Read `cortex-fulltext` worker logs since the last bootstrap of `Rulebook` and `Vectorizer`; record whether non-Cortex events were ever observed
- [ ] 1.2 Inspect the event archive partition layout under `~/.cortex/archive/events/` and confirm whether `Rulebook`/`Vectorizer` events are present in the stream the worker consumes
- [ ] 1.3 Capture findings in `.rulebook/learnings/phase4a_fulltext_drift.md` with the root cause statement (what was known, what was missing, what the data shows)

## 2. Replay-missing-repos path
- [ ] 2.1 Add `MeiliClient::list_indexes` and `delete_index` if not already present
- [ ] 2.2 In `worker::run`, on startup, list existing Meili indexes and compare against the `(repo, family)` set observed in the event archive
- [ ] 2.3 For every missing partition, replay all archived events whose routing maps to that index — idempotent via Meili upsert by `id`
- [ ] 2.4 Emit per-replay metrics: `cortex_fulltext_replay_events_total{repo,family}` and a final summary log line

## 3. Stale-index sweep on boot
- [ ] 3.1 Compile the regex `^cortex-([a-z0-9_-]+)-(code|docs|decisions|turns|governance|misc)$` once at startup
- [ ] 3.2 List indexes; for each name that does NOT match AND has `numberOfDocuments == 0`, delete; log the deletion at info
- [ ] 3.3 For non-matching names with `numberOfDocuments > 0`, emit a warning naming the index and DO NOT delete
- [ ] 3.4 One-shot drop of the six known stale indexes (`cortex-code`, `cortex-decisions`, `cortex-docs`, `cortex-governance`, `cortex-misc`, `cortex-turns`); accept HTTP 404 as success

## 4. Routing invariant guard
- [ ] 4.1 Extend `routing::index_name` with `debug_assert!` that the produced name splits into exactly 3 tokens on `-`
- [ ] 4.2 Property test: random `(prefix, repo_slug, family)` triples ALWAYS produce a 3-token name
- [ ] 4.3 Reject empty `repo_slug` at compile-time-equivalent (return `Err` or panic in debug); production fallback already lands in `UNKNOWN_REPO_SLUG`

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation (extend spec-08 with a `## Replay & sweep` section)
- [ ] 5.2 Write tests covering the new behavior: Meili pre-seeded with cortex-only data + 1 stale `cortex-misc` index; archive contains rulebook events; after worker boot, assert (a) `cortex-rulebook-misc` exists and is non-empty, (b) `cortex-misc` is gone
- [ ] 5.3 Run tests and confirm they pass (`cargo check -p cortex-fulltext` → `cargo clippy -p cortex-fulltext --all-targets -- -D warnings` → `cargo test -p cortex-fulltext`)
