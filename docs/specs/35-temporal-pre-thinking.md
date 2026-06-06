# 35 — Temporal Pre-Thinking Bundle

> **Status:** 🟡 P5 partially shipped (schema + budget + audit; live population pending) · **Owner:** Core team · **Depends on:** 12, 30, 31, 32
> **Phase:** phase18_tlb-timeline-branching

## Goal

Inject temporal anchors into the pre-thinking bundle (spec 12) so the LLM sees recent
bitemporal events, active vs. superseded decisions, and branch context before the query
body. Built on spec 30 schema, spec 31 classifier, and spec 32 branches; reuses the
bundle machinery to keep sections supplementary and budget-aware.

## Scope

**In:**

- Three grounding sections added to `GroundingSections` as `Option<_>` (None ⇒ omitted):
  - `timeline_window` — recent bitemporal `TimelineEvent`s on the active project+branch as of the query instant. Max 8 rows, 1400 bytes.
  - `supersession_overlay` — active decisions + recently superseded (successor, predecessor) pairs. Max 1000 bytes.
  - `branch_context` — current branch + active siblings + recently merged. Max 800 bytes.
- Rendering rules: each section returns `""` when input is `None` or fully empty; renders
  up to its byte cap; emits `… (truncated)` when cap is hit (mirrors `render_similar_sessions`).
- Three sections render at the TOP of the grounding block (before `render_active_work`) so
  the LLM sees temporal anchors first.
- Supplementary sections: NOT part of empty-bundle short-circuit. A bundle with only
  temporal sections and no main section still returns empty (spec 12 Decision 4).
- Budget participation: self-byte-capped, render into the bundle string that `clip_to_budget`
  measures. No dedicated `TrimStep` — consistent with other grounding sections.
- Audit: `count_sections(response, opts)` returns `BTreeMap<String, u32>` with keys
  `timeline_window`, `supersession_overlay`, `branch_context` (only when count > 0).
  `ClippedBundle.section_counts` carries the map; `pipeline.rs` emits `observe_section_count`
  per entry.

**Out:**

- Live population from cortex-api per request (fetching timeline window / supersession
  overlay / branch context from the graph + meilisearch). Boundary: `clip_to_budget` builds
  `FormatOptions::default()` (empty grounding), so pipeline sections render empty until
  a follow-up wires the population path. Direct `format_bundle` callers that populate
  `FormatOptions.grounding` get them today.
- Temporal classifier wedge itself — spec 31.
- Branch surfaces (CLI / HTTP / MCP) — spec 32.

## Bundle sections

### TimelineWindow

```rust
pub struct TimelineWindow {
    pub project: String,           // project identifier
    pub as_of: String,             // RFC3339 timestamp; empty = "now"
    pub branch: String,            // "<project>:<branch>"; empty = main
    pub recent_events: Vec<TimelineEventRow>,
}

pub struct TimelineEventRow {
    pub event_id: String,          // ULID of the TimelineEvent
    pub kind: String,              // 12 discriminators (commit, adr, decision, …)
    pub title: String,             // ≤ 80 chars
    pub valid_from: String,        // RFC3339 timestamp
    pub summary: String,            // ≤ 2 KiB markdown
}
```

**Header:** `## Timeline window — {project}@{branch} as of {as_of|now} ({n} events)`

**Rendering:**

- The renderer MUST skip the section when `w` is `None` or `recent_events` is empty.
- The renderer MUST cap to `min(recent_events.len(), TIMELINE_WINDOW_EVENTS=8)` rows.
- Format per row: `{index}. [{kind}] {valid_from} · {title}\n   {summary}\n`
- The renderer MUST NOT exceed `TIMELINE_WINDOW_BYTES = 1400`; when the cap is hit, emit
  `… (truncated)\n` and stop.

### SupersessionOverlay

```rust
pub struct SupersessionOverlay {
    pub active_decisions: Vec<ActiveDecisionRow>,
    pub recently_superseded: Vec<SupersessionPairRow>,
}

pub struct ActiveDecisionRow {
    pub decision_id: String,        // decision identifier
    pub title: String,              // display title
}

pub struct SupersessionPairRow {
    pub successor_id: String,       // ULID of the successor decision
    pub successor_title: String,    // display title
    pub predecessor_id: String,     // ULID of the decision that was superseded
    pub predecessor_title: String,  // display title
}
```

**Header:** `## Supersession overlay`

**Rendering:**

- The renderer MUST skip the section when `o` is `None` or both lists are empty.
- Sub-section "Active decisions:" lists active-lifecycle decisions one per line:
  `- {decision_id} · {title}`
- Sub-section "Recently superseded:" lists (successor, predecessor) pairs:
  `- {successor_id} ({successor_title}) ⊃ supersedes {predecessor_id} ({predecessor_title})`
- The renderer MUST NOT exceed `SUPERSESSION_OVERLAY_BYTES = 1000`; when the cap is hit,
  emit `- … (truncated)\n` and return immediately (stop adding rows).

### BranchContext

```rust
pub struct BranchContext {
    pub current_branch: String,     // composite "<project>:<branch>"; empty = main
    pub active_sibling_branches: Vec<BranchRefRow>,
    pub recently_merged: Vec<BranchRefRow>,
}

pub struct BranchRefRow {
    pub branch_id: String,          // composite "<project>:<branch>"
    pub status: String,             // "active" | "merged" | "abandoned"
}
```

**Header:** `## Branch context — {current_branch|—}`

**Rendering:**

- The renderer MUST skip the section when `b` is `None` or all three lists are empty
  (current_branch empty AND siblings empty AND merged empty).
- Sub-section "Active siblings:" lists active-status branches one per line:
  `- {branch_id} [{status}]`
- Sub-section "Recently merged:" lists merged branches:
  `- {branch_id} [{status}]`
- The renderer MUST NOT exceed `BRANCH_CONTEXT_BYTES = 800`; when the cap is hit, emit
  `- … (truncated)\n` and return immediately.

## Ordering & budget

**Top-of-grounding placement:** The three sections render in order (timeline → supersession
→ branch) immediately after the header comment and the "Active laws" block, before
`render_active_work`. This ensures the LLM has temporal context before the query body.

**Supplementary contract:** The three sections do NOT participate in the empty-bundle
short-circuit. If all main sections (laws, decisions, snippets, graph) are empty but a
temporal section has data, the bundle still returns empty (per spec 12 Decision 4). This
keeps temporal grounding from forcing a bundle render when the query had no hits.

**Budget participation:** Each section's render output is included in the final bundle
string that `clip_to_budget` measures. The byte caps (`TIMELINE_WINDOW_BYTES`, etc.)
ensure individual sections stay within bounds. No dedicated `TrimStep` ladder entry —
temporal sections trim themselves inline (mirrors `render_similar_sessions`).

## Audit & observability

**count_sections (§6.2):** Returns a `BTreeMap<String, u32>` with:

- `"timeline_window"`: `recent_events.len()` clamped to `TIMELINE_WINDOW_EVENTS = 8`
- `"supersession_overlay"`: `active_decisions.len() + recently_superseded.len()`
- `"branch_context"`: `active_sibling_branches.len() + recently_merged.len()`

Only keys with count > 0 are inserted (keeps the map compact).

**ClippedBundle.section_counts:** The result of `count_sections` is carried in the
`ClippedBundle` envelope returned by `clip_to_budget`. The pipeline (phase18 §6.2)
invokes `observe_section_count(section_name, count)` for every entry, stamping metrics
to the audit stream.

## Boundary

**Current state:** The schema + rendering + budget machinery + audit are shipped. Sections
render correctly when directly populated via `FormatOptions.grounding`. However, the live
pre-thinking pipeline uses `FormatOptions::default()` (empty `GroundingSections`), so
temporal sections emit empty strings until a follow-up task wires the population path.

**Follow-up:** Fetch the three structures from cortex-api (timeline window from
`TimelineEvent` graph nodes, supersession overlay from `Decision` query + SUPERSEDES walk,
branch context from `Branch` query) and populate `FormatOptions.grounding` at request
time. This unblocks live temporal anchors in production.

## Pinned tests

**Render tests** — `crates/cortex-pre-thinking/src/formatter.rs::tests`:

- `render_timeline_window_emits_events_and_caps` — window with 3 events caps to 3, emits
  header + events + summary per line.
- `render_timeline_window_empty_is_blank` — `None` and empty `recent_events` both return `""`.
- `render_supersession_overlay_lists_active_and_superseded` — overlay with 2 active + 1
  superseded pair emits header + sub-sections + data.
- `render_supersession_overlay_empty_is_blank` — `None` and empty lists both return `""`.
- `render_branch_context_lists_siblings_and_merged` — context with 1 sibling + 1 merged
  branch emits header + sub-sections + data.
- `render_branch_context_empty_is_blank` — `None` and all-empty lists both return `""`.

**Count & budget tests** — `crates/cortex-pre-thinking/src/formatter.rs::tests` +
`crates/cortex-pre-thinking/src/budget.rs::tests`:

- `count_sections_timeline_window` — populated window counts as the capped event count.
- `count_sections_supersession_overlay` — populated overlay counts as active + superseded.
- `count_sections_branch_context` — populated context counts as siblings + merged.
- `count_sections_omits_zero_entries` — sections with no data omit their keys from the map.
- `clip_to_budget_includes_temporal_sections_in_measurement` — temporal sections contribute
  bytes to the budget; when combined, they may trigger downstream trim steps.
