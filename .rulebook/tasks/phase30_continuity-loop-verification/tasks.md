## 1. Verification and wiring
- [ ] 1.1 Write an integration test that (a) runs a session that produces a consolidation (session, topic, or decision-trace grain), (b) starts a fresh session scoped to the same repo, (c) calls `cortex_pre_thinking` with a query related to the first session's content, (d) asserts the prior session's consolidation appears in the returned bundle
- [ ] 1.2 If 1.1 fails, root-cause exactly where the loop breaks (embedding lag, wrong collection queried, pre-thinking renderer filtering it out, scope mismatch, etc.) and record the finding here — do not fix it inline; if the root cause needs a code fix beyond the active-work wiring in 1.3, create a separate follow-up Rulebook task for that fix before this task is archived
- [ ] 1.3 Wire `cortex_active_work` (or equivalent) into the session-start flow (the adapter's `SessionStart` hook and/or the pre-thinking bundle assembly) so a new session automatically surfaces prior in-flight Rulebook tasks without the agent having to call the tool explicitly
- [ ] 1.4 Document the verified (or newly-fixed) continuity loop as an operator runbook under `docs/cortex/` — what to check if a user reports "the agent forgot what we just did"

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 2.1 Update or create documentation covering the implementation — specifically `docs/specs/12-pre-thinking-injection.md` and/or `docs/specs/10-claude-code-adapter.md`, describing the session-start active-work wiring added in 1.3
- [ ] 2.2 Write tests covering the new behavior — the session-start wiring added in 1.3, separate from the cross-session consolidation test in 1.1
- [ ] 2.3 Run tests and confirm they pass
