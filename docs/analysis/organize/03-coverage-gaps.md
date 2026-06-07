# 03 — Coverage gaps (designed vs. captured)

Snapshot 2026-05-01, 14:00Z. `cortex-api` daemon up, `git_sha=888aeda`
(stale; see `phase11h_cortex_query_recall_recovery`). Numbers below
were pulled directly from `/v1/health/coverage` and authenticated
Meili `/indexes/{name}/stats` calls.

## 1. Backend health

| Backend | Expected | Present | Missing | Unexpected | Severity |
|---|---:|---:|---:|---:|---|
| Vectorizer | 144 | 4 | 140 | 0 | warn |
| Meili | 144 | 29 | 115 | 7 | warn |

Vectorizer present: `cortex-cortex-{code,docs,governance,misc}` only.
Meili unexpected: 7 legacy `cortex-{family}` indexes with no repo
prefix — leftovers from the pre-slug naming scheme. Both classes are
addressed by phase 11h §2.

## 2. What's captured today (live)

Every entry verified against either `/v1/health/coverage` doc counts
or the running adapter daemon's emit log.

| Source | Hook / path | Captured | Lane | Doc count (cortex repo) |
|---|---|---|---|---:|
| User prompts | `cortex-user-prompt.sh` → adapter | ✅ | turns | 673 |
| Assistant responses | `cortex-stop.sh` (folds into Turn) | ✅ | turns | (same Turn) |
| Tool calls | `cortex-post-tool.sh` | ✅ | code | 1 986 (cortex-cortex-code) |
| Sub-agent calls | `cortex-subagent-stop.sh` | ⚠️ partial | turns (designed) | unverified |
| File edits (paths) | `tool_call.touched[]` | ⚠️ partial (no diffs) | code | embedded in tool_call |
| Bootstrap commits | `cortex-bootstrap` | ✅ historical | turns | (folded into 673) |
| ADRs | bootstrap promote pattern | ⚠️ historical only | decisions | 10 |
| Analyses | bootstrap promote pattern | ⚠️ historical only | analyses | 33 |
| Knowledge | rulebook MCP | ⚠️ partial | knowledge | 3 (rulebook repo) |
| Learnings | rulebook MCP | ⚠️ sparse | learnings | low single digits |

## 3. What's NOT captured

Designed in the specs but not landing in any lane today:

| Source | Spec | Why not | Impact |
|---|---|---|---|
| Live git commits | spec 10 §pre-commit | no git hook wired | every new commit invisible until next bootstrap |
| Slash command invocations | not in schema | no kind, no hook | `/clear`, `/loop`, `/handoff` boundaries opaque |
| Session start metadata | spec 10 (`SessionStart`) | event marked `_drop_` | session-level cohesion has to be reconstructed from Turns |
| Plans (Plan tool) | not modelled | no kind | plan creation/exit invisible |
| TodoWrite mutations | not modelled | no hook | task progress invisible |
| `rulebook_decision_create` | spec 01 (Decision kind) | MCP tool not instrumented | new ADRs only land via next bootstrap pass |
| `rulebook_knowledge_add` | spec 01 (Knowledge kind) | same | sparse knowledge family stays sparse |
| `rulebook_learn_capture` | spec 01 (Learning kind) | same | learnings family stays sparse |
| `rulebook_memory_save` | spec 01 (Memory kind) | same | rulebook memory invisible to Cortex |
| Cortex queries (meta-trail) | not in schema | no self-instrumentation | no ground truth for relevance tuning |
| External AIs (Cursor, Codex, Gemini) | spec 17 | adapters not built | corpus stuck at one AI |
| Conversation archive (`~/.claude/projects/`) | architecture §6 | walker not built | **9 835 sessions / 4.5 GB unread** |

The bottom row is what phase 11i closes. The rows above it are
either tracked in other phases or queued for spec-17 adapter work.

## 4. Per-family doc counts (Meili, cortex repo, after phase11h fix)

These are the counts before redeploy + bootstrap. Phase 11h moves
them; phase 11i adds an order of magnitude on top once the archive
ingests.

| Family | Index | Docs |
|---|---|---:|
| code | `cortex-cortex-code` | 1 986 |
| docs | `cortex-cortex-docs` | (small, mostly markdown) |
| turns | `cortex-cortex-turns` | 673 |
| decisions | `cortex-cortex-decisions` | 10 |
| analyses | `cortex-cortex-analyses` | 33 |
| governance | `cortex-cortex-governance` | empty |
| knowledge | `cortex-cortex-knowledge` | absent |
| learnings | `cortex-cortex-learnings` | absent |
| misc | `cortex-cortex-misc` | small |

Estimated Meili docs after phase 11i ingests `e--HiveLLM-Cortex`
project alone (798 sessions × ~700 records, 30 % keep rate):
**~170 000 new turn-docs + ~170 000 tool_call-docs**. Across all 31
projects: **~2.4 M docs**.

## 5. Hooks already firing (today)

Cortex plugin hooks under `packages/cortex-claude-plugin/hooks/`:

| Hook | Status | Notes |
|---|---|---|
| `cortex-user-prompt.sh` | 🟢 wired | sync with daemon; expects `additionalContext` response |
| `cortex-pre-tool.sh` | 🟢 wired | blocking law-check |
| `cortex-post-tool.sh` | 🟢 wired | tool_call event |
| `cortex-session-start.sh` | 🟡 partial | metadata only; event itself dropped |
| `cortex-stop.sh` | 🟢 wired | turn closure |
| `cortex-subagent-stop.sh` | 🟢 wired | not verified ingesting |
| `cortex-notification.sh` | 🟡 metrics only | no canonical kind |

Rulebook hooks under `.claude/hooks/` are governance only — they
enforce rules on the active session, **they do not feed Cortex**.

## 6. Why this analysis names "phase11i"

Phase 11h (just filed) closes coverage gaps that came from a stale
daemon and an incomplete bootstrap of the existing 16-repo set.
**It does not touch the Claude archive.** Phase 11i sits on top:
it presumes 11h's coverage is `ok`, then adds a wholly new corpus
(`cortex-claude-archive`) and the relevance axes that make it
queryable. Sequencing matters — running 11i with 11h still
outstanding would write the new corpus into a half-bootstrapped
backend and produce inconsistent indexes.

The implementation plan in
[05-implementation-plan.md](./05-implementation-plan.md) calls out
the phase 11h dependency in §1 of the build sequence.
