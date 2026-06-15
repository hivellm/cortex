## 1. Operator-led smoke validation (carry-over from phase16a §4.4 + §6)
- [ ] 1.1 Spot-check 3 commands + 3 agents inside a live OpenCode session (phase16a §4.4).
- [ ] 1.2 Run an OpenCode session: prompt + 2 tool calls + 1 subagent; confirm plugin fires on all lifecycle events (phase16a §6.1).
- [ ] 1.3 Synap `cortex.events.raw` shows ≥4 envelopes with `tool: "opencode"` (phase16a §6.2).
- [ ] 1.4 Audit envelope shows the pre-thinking bundle reached the model (phase16a §6.3).

## 2. Post-smoke close-out
- [ ] 2.1 Archive task `phase11w_opencode-adapter` once §1.2–§1.4 confirm parity (phase16a §7.4).

## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
