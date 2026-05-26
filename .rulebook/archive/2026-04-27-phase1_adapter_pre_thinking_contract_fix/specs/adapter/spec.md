# Adapter pre-thinking contract spec

## MODIFIED Requirements

### Requirement: UserPromptSubmit sync path uses cortex-pre-thinking pipeline
The `cortex-adapter-claude-code` daemon SHALL drive its `UserPromptSubmit` synchronous path through the `cortex_pre_thinking::pipeline::run` pipeline. The adapter MUST NOT POST a hand-rolled JSON body that disagrees with the `cortex_api::QueryRequest` shape.

#### Scenario: pipeline produces a non-empty bundle
Given the cortex-api daemon is running and `MemoryKeywordLane` has been seeded from the archive
And the adapter receives a `UserPromptSubmit` frame with a non-empty `prompt`
When the adapter dispatches the frame
Then the adapter MUST construct a `cortex_api::QueryRequest` with `intent` and `query` populated
And the adapter MUST POST that body verbatim to `/v1/query`
And the adapter MUST hand the parsed `QueryResponse` to `cortex_pre_thinking::pipeline::run`
And the adapter MUST return the resulting markdown `String` as `additionalContext`

#### Scenario: API call fails or times out
Given the cortex-api daemon is unreachable or returns a non-success status
When the adapter dispatches a `UserPromptSubmit` frame
Then the adapter MUST fail-open with an empty `HookResponse`
And the adapter MUST NOT panic or block past the configured timeout

### Requirement: Hook response uses Claude Code camelCase contract
The adapter SHALL serialize hook responses to match the Claude Code hook contract: `additionalContext` (string) MUST be nested under `hookSpecificOutput`, and `permissionDecision` / `permissionDecisionReason` MUST be camelCase top-level fields.

#### Scenario: UserPromptSubmit response with bundle
Given the pre-thinking pipeline produced a non-empty markdown `String`
When the adapter writes the hook response
Then the response JSON MUST contain `hookSpecificOutput.hookEventName = "UserPromptSubmit"`
And the response JSON MUST contain `hookSpecificOutput.additionalContext` as a string
And the response JSON MUST NOT contain `additional_context` (snake_case)

#### Scenario: PreToolUse deny response
Given the law-check returned `deny`
When the adapter writes the hook response
Then the response JSON MUST contain `permissionDecision = "deny"`
And the response JSON MUST contain `permissionDecisionReason` as a string
And the response JSON MUST NOT contain `permission_decision` (snake_case)

#### Scenario: empty response
Given the dispatcher chose to return `HookResponse::empty()`
When the adapter writes the hook response
Then the response JSON MUST be exactly `{}`
