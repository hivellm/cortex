# Narrative docs (README/architecture) silently drift years behind actual shipped state

**Category**: code
**Tags**: cortex, docs, analysis:cortex-platform-2026-07

## Description

README.md claimed "Draft v0.1, no spec has flipped to green, no runnable binary in master" while in reality 13+ of 18 core specs were implemented, the live Docker stack had 12 services running continuously for 8-13 days, and the MCP server exposed 37 tools. Unlike code, narrative docs have no test suite to catch staleness — nothing fails CI when a README's factual claims stop matching reality, so the gap grows silently and compounds (found ~2 years of drift in this case).

## When to Use

When a project has fast task/phase cadence and top-level docs (README, architecture doc, spec index) that assert factual claims about what's implemented.
