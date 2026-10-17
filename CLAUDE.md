<!-- RULEBOOK:START v7.0.0 — generated; put custom content outside this block. -->

# CLAUDE.md

Managed by @hivehub/rulebook — few rules, all deliberate.

## Project overrides (user-owned, survive updates, win on conflict)
@AGENTS.override.md

## Commands
- Before each commit: type-check + lint + tests covering the change.
- Before push/PR/task archive: FULL quality gate (type-check → lint → full
  suite), all green. Never bypass hooks — pre-commit/pre-push wiring is the floor.
- Type-check before tests; it is the faster signal.
- No OS schedules (cron, systemd, launchd, schtasks): scheduling lives in the app.

## Values
1. Complete implementations — no stubs or TODO markers; finish, or say concretely why you can't.
2. Root causes, not workarounds — diagnose before changing code; never guess at bug causes.
3. Surgical diffs — touch only what the task needs; match existing style.
4. Simplicity first — the least code that solves the problem; nothing unrequested.
5. Fix forward — never discard uncommitted work.
6. State assumptions — if interpretations diverge, say so instead of picking silently.

## Communication
- Plain words over jargon; gloss a term when it isn't obvious from context.
- Answer first, reasoning after.
- Cut what the reader knows and options you didn't take. Length follows
  the result, not the effort behind it.

## Git safety (requires explicit user authorization)
`reset --hard` · `checkout -- .` / `restore .` · `clean -f` · `push --force` ·
`rebase` on shared branches · `stash` · `branch -D` · switching a shared checkout
with changes you did not author. Yours: status/diff/log/add/commit,
branches you create (create/switch/merge), `git worktree`, PRs via `gh`.
Worktrees live outside the repo tree and come out via `git worktree remove` —
never `rm -rf` a worktree, never delete or move a `.git`.

## Orchestration
Do small, local work yourself (files in context, ~3 or fewer to change). Delegate 2+ independent parts with disjoint files (in parallel), broad exploration, long work, or past ~150k context — for context and parallelism, not speed; a spawn costs ~20–40k tokens. Brief fully (files, decisions); ask back outcome, files changed, checks run, open risks. Prefer narrow agents to general-purpose; one progress line per dispatch and hand-back. Model per call: Fable 5.1 architecture, hard bugs, review; Opus 5.5 edits, tests, docs; Haiku 4.5 research.
CHANGELOG per PR/release, not per task. See `.rulebook/specs/orchestration.md`.

## Rulebook (on demand — no ceremony for small fixes)
- Multi-session or multi-phase work: track via the `rulebook` MCP (`rulebook_task`).
  Checklist order = dependencies; independent items may run in parallel.
- Undecidable choice? `rulebook_task {action:"ask"}`, show the form. Same request
  again? Capture a learning under one title; at 2+ promote it to a skill.
- Optional: `rulebook_session` (context), `rulebook_memory` (learnings).
- Read a `.rulebook/specs/` spec when work touches its area.
- Analyses: `docs/analysis/<slug>/`, numbered files, one theme each.
- `/compact <focus>` at a task boundary (~60% context); after an archive
  `/clear` is free — state lives in `.rulebook/`.

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
