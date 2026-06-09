<!-- RULEBOOK:START v5.3.0 — DO NOT EDIT BY HAND. Regenerated on `rulebook update`.
     Put project-specific content in AGENTS.override.md or CLAUDE.local.md.
     Anything outside the RULEBOOK:START/END sentinels is preserved across updates. -->

# CLAUDE.md

This project is managed by [@hivehub/rulebook](https://github.com/hivellm/rulebook).
The authoritative rules come from the imports below. Claude Code loads all of them
automatically at session start (see [Anthropic memory docs](https://code.claude.com/docs/en/memory#claude-md-imports)).

## Project identity & live state
@.rulebook/STATE.md

## Core standards (team-shared, versioned)
@AGENTS.md

## Project-specific overrides (user-owned, survives `rulebook update`)
@AGENTS.override.md

## Session scratchpad (human notes)
@.rulebook/PLANS.md

## Critical rules (highest precedence — apply on every turn)

1. **Read `AGENTS.md` and `AGENTS.override.md`** before making changes. These contain project-specific conventions that override generic guidance.
2. **Never revert or discard uncommitted work** — fix forward. Treat the working tree as sacred; investigate before destructive operations.
3. **Edit files sequentially**, not in parallel. When a task touches 3+ files, decompose into 1–2 file sub-tasks.
4. **Run `check`/type-check before `test`** — diagnostic-first. Cheap diagnostics catch issues that expensive test suites miss or take longer to surface.
5. **If a fix fails twice, escalate** — stop, research, or open a team. Do not retry the same approach a third time.
6. **Prefer MCP tools** (`mcp__rulebook__*` and project-specific MCP servers) over shell commands when the equivalent tool exists.
7. **Capture learnings**: at the end of significant work, save patterns and anti-patterns to `.rulebook/knowledge/` and insights to `.rulebook/learnings/`.
8. **Never archive a task** without docs updated, tests written, and tests passing — the task tail enforces this structurally.

## Delegation & parallelism (highest precedence — apply on every turn)

**Default behavior: delegate, don't do it yourself. Parallelize, don't serialize. Create new agents/skills when the gap is real.**

1. **Delegate by default.** If a step matches an agent in the delegation table, dispatch it via `Agent` instead of doing it inline. Implementation → `implementer` (sonnet). Research / read-only exploration → `researcher` (haiku). Tests → `tester`. Docs → `docs-writer` (haiku). Architecture / cross-cutting → `architect` (opus). Reserve the main conversation for orchestration + decisions.
2. **Parallelize independent work.** When a turn requires multiple independent investigations or edits, dispatch every independent piece in **a single message with multiple `Agent` tool-use blocks**. Sequential `Agent` calls are a smell — every time you catch yourself writing "first X, then Y", check whether the two halves are independent.
3. **Use Teams for multi-specialist work.** Anything that needs ≥2 background agents to coordinate MUST go through a Team (`TeamCreate` + `team_name` on dispatch). Standalone background `Agent` calls without `team_name` are blocked by the enforcement hook.
4. **Create skills + agents when the gap is real.** If you write the same multi-step instructions twice in one session, lift it into a skill (`templates/skills/<category>/<name>/SKILL.md`). If a class of work repeats across projects, create an agent definition under `.claude/agents/`. Default to creating, not improvising.
5. **Foreground vs background.** Use foreground `Agent` when you need the result to inform your next step. Use background only with `team_name` so messages can flow.

## Editing discipline (Karpathy-inspired)

Behavioral guidelines that reduce common LLM coding mistakes. Adapted from [forrestchang/andrej-karpathy-skills](https://github.com/forrestchang/andrej-karpathy-skills), grounded in [Andrej Karpathy's observations](https://x.com/karpathy/status/2015883857489522876).

1. **Think before coding.** State assumptions explicitly. If multiple interpretations exist, present them — don't pick silently. If a simpler approach exists, say so. If something is unclear, stop and ask. Don't hide confusion.
2. **Simplicity first.** Minimum code that solves the problem. No features beyond what was asked, no abstractions for single-use code, no "flexibility" that wasn't requested, no error handling for impossible scenarios. If you write 200 lines and 50 would do, rewrite.
3. **Surgical changes.** Touch only what you must. Don't "improve" adjacent code, comments, or formatting. Don't refactor things that aren't broken. Match existing style. If you notice unrelated dead code, mention it — don't delete it. Every changed line must trace directly to the user's request.
4. **Goal-driven execution.** Define verifiable success criteria upfront. "Add validation" → "write tests for invalid inputs, then make them pass." For multi-step tasks, state a brief plan: `[step] → verify: [check]`. Strong criteria let you loop independently; weak criteria require constant clarification.

## Session continuity

- **Start of session**: read `.rulebook/PLANS.md` and call `rulebook_session_start` to load prior context.
- **End of session**: `rulebook_session_end` writes a summary to `.rulebook/PLANS.md`.

## Knowledge base

Before implementing anything non-trivial:

- `rulebook_knowledge_list` — check existing patterns and anti-patterns.
- `rulebook_learn_list` — review past learnings.
- `rulebook_decision_list` — review architectural decisions.

After implementing, capture at least one entry per task:

- `rulebook_knowledge_add` for reusable patterns or anti-patterns to avoid.
- `rulebook_learn_capture` for implementation insights that don't belong in code comments.
- `rulebook_decision_create` for significant architectural choices.

## Task workflow

**MANDATORY: ALWAYS use the Rulebook MCP tools for task management.** Never create task directories or files manually — use `rulebook_task_create`, `rulebook_task_update`, `rulebook_task_archive`, `rulebook_task_list`, `rulebook_task_show`, `rulebook_task_validate`. These tools enforce naming conventions, mandatory tail items, phase structure, and metadata that manual file creation skips.

1. `rulebook_task_list` to see pending work.
2. `rulebook_task_create` to create new tasks — **never `mkdir` + `Write` manually**.
3. Pick the **first unchecked item from the lowest-numbered phase** — never reorder.
4. Read the task's `proposal.md` and `tasks.md` before touching code.
5. Implement step by step. Run lint + type-check after each significant change.
6. `rulebook_task_update` to change task status as you progress.
7. Mark items `[x]` in `tasks.md` as you finish them.
8. The mandatory tail (docs + tests + verify) is **not optional** — `rulebook_task_archive` will refuse to close the task otherwise.

<!-- RULEBOOK:END -->

## Agentes, Teams e paralelismo (project-specific — sobrevive a `rulebook update`)

**Use agentes e Teams agressivamente. O default é delegar e paralelizar, não executar tudo no main thread.**

### Quando delegar a um agente (não fazer no main)

Spawn um agente sempre que um destes for verdade:

- **Pesquisa/exploração que vai gastar >3 queries** → `Explore` ou `researcher` (haiku). Protege o contexto principal.
- **Implementação de >50 linhas em 1+ arquivo** → `implementer` (sonnet). Main coordena, agente escreve.
- **Escrever testes** → `tester` (sonnet).
- **Code review pós-implementação** → `code-reviewer` ou `feature-dev:code-reviewer`.
- **Decisão arquitetural / ADR** → `architect` (opus).
- **Build/CI quebrado** → `build-engineer`.
- **Auditoria de segurança / deps** → `security-reviewer` (haiku).
- **Refactor que toca padrões repetidos** → `refactoring-agent`.
- **Migração (DB / API / framework)** → `migration-engineer`.
- **Performance profiling** → `performance-engineer`.

Regra prática: se a tarefa não cabe em ≤2 ferramentas + uma resposta curta, **delega**.

### Paralelismo é mandatório para trabalho independente

- Múltiplas leituras / greps / globs independentes → **uma única mensagem com várias tool calls em paralelo**, nunca em sequência.
- Múltiplos agentes em frentes ortogonais → spawn no mesmo turno (uma mensagem, vários blocos `Agent`).
- Pesquisa + implementação de feature diferente → paralelo.
- Type-check + test + lint independentes → paralelo.

Se você se pegar fazendo tool calls em série quando poderiam ser paralelas, está errado.

### Teams para trabalho multi-agente em background

Trabalho de fundo com 2+ agentes **DEVE** usar Team (`team_name`). Background `Agent` standalone não consegue `SendMessage` — viola a regra `multi-agent-teams.md` e o hook bloqueia.

Padrão: `team-lead` orquestra → `researcher` (haiku) + `implementer` (sonnet) + `tester` (sonnet). Spawn todos no mesmo turno com `team_name` setado.

### Skills e agentes custom

Sempre que identificar um padrão recorrente que valha automatizar, **proponha criar uma skill ou agente**:

- Workflow que repete em ≥3 sessões → skill em `.claude/skills/`.
- Tarefa especializada com prompt longo + ferramentas restritas → agente em `.claude/agents/`.
- Cron / loop / sweep → `/schedule` ou `/loop`.

Não pergunte permissão para sugerir — sugira concretamente (nome, escopo, gatilhos) ao final do trabalho relevante.

### Anti-padrões (não fazer)

- Main agent escrevendo 300 linhas de código direto sem delegar.
- Tool calls em série quando são independentes.
- Reusar o main para pesquisa exploratória extensa (queima contexto).
- Background `Agent` sem `team_name` (será bloqueado).
- "Vou eu mesmo, é mais rápido" — quase sempre não é, e custa contexto.
