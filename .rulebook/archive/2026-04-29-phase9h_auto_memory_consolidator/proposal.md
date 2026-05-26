# Proposal: phase9h_auto_memory_consolidator

## Why

Claude Code's auto-memory writes one Markdown file per memory under
`~/.claude/projects/<project>/memory/*.md` plus an index `MEMORY.md`.
Today the Cortex project has 9 entries; the policy is "save anything
non-obvious", so this set grows linearly and tends to accumulate
near-duplicates (`feedback_no_summaries_no_questions.md`,
`feedback_be_autonomous.md`, `feedback_dont_blame_hive_services.md`
are three files saying overlapping things, all valid but not as
distinct entries).

Beyond N≈30 entries the index runs past Claude's 200-line truncation
budget, defeating the purpose of the file. We need a consolidator that
treats the auto-memory directory like any other Cortex memory store:
embed each entry, cluster by similarity, ask Sonnet to merge clusters
into denser memories, archive the originals.

## What Changes

1. NEW subcommand `cortex memory consolidate <project>` (lives in
   `crates/cortex-ops/`, surfaces through `bin/cortex.bat`).
2. Discovery: locates
   `~/.claude/projects/<project-slug>/memory/{MEMORY.md,*.md}`. If the
   slug is omitted, derives it from the current working tree the same
   way Claude Code does (replace `:` and `/` with `--`).
3. For each `*.md` (excluding `MEMORY.md`):
   - read frontmatter (`name`, `description`, `type`),
   - embed the body via the embedder worker,
   - cluster with a cosine threshold (default 0.78) within the same
     `type`,
   - clusters of size ≥ 2 are sent to Sonnet with a prompt that:
     "produce one memory entry preserving every concrete instruction,
     with deduplicated rationale; keep the strictest constraint when
     two entries conflict",
   - the new entry replaces the cluster: a fresh `<consolidated_<n>.md`
     is written, originals move to `memory/_archive/<timestamp>/`.
4. `MEMORY.md` is regenerated at the end from the surviving files'
   frontmatter (one line each, capped at 150 chars per the auto-memory
   rules).
5. Default mode is `--dry-run`; `--apply` is required to actually
   touch the filesystem. Dry-run prints a diff: clusters found, new
   bodies that would be written, files that would be archived.
6. Idempotent: re-running `--apply` after a successful run finds no
   clusters of size ≥ 2 and exits clean.
7. Conflict guard: if Sonnet's merged entry's similarity to any cluster
   member drops below 0.6, abort the merge for that cluster and keep
   originals (prevents an over-eager merge that loses content).

## Impact

- Affected specs: NEW `docs/specs/19-retention.md` §"Auto-memory
  consolidator".
- Affected code: NEW `crates/cortex-ops/src/memory_consolidate.rs`,
  small additions in the embedder client wrapper, prompt template in
  `cortex-classifier`.
- Breaking change: NO. The auto-memory directory layout is preserved;
  only file count shrinks.
- User benefit: keeps Claude Code's auto-memory under the 200-line
  truncation cap; deduplicates feedback entries that say the same
  thing in three voices.
