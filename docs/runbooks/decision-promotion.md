# ADR promotion runbook (proposed → accepted / superseded / deprecated)

## Why this exists

Every ADR in `.rulebook/decisions/` ships with `status: proposed`.
None of the 8 cortex ADRs (DEC-001 through DEC-009) is currently
`accepted`. As a consequence:

- `cortex_decision_search?status=accepted` returns empty.
- The dashboard cannot distinguish "decision still under
  review" from "decision stuck in proposed forever".
- The `cortex_query` `decision_lookup` intent surfaces
  superseded + still-proposed ADRs with the same weight (every
  hit is `status=proposed`), eroding ranking quality.

This runbook describes the manual promotion path. CI gating is
optional and tracked at the bottom.

## Manual promotion path

1. Pick the target ADR (`DEC-NNN-some-slug`). The full id is the
   directory name under `.rulebook/decisions/`.

2. Run the Rulebook MCP tool:

   ```
   rulebook_decision_update --decision_id DEC-NNN-some-slug \
       --status accepted
   ```

   This rewrites the front-matter `status:` line in the source
   `.md` file. The next consolidator / fulltext re-projection
   picks the new value up and stamps `decision_status: accepted`
   onto the per-repo Meili doc.

3. (Optional) Add a brief acceptance note explaining the trigger:

   ```
   rulebook_decision_update --decision_id DEC-NNN \
       --status accepted \
       --note "Accepted after phase20 §7 shipped the rulebook-shape \
               fallback. Verified live via cortex_law_violations."
   ```

4. Confirm the projection landed:

   ```sh
   curl -sS -X POST http://127.0.0.1:17000/v1/decisions/search \
       -H 'content-type: application/json' \
       -d '{"q":"","status":"accepted","repo":"cortex","limit":5}'
   ```

   The response should include the promoted ADR.

## Supersession path

An ADR that REPLACES a prior one uses `superseded` on the older
ADR and stamps `supersedes: <old-id>` on the new ADR:

```
rulebook_decision_update --decision_id DEC-OLD --status superseded
# Then create DEC-NEW carrying `supersedes: DEC-OLD` in the frontmatter
```

The chain is walkable via `cortex_decision_chain --event_id <id>`.

## Deprecation path

An ADR that no longer applies (without a replacement) gets
`status: deprecated`:

```
rulebook_decision_update --decision_id DEC-NNN --status deprecated
```

## When to promote

| trigger                                               | promote to |
|-------------------------------------------------------|------------|
| A feature commit references the ADR by id             | `accepted` |
| A spec doc in `docs/specs/` cites the ADR rationale   | `accepted` |
| Two or more downstream phases rely on its invariant   | `accepted` |
| A newer ADR replaces it (`supersedes:` filled)        | `superseded` |
| The constraint the ADR encodes no longer applies      | `deprecated` |
| The proposed ADR is older than 30 days with no triage | review     |

## Dashboard hook (phase20 §8.2 future work)

A dashboard view "Proposed ADRs older than 30 days" lives at
`/v1/dashboard/decisions/stale` (TODO — not yet shipped). It
queries `cortex_decisions` filtered on `decision_status =
"proposed" AND ts < now() - 30d` and surfaces stuck ADRs for
triage. Phase20 §8.2 is documented as a future enhancement but
intentionally not auto-shipped; the manual cadence above keeps
the workflow honest until the dashboard slot is implemented.

## CI rule (phase20 §8.3 optional)

A GitHub Actions check that scans commit messages for
`DEC-\d{3}` references and prompts for ADR promotion when the
referenced ADR is still `proposed` would close the gap
automatically. Optional because:

- The trigger is high-noise (every commit referencing the ADR
  body would fire) without intent classification.
- The manual cadence + dashboard surfacing already covers the
  signal.

If a future operator wants this, the implementation is one
workflow file scanning `git log` since the last tag for
`DEC-\d{3}` and cross-referencing `rulebook_decision_show`.

## Related files

- `.rulebook/decisions/*.md` — source ADR docs.
- `crates/cortex-api/src/search/decision_search.rs` — read-side
  filter axis (`decision_status` is already filterable).
- `crates/cortex-workers/src/fulltext/builders.rs` —
  `apply_top_level_projection(Kind::Decision)` — stamps
  `decision_status` onto the Meili doc.
