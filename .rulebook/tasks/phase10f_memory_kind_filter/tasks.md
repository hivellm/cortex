## 1. Backend
- [ ] 1.1 In `crates/cortex-api/src/dashboard.rs`, the `memory` handler reads `?kind=<canonical>` (repeated) and `?facets=<kind>` (alias)
- [ ] 1.2 Filter the lane projection by kind before pagination so `limit=80` returns 80 of the requested kind, not 80 mixed
- [ ] 1.3 Return `400 unknown_kind` when an unknown value lands; the canonical set is `turn|tool_call|agent_call|memory|decision|analysis|law_violation|knowledge|learning`

## 2. GUI
- [ ] 2.1 In `gui/src/views/Memory.tsx`, render a chip row for every canonical kind above the search input
- [ ] 2.2 Click toggles `?kind=<chip>` in the active filter; multi-select ORs
- [ ] 2.3 Default empty selection means "all kinds" (current behaviour)
- [ ] 2.4 `gui/src/lib/api.ts` accepts `kinds: string[]` parameter

## 3. Tests
- [ ] 3.1 Unit test: `?kind=decision` returns only `kind=decision` rows from a seeded lane
- [ ] 3.2 Unit test: unknown kind returns 400 with the structured body
- [ ] 3.3 Vitest smoke for the new chip row in Memory.tsx

## 4. Spec / docs
- [ ] 4.1 Update `docs/specs/16-dashboard.md` §"Memory browser" with the kind filter

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior
- [ ] 5.3 Run tests and confirm they pass
