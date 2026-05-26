# Adapter sync paths must use the cortex-pre-thinking pipeline, not a bespoke HTTP client
**Source**: manual
**Date**: 2026-04-27
**Related Task**: phase1_adapter_pre_thinking_contract_fix
**Tags**: pre-thinking, claude-code, spec-10, spec-12, adapter, contract-bug
Symptom: every UserPromptSubmit hook returned `{}` to Claude Code; pre-thinking enrichment never reached the model even though every cortex daemon was up and the archive was being written.

Root cause: cortex-adapter-claude-code shipped its own minimal `PreThinkingRequest { prompt, session_id, cwd, max_bundle_bytes }` and POSTed to `/v1/query?intent=pre_change_context`. cortex-api expects `Json<QueryRequest> { intent, query, ... }` and ignores URL query strings, so every call returned 422 and the adapter fail-opened with an empty `additional_context`. On top of that, the response struct serialized as snake_case (`additional_context`, `permission_decision`) when Claude Code's hook contract requires camelCase under `hookSpecificOutput.additionalContext`.

Lesson: when an internal crate already implements the orchestration (here `cortex_pre_thinking::pipeline::run`), wire the adapter through it. Don't reach across the architectural boundary with a parallel HTTP client — the request/response shapes drift the moment one side changes.

How verified: stopped PID 93544 (`~/.cargo/bin/cortex-adapter-claude.exe`), `cargo build --release -p cortex-adapter-claude-code`, replaced the binary, restarted. Manually piped a `UserPromptSubmit` frame to the named pipe and got back `{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"<!-- cortex: pre_change_context · query_id=... -->\n\n## Relevant snippets (5)\n..."}}` — keyword lane is hitting archived turns and the bundle is the right Markdown shape.

Action items not in scope of this task: live VectorLane (Vectorizer SDK) and live GraphLane (Nexus SDK) are still `MemoryVectorLane`/`MemoryGraphLane` test-doubles in `crates/cortex-api/src/main.rs:40-42` — split into a follow-up task.