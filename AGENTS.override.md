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

## LAW-CORTEX-002 — Long autonomous sessions: chain tasks, don't ask to continue

**Trigger:** any time there is a backlog of work — multiple Rulebook tasks
(e.g. several `phase0_*` bug-fixes), a multi-section `tasks.md`, a list of
findings, or the user says "continue" / "continua" / "siga".

**Rule:** Work the backlog end-to-end in ONE session. Finish a task, then
**immediately pick up the next** one without stopping to ask. The default
is to keep going until the backlog is empty or a genuine stop condition
(below) fires — not to checkpoint-and-ask after each task.

- After completing + committing a task, go straight to the next eligible
  task (lowest-numbered pending, per LAW-CORTEX-001) and start it. Do NOT
  end the turn with "want me to continue?", "should I take X next?", or a
  status report that waits for approval.
- Commit per task (each commit is the checkpoint), keep the docs/tests/
  archive tail per task, and run the quality gates per task — but never
  pause the session for permission between tasks.
- A single "continue" from the user authorizes the WHOLE remaining
  backlog, not just the next one task.
- Prefer delegating sub-work to agents/Teams (per the delegation rules)
  to stretch the session further without burning the main context.

**The ONLY stop conditions** (otherwise keep working):

1. **Genuine ambiguity that changes the result** — two+ valid
   interpretations where picking wrong wastes real work. Ask, then resume
   the backlog with the answer. (A conventional default or a verifiable
   fact is NOT ambiguity — decide and proceed.)
2. **Destructive / outward-facing op needing authorization** — deletes,
   force-push, dropping data, publishing externally. Ask only for that
   op; keep going on everything else.
3. **Hard external blocker** — a task depends on a service, credential,
   or upstream fix the user controls (mark it `blocked`, file/track it,
   move to the next task).
4. **Context-handoff force threshold** — `respect-handoff-trigger.md`
   still applies at the FORCE threshold (default 90%): invoke `/handoff`
   and stop. Below that, keep chaining. (The user may raise
   `handoff.warnThresholdPct` / `forceThresholdPct` in
   `.rulebook/rulebook.json` to lengthen sessions further.)

**Why this is a Law:** the agent has repeatedly done one task, then ended
the turn with a summary + "want me to continue?", forcing the user to
re-prompt for work already implicitly authorized. That fragments a
backlog across many round-trips. One "continue" means: drain the queue.

**Interaction with other laws:** LAW-CORTEX-001 (strict sequence) still
governs ORDER within a task; this law governs CONTINUITY across tasks.
`full-task-no-questions.md` is reinforced, not replaced. Quality gates,
git-safety, and the mandatory task tail are never skipped for speed.
<!-- OVERRIDE:END -->
