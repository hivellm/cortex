# Proposal: phase33_adapter-opencode-cursor

## Why

Source: docs/analysis/cortex-platform-2026-07/README.md (see
execution-plan.md, "Better agent workflows" section,
`phase33_adapter-opencode-cursor`).

Goal G1 in `docs/architecture.md` ("Capture 100% of AI interactions...
across every supported AI tool") is false today. The 2026-07-05 platform
re-scope analysis lists adapter coverage as half-built, "Claude-Code-only
in practice." Tracing what that means for OpenCode specifically (the
initial framing for this task assumed `cortex-adapter-opencode` was
still a placeholder; that assumption did not survive reading the code):

- `crates/cortex-adapter-opencode/` (the `EnvelopeProducer` impl per
  ADR-010) and `packages/cortex-opencode-plugin/` (the TS plugin per
  ADR-017) are code-complete, not placeholders. Both shipped in
  `phase16a_opencode-adapter-via-envelope-producer` (archived
  2026-06-10) with green unit tests (Rust producer tests + 18 TS
  tests), and `docs/specs/23-opencode-adapter.md` already carries
  status "Implemented."
- What never happened is live end-to-end verification. phase16a's
  §4.4/§6.1-6.3 (run a real OpenCode session; confirm envelopes with
  `tool: "opencode"` land in Synap; confirm the pre-thinking bundle
  reaches the model) were blocked on an operator-run OpenCode session.
  The follow-up task (`phase16b_opencode-smoke-validation`, archived
  2026-06-22) closed those items **WON'T-DO** by explicit operator
  decision: "OpenCode is deprioritized — the operator works in Claude
  Code." That closure note records the consequence directly: "OpenCode
  adapter parity (phase16a) remains UNVALIDATED end-to-end," and
  `phase11w_opencode-adapter` was deliberately left OPEN rather than
  archived because its archival gate (confirmed parity) never fired.
- Cursor has no adapter at all — no `crates/cortex-adapter-cursor/`
  exists anywhere in the workspace. `docs/specs/17-additional-adapters.md`
  (status: Draft) already specifies its design: file-watcher based
  (Cursor exposes no `PreToolUse`-equivalent hook), reusing the same
  `EnvelopeProducer` contract via a `cortex-adapters/common/`
  extraction — but none of it is built.

Net effect matches the analysis's framing even though the OpenCode
sub-claim needed correcting: in practice only Claude Code sessions are
captured today, including this project's own multi-agent Team workflows
if they ever route through a non-Claude-Code tool. This task closes what
phase16a/phase16b left open for OpenCode (live-verify the existing
implementation, fixing whatever the live run finds) and builds Cursor
for the first time, making Goal G1 true for two of the four tools spec
17 names. Codex and Gemini stay out of scope — a separate follow-up
task, not claimed here.

**Flag for whoever picks this up:** item 1 below effectively revisits
the 2026-06-22 operator decision to deprioritize OpenCode. That decision
was reasonable given the operator's daily-driver tool at the time; this
task exists because the 2026-07-05 re-scope raises the priority of
capture breadth. If OpenCode still is not in active use when this task
starts, confirm the live session (item 3) is still wanted before
spending the time on it.

## What Changes

1. Confirm and finish `cortex-adapter-opencode` per ADR-017's design (TS
   plugin + shared Rust daemon, `EnvelopeProducer` trait from ADR-010).
   Since the crate and plugin already satisfy this design at the code
   level (phase16a), this item is a review pass plus fixing whatever
   the live run in item 3 surfaces — not a rewrite of a "placeholder"
   that does not exist.
2. Implement a Cursor adapter per spec 17, built on the shared
   `EnvelopeProducer` trait for consistency with the OpenCode and
   Claude Code adapters. This is new work end-to-end: a filesystem
   watcher on `.cursor/rules/*.md` / `.cursor/chat/*.jsonl`, edit
   inference via workspace filesystem watch (tagged `edit_inferred`,
   kept distinguishable from directly-observed tool calls),
   pre-thinking injection via a rewritten `_cortex_context.md`, and
   observational-only governance (no blocking laws — Cursor has no
   synchronous hook to block on).
3. Live-verify both: run a real OpenCode session and a real Cursor
   session, confirm their events (prompts, tool calls) reach the same
   ingestion pipeline, get classified/embedded/indexed the same way,
   and become retrievable via `cortex_query` / `cortex_pre_thinking`
   exactly like Claude Code sessions do. For OpenCode this closes
   phase16a §4.4/§6.1-6.3 and re-opens phase16b's WON'T-DO validation;
   for Cursor this is the adapter's first live test.
4. Update `docs/architecture.md`'s capability table and Goal G1's
   status once verified. Do not overclaim "100%" — Codex and Gemini
   (also named in spec 17) remain unbuilt; track that as a separate
   follow-up task, not in this one's scope.

## Impact

- Affected specs: `docs/specs/23-opencode-adapter.md` (add the
  live-verified confirmation once item 3 closes),
  `docs/specs/17-additional-adapters.md` (Cursor's acceptance-criteria
  subset), new spec delta `adapters` (this task).
- Affected code: `crates/cortex-adapter-opencode/` and
  `packages/cortex-opencode-plugin/` (fixes only, if the live run
  surfaces a bug — no rewrite expected), a new
  `crates/cortex-adapter-cursor/` crate plus install/uninstall
  scripts, and a `cortex-adapters/common/` extraction if spec 17's
  shared crate does not already exist for the Cursor crate to build
  on.
- Breaking change: NO — purely additive capture surface.
- User benefit: Cortex's memory becomes genuinely tool-agnostic instead
  of Claude-Code-only, which matters directly for a team that might use
  multiple AI tools across the HiveLLM ecosystem.
