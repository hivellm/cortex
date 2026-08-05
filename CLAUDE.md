<!-- RULEBOOK:START v7.0.0 — DO NOT EDIT BY HAND. Regenerated on `rulebook update`.
     Put project-specific content in AGENTS.override.md or CLAUDE.local.md.
     Anything outside the RULEBOOK:START/END sentinels is preserved across updates. -->

# CLAUDE.md

Managed by [@hivehub/rulebook](https://github.com/hivellm/rulebook) — few rules, all deliberate.

## Project-specific overrides (user-owned, survives `rulebook update`, wins on conflict)
@AGENTS.override.md

## Commands
- Before each commit: type-check + lint + the tests covering what changed.
- Before push / PR / task archive: the FULL quality gate (type-check → lint →
  full test suite), all green. Never bypass hooks — what the project wired
  into pre-commit/pre-push is the floor.
- Diagnostic-first: run the type-checker before the test suite; it is the faster signal.

## Values
1. Complete implementations — no stubs, no TODO markers left behind; finish, or say concretely why you can't.
2. Root causes, not workarounds — diagnose before changing code; never guess at bug causes.
3. Surgical diffs — touch only what the task needs; match existing style.
4. Simplicity first — the least code that solves the problem; no unrequested abstractions or features.
5. Fix forward — never discard uncommitted work.
6. State assumptions — if interpretations diverge, say so instead of picking silently.

## Git safety (requires explicit user authorization)
`reset --hard` · `checkout -- .` / `restore .` · `clean -f` · `push --force` ·
`rebase` on shared branches · `stash` · `branch -D` · switching a shared checkout
with changes you did not author. Yours autonomously: status/diff/log/add/commit,
branches you create (create/switch/merge), `git worktree`, PRs via `gh`.

## Orchestration
Subagents, parallel dispatch, and teams are your call — fan out freely when work is
parallel or context-heavy; work directly when it isn't. Rulebook never blocks or
mandates orchestration.

## Rulebook (on demand — no ceremony for small fixes)
- Multi-session or multi-phase work: track via the `rulebook` MCP (`rulebook_task`).
  Checklist order = dependencies; independent items may run in parallel.
- Optional session context: `rulebook_session`. Learned something non-obvious?
  `rulebook_memory`.
- Project specs live in `.rulebook/specs/` — read a spec when the work touches its area.
- Analyses live in `docs/analysis/<slug>/` — numbered files, one theme per file.
- Long session? `/compact <focus>` at a task boundary (~60% context). After
  `rulebook_task {action:"archive"}`, `/clear` is free — state lives in `.rulebook/`.

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
