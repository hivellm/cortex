# phase10h — scope.{since,topics,files} fan out to lane filters
**Source**: manual
**Date**: 2026-04-30
**Related Task**: phase10h_query_scope_filters
**Tags**: scope, filter, meili, vectorizer, phase10h
The 2026-04-29 audit caught `/v1/query` accepting the full `Scope { repo, files, since, topics }` shape but only `repo` actually filtering. `since` / `topics` / `files` were silently dropped at fan-out, so a query for "decisions about Meili from the last 30 days" returned every Meili-related row ever indexed.

Phase10h wires the dimensions, with each lane handling them per its own capabilities:

1. **Meili lane** — new `build_meili_filter(scope, index)` composes a per-index AND-joined filter expression. Capabilities are looked up via `caps_for(index)` because Meili rejects clauses against unfilterable attributes with a 4xx:
   - Per-repo legacy `cortex-{slug}-{family}` indexes use `ts` for `since`, plus `repo` / `path` / `topics`.
   - Global `cortex_turns` / `cortex_decisions` / `cortex_analyses` use `occurred_at`.
   - `cortex_laws` carries no scope facets (severity / applies_to only) — clauses skipped.
   - `cortex_knowledge` / `cortex_learnings` (phase10e) use `occurred_at` + `repo` + tags.

2. **Vectorizer lane** — the SDK's `search_vectors` doesn't expose a server-side filter parameter, so the lane filters client-side after projection (`scope_matches`). Fail-open: hits whose metadata lacks the filtered field round-trip rather than being dropped silently. Better to surface a possibly out-of-window row than to drop everything.

3. **Nexus graph lane** — graph hits don't carry per-row timestamp / topic / path in the projected `LaneHit`, so structural filtering against the graph is deferred to a follow-up. Document scope still flows into `scope_resolved`.

4. **Audit envelope** — `canonicalise_scope` in `service.rs` now normalises every dimension: lowercased `repo`, deduplicated + lowercased `topics`, trimmed `since`, trimmed non-empty `files`. The audit envelope's `scope_resolved` echoes the canonical form so dashboards show the applied filters, not the raw user input.

5. **Pre-thinking scope inference** — new `topic_for_path` maps file extensions to canonical topics (`code` / `docs` / `config`). `derive()` stamps topics from recent files + prompt-mentioned paths so the bundle naturally scopes to the corpus the user is editing.

Multi-Meili-index awareness was the trickiest part: per-repo indexes use `ts` while global indexes use `occurred_at`. A pure post-projection filter would have been simpler but would have wasted Meili budget on filters Meili could enforce server-side.