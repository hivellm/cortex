> CLOSED WON'T-DO (operator decision 2026-06-22): every item below is a
> live OpenCode smoke validation that requires the operator to run an
> OpenCode session. OpenCode is deprioritized — the operator works in
> Claude Code — so this validation will not be run. Items are marked
> `[x]` to allow archival, NOT because they were performed. CONSEQUENCE:
> OpenCode adapter parity (phase16a) remains UNVALIDATED end-to-end, and
> `phase11w_opencode-adapter` stays OPEN (its §2.1 archival gate never
> fired). Re-open this validation if OpenCode adoption resumes.

## 1. Operator-led smoke validation (carry-over from phase16a §4.4 + §6)
- [x] 1.1 WON'T-DO — needs a live OpenCode session (operator); OpenCode deprioritized, not run.
- [x] 1.2 WON'T-DO — needs a live OpenCode session (operator); OpenCode deprioritized, not run.
- [x] 1.3 WON'T-DO — needs a live OpenCode session (operator); OpenCode deprioritized, not run.
- [x] 1.4 WON'T-DO — needs a live OpenCode session (operator); OpenCode deprioritized, not run.

## 2. Post-smoke close-out
- [x] 2.1 WON'T-DO — parity (§1.2–§1.4) never confirmed, so `phase11w_opencode-adapter` is intentionally left OPEN, not archived.

## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation — N/A: validation-only task closed won't-do; the closure note above is the only documentation.
- [x] 99.2 Write tests covering the new behavior — N/A: no behavior changed (no code touched).
- [x] 99.3 Run tests and confirm they pass — N/A: no code changed; nothing to test.
