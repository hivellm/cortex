# Adapters

## ADDED Requirements

### Requirement: OpenCode and Cursor sessions are captured through the same pipeline as Claude Code

Sessions initiated through OpenCode or Cursor SHALL be captured via the
shared `EnvelopeProducer` trait and SHALL become retrievable through
`cortex_query` / `cortex_pre_thinking` with the same fidelity as Claude
Code sessions.

#### Scenario: OpenCode/Cursor session results returned with Claude-Code fidelity

Given a real OpenCode (or Cursor) session produces several tool calls and a decision
When a subsequent `cortex_query` with `intent: similar_problems` is issued for a related topic
Then results from that OpenCode/Cursor session MUST be returned exactly as a Claude Code session's results would be
