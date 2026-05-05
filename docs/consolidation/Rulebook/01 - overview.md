# Rulebook Overview

## Purpose

Rulebook is a tool-agnostic AI development framework that standardizes AI-assisted coding across 23 tools (Claude Code, Cursor, Gemini, Copilot, Windsurf, etc.). It solves inconsistent, error-prone AI output by giving every tool the same rules, quality gates, and task structure.

## What It Does

**One-command setup**:
```bash
npx @hivehub/rulebook@latest init
```

Auto-detects 28 languages, 17 frameworks, 13 MCP modules, 20 services, and configures:
- Modular rule system (`AGENTS.md` + `AGENTS.override.md` + path-scoped `.claude/rules/`)
- Quality gates (lint, type-check, format, tests, coverage via git hooks)
- MCP integration (44+ tools for task, memory, skills, decisions, knowledge)
- Session continuity (persistent memory, handoff at context limits, live status)
- Autonomous loop (Ralph: multi-iteration AI task solver with fresh context per iteration)

## Role in HiveLLM

Rulebook is the **rule engine and task/memory backbone** for HiveLLM projects. Every project imports from Rulebook:
- Cortex uses Rulebook for task management, persistent memory, and agent coordination
- Other HiveLLM services (Vectorizer, Nexus, Synap, etc.) use Rulebook for consistent AI rules
- Projects auto-detect themselves via Rulebook and tailor rules per language/framework

## Core Principles

1. **Modular rules** — base rules regenerate on `update`; project overrides survive
2. **Structural enforcement** — `PreToolUse` hooks block forbidden patterns before edits reach disk
3. **Persistent memory** — hybrid BM25+vector search across sessions
4. **Autonomous solving** — Ralph loop: quality gates, learning extraction, pause/resume
5. **No shortcuts** — complete implementations only; stubs/TODOs forbidden

## Supported Stack

- **28 languages**: TypeScript, Rust, Python, Go, Java, C/C++, etc.
- **17 frameworks**: NestJS, Django, React, Rails, Angular, etc.
- **20 services**: PostgreSQL, MongoDB, Redis, Neo4j, S3, etc.
- **13 MCP modules**: Vectorizer, Synap, Context7, GitHub, Playwright, Memory, etc.
- **23 AI tools**: Cursor, Windsurf, VS Code, GitHub Copilot, Claude Code, etc.

## Version

Current: **5.5.2** (2026-05-04)
- Published as npm package: `@hivehub/rulebook`
- Embedded in every HiveLLM project via `.rulebook/` directory
