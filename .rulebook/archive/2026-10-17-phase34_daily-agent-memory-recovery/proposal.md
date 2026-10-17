# Proposal: phase34_daily-agent-memory-recovery

## Why
The operator reports no practical benefit despite sustained resource use. Verified capture, retrieval, delivery and durability gaps prevent agents remembering decisions and avoiding repeated errors.

## What Changes
One bounded recovery task: protect native memories, restore durable cross-agent recall, add candidate-only Jev classification, selective ai-memory patterns, and a measured lightweight daily runtime. User explicitly authorized this ONE task before older backlog on 2026-10-16; old tasks execute only if necessary for this objective.

## Impact
- Affected specs: 12 pre-thinking, 20 MCP, storage and runtime contracts
- Affected code: cortex-storage, cortex-api, cortex-pre-thinking, cortex-mcp-server, selected workers/adapters, compose and operator docs
- Compatibility: memory APIs remain additive; archive projection now requires explicit opt-in instead of starting from an archive root alone.
- User benefit: Correct decisions and verified lessons arrive in later sessions with sources, bounded latency and reduced idle resource use.
