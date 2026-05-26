# Spec: Auto-memory consolidator

## ADDED Requirements

### Requirement: Cluster auto-memory by semantic similarity

`cortex memory consolidate` MUST embed every `*.md` (excluding
`MEMORY.md`) in `~/.claude/projects/<project-slug>/memory/`, group files
by their declared `type`, and cluster them within each group using
cosine similarity with a configurable threshold (default 0.78).

#### Scenario: three near-duplicate feedback entries cluster together
Given three feedback files whose bodies are mutually ≥ 0.85 cosine
When the consolidator runs in dry-run mode
Then it MUST report exactly one cluster covering the three files
And it MUST NOT cluster files of different `type` together.

### Requirement: Sonnet-driven merge with conflict guard

For every cluster of size ≥ 2 the consolidator MUST request a single
merged memory from the classifier model, then re-embed the merged body
and compare it to every source body. If any source-to-merged cosine
drops below 0.6 the cluster MUST be left intact.

#### Scenario: drifted merge is rejected
Given a cluster of two files whose merged body diverges (cosine 0.55
  against one of them)
When the merge step runs
Then the merge MUST be discarded
And the original files MUST remain in `memory/`
And the run MUST report the cluster as `skipped: drift`.

### Requirement: Default dry-run

The default mode MUST be `--dry-run`. The filesystem MUST NOT be
modified unless `--apply` is supplied. Dry-run output MUST list
clusters, the proposed merged frontmatter, and the files that would be
archived.

#### Scenario: dry-run leaves the directory untouched
Given a memory directory with 9 files
When the consolidator runs without `--apply`
Then the directory MUST still contain exactly 9 files afterwards.

### Requirement: Archive originals before replacing

When `--apply` succeeds for a cluster, source files MUST be moved into
`memory/_archive/<RFC3339>/` (preserving original filenames) before
the merged file is written. The merged file MUST be named
`consolidated_<short-hash>.md`.

#### Scenario: applied merge archives originals
Given a successful merge of three files
When `--apply` is supplied
Then `memory/_archive/<timestamp>/` MUST contain the three source files
And `memory/` MUST contain the new `consolidated_<hash>.md`
And `memory/` MUST NOT contain the three original files.

### Requirement: Regenerated index

After a successful run, `MEMORY.md` MUST be regenerated from the
surviving files' frontmatter — one line per entry, each line ≤150
chars, no YAML frontmatter on the index file itself.

#### Scenario: post-run index reflects only survivors
Given a run that reduces 9 files to 6
When `--apply` finishes
Then `MEMORY.md` MUST contain exactly 6 lines.

### Requirement: Idempotence

A second `--apply` immediately after a successful run MUST find no
clusters of size ≥ 2 and MUST exit reporting zero merges.
