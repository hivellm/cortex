# Spec: cortex-claude-archive ingestor

## ADDED Requirements

### Requirement: New crate ingests `~/.claude/projects/` JSONL transcripts

The system SHALL provide a `cortex-claude-archive` crate that walks
the Claude Code conversation archive and emits canonical Cortex
`Envelope` records onto the existing ingestion bus, without
modifying the upstream archive on disk.

#### Scenario: bootstrap subcommand ingests the full corpus

Given the user runs `cortex-claude-archive bootstrap --root C:/Users/Bolado/.claude/projects/ --sink synap`
When the binary completes
Then it MUST emit one `Envelope` with `kind=Turn` per matched user↔assistant pair
And it MUST emit one `Envelope` with `kind=ToolCall` per `assistant.tool_use` block paired with its `attachment.tool_result`
And it MUST emit one `Envelope` with `kind=AgentCall` for every `assistant.tool_use` whose `tool_name` is `Agent`
And every emitted envelope MUST have `tool="claude-code"` and `stream=Bootstrap`
And every emitted envelope MUST pass `cortex_core::validate_event`
And the binary MUST exit with status 0 on a clean run

#### Scenario: tail subcommand watches live sessions

Given the user runs `cortex-claude-archive tail --root C:/Users/Bolado/.claude/projects/`
When a Claude Code session writes a new record to a session JSONL file
Then the binary MUST detect the write within 2 s
And MUST emit the corresponding envelope to the configured sink
And MUST checkpoint `(session_id, last_record_uuid)` within 5 s of the emission

#### Scenario: estimate subcommand reports without emitting

Given the user runs `cortex-claude-archive estimate --root C:/Users/Bolado/.claude/projects/`
When the binary completes
Then it MUST report `files_total`, `bytes_total`, `envelopes_estimated` to stdout
And it MUST NOT publish to Synap
And it MUST NOT write to the archive root

#### Scenario: resume from checkpoint

Given a previous bootstrap was interrupted with a checkpoint at session S, last_record_uuid U
When the user re-runs `cortex-claude-archive bootstrap --resume`
Then the binary MUST skip every record ≤ U in session S
And MUST resume emission with the next record after U

#### Scenario: redaction is mandatory

Given a session JSONL contains a string matching `sk-ant-[A-Za-z0-9_-]{20,}`
When the binary maps it to an envelope
Then the envelope's payload MUST NOT contain the secret
And the envelope's `redactions` field MUST contain at least one entry referencing the redaction kind

#### Scenario: corrupt records do not panic

Given a session JSONL contains a malformed line (truncated JSON)
When the binary processes the file
Then it MUST log a warning naming the file + line number
And it MUST increment an `envelopes_dropped` metric
And it MUST continue processing subsequent records in the same file

### Requirement: No new `Kind` variant is introduced

The conversation archive SHALL be representable using the existing
`Kind::{Turn, ToolCall, AgentCall, Memory, Artifact}` variants and
the existing per-payload schemas.

#### Scenario: schema validation passes for every envelope shape

Given the test fixture set covering all 7 JSONL record types
When `cortex-claude-archive bootstrap --sink archive` writes envelopes
And the test reads each envelope back through `cortex_core::Envelope::deserialize`
Then deserialization MUST succeed for every emitted envelope
And `Envelope::kind` MUST be one of `Turn`, `ToolCall`, `AgentCall`, `Memory`, `Artifact`

## MODIFIED Requirements

### Requirement: Classifier routes claude-code bootstrap kinds

The classifier worker SHALL recognise the bootstrap kind strings
emitted by `cortex-claude-archive` and MUST route them to the
canonical `Kind::{Turn, ToolCall, AgentCall}` variants.

#### Scenario: kind_from_bootstrap accepts new strings

Given the classifier worker receives an envelope on `cortex.events.bootstrap` with `kind="turn.claude-code"`
When `kind_from_bootstrap("turn.claude-code")` runs
Then it MUST return `Ok(Kind::Turn)`
And the equivalent calls for `"tool_call.claude-code"` and `"agent_call.claude-code"` MUST return `Ok(Kind::ToolCall)` and `Ok(Kind::AgentCall)` respectively
