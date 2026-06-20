# Proposal: phase28_manual-e2e-feature-verification

## Why

Features have accumulated across many sessions (issue#4 fixes, model-name
in timeline, classifier idle, lru bump, phase17 reranker + phantom-link
verifier, phase27a edge confidence, plus the whole standing surface)
without a single end-to-end pass confirming the running stack is actually
functional. Unit tests pass in isolation, but the deployed containers
have lagged the committed code, so nobody has verified the real system
behaves correctly feature-by-feature. This task is a living, executable
checklist: every feature gets one concrete manual probe (command +
expected result) run against the live docker stack, checked off only when
it actually works.

## What Changes

- No production code. This is a verification harness: `tasks.md` enumerates
  every feature area as a checklist of concrete manual probes (curl / MCP
  tool / docker / cargo) with the expected result inline.
- Each item is checked `[x]` only after it passes against the live stack;
  failures are recorded inline with the actual output and converted into a
  follow-up rulebook bug task (never left as a silent unchecked item).
- Before running the API/graph/timeline probes, the relevant images are
  rebuilt + recreated so the running container matches committed HEAD
  (the gap that let features accumulate unverified).

## Impact

- Affected specs: none modified — this exercises specs 01–36.
- Affected code: none (verification only; any defect found spawns a
  separate fix task).
- Breaking change: NO.
- User benefit: a trustworthy, repeatable "is the app actually working"
  pass, so features stop accumulating without real-world confirmation.
