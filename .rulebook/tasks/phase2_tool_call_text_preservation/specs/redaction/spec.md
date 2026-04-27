# Tool-call text preservation spec

## MODIFIED Requirements

### Requirement: Redaction is field-level, not payload-level
The system SHALL redact secrets at the value level. Replacing the entire `input` JSON object with `{}` is forbidden — structural fields (`command`, `file_path`, `pattern`, `tool_name`, `description`) MUST survive redaction so retrieval can match on them.

#### Scenario: bash command with no secrets passes through verbatim
Given a `Bash` tool_call with `input.command = "git status"`
When the redactor runs
Then the post-redaction `input.command` MUST equal `"git status"` exactly
And the lane-facing text MUST equal `"[Bash] git status"`

#### Scenario: bash command with a secret has the secret masked, command preserved
Given a `Bash` tool_call with `input.command = "AWS_SECRET_ACCESS_KEY=AKIA… aws s3 ls"`
When the redactor runs
Then the post-redaction `input.command` MUST contain `"aws s3 ls"`
And the post-redaction `input.command` MUST NOT contain the literal AKIA token
And `envelope.redactions` MUST contain a trace entry pointing at `input.command`

#### Scenario: long Write content becomes hash + preview
Given a `Write` tool_call with `input.content` of 50 KB
When the redactor runs
Then `input.content` MUST be replaced with `{ "sha256": "...", "preview": "<first non-empty line>" }`
And the lane-facing text MUST contain the file path and the preview line, not the 50 KB body

## ADDED Requirements

### Requirement: Lane-facing text composition for tool_calls
The classifier (or a downstream helper) SHALL produce a lane-facing `text` field for every tool_call envelope, composed of `"[<tool_name>] <salient-fields>"` where `<salient-fields>` includes the post-redaction structural fields appropriate to the tool.

#### Scenario: Edit tool composition
Given an `Edit` tool_call with `file_path = "crates/x/foo.rs"`, `old_string = "..."`, `new_string = "..."`
When the lane-facing text is built
Then it MUST equal `"[Edit] crates/x/foo.rs"` followed by a short summary of the change (e.g. SHA-of-old → SHA-of-new) — never `"[Edit] {}"`

### Requirement: Backfill of existing archive
The system SHALL provide a one-shot tool that re-runs the new redactor over the existing `~/.cortex/archive/` parquet files and re-seeds the keyword lane, without losing the raw envelopes.

#### Scenario: replay produces non-empty text on legacy archive
Given the archive at `~/.cortex/archive/` was captured before this change
When `cortex-ingestion replay --redact-fix` runs
Then the keyword lane MUST be reseeded with text derived from the new builder
And a probe `/v1/query` for a known historical command MUST return at least one hit referencing it
