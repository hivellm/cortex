<!-- OVERRIDE:START -->
# Project-Specific Overrides

These rules sit above every other layer. They survive `rulebook init` and
`rulebook update`. Treat them as **laws** (Tier 0) — they precede even
`AGENTS.md` Tier 1 prohibitions when there is overlap.

## LAW-CORTEX-001 — Strict task-sequence execution

**Trigger:** any time work is governed by a `tasks.md` checklist (Rulebook
task tree, ad-hoc plan, anywhere a numbered list of items appears).

**Rule:** Execute every checklist top-to-bottom in the EXACT order listed.

- Pick the **first unchecked item** in the **lowest-numbered section**.
- Implement THAT item, mark it `[x]`, then pick the next first-unchecked
  item — even when a later item looks "smaller", "more user-visible",
  "more interesting", or "easier to ship now".
- Do NOT cherry-pick across sections.
- Do NOT start section N+1 until every item in section N is `[x]` (the
  optional / N/A path is to mark the item `[x]` with a one-line
  justification, NOT to silently skip it).
- The user OWNS the order. The order encodes dependencies, priorities,
  and rollout strategy that the agent does not always see.

**Why this is a Law and not a guideline:** the agent has cherry-picked
sections multiple times in this project (phase11e §6 was implemented
before §2.2 / §2.3 / §3 / §4 / §5 — all of which sit ahead of it in
`tasks.md`). Each cherry-pick adds reordering debt the next session has
to untangle. Strict sequence is the only invariant that survives a
multi-session task.

**Exemptions (the only three):**

1. **Hard dependency inversion at the language level** — section N has
   an item that literally cannot compile without an item from section
   N+1 (rare; document it inline in `tasks.md`).
2. **External blocker** — section N's next item depends on a service /
   credential / decision the user controls and has not provided. Mark
   the item `[ ] ⏸ blocked: <reason>` and SKIP only this item; the next
   in-section item still runs.
3. **Explicit user override** — the user types "skip §X" or
   "do §Y first". The override is for ONE task, not a precedent.

Outside these three, the agent must NOT pick from later sections.

**Enforcement:** every commit message that lands work for a numbered
section MUST cite the section by `§<N>.<M>` AND, when the work
completes only a subset of the section, list the still-pending items.
A commit that names `§N` while leaving a lower-numbered section
incomplete is a Law violation — the next agent restoring the work
must surface it as such.

## LAW-CORTEX-002 — Reserved

(Keeping numbering reserved so future laws extend cleanly.)
<!-- OVERRIDE:END -->
