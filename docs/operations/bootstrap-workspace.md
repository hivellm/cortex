# Workspace bootstrap (phase4g runbook)

[`cortex-bootstrap`](../../crates/cortex-bootstrap) populates Vectorizer
+ Meilisearch + Nexus from a snapshot of every HiveLLM checkout in one
invocation. This runbook closes the gap audited on 2026-04-27 (3/17
repos covered) by walking the workspace template against the operator's
local stack.

The orchestrator code is feature-complete from
[phase4b](../../.rulebook/tasks/phase4b_bootstrap_resume_remaining_repos/proposal.md):
the `--workspace` flag, pre-flight verifier, per-repo checkpoint, and
estimate / live modes are already shipped. This runbook just authors
the workspace TOML and runs the binary.

## Prerequisites

| Requirement | Notes |
|---|---|
| `cortex-bootstrap` on `PATH` | Built from the current `main` after the phase4b merge. |
| All 17 HiveLLM repos cloned under `$HIVE_ROOT` | The exact directory layout matches the canonical repo names — see [`bootstrap.workspace.toml.example`](../../bootstrap.workspace.toml.example) for the full list. |
| Each repo has a `cortex.toml` at its root | The pre-flight verifier rejects missing config files up front. Keep the file source-controlled in each repo. |
| Vectorizer / Meilisearch / Nexus / Synap reachable | Same env vars the live workers use (`CORTEX_VECTORIZER_URL`, `CORTEX_FULLTEXT_MEILI_URL`, `CORTEX_NEXUS_URL`, `CORTEX_SYNAP_URL`). |

## Steps

### 1. Clone every repo under `$HIVE_ROOT`

The operator's machine layout (`E:/HiveLLM/` on Windows) becomes the
`HIVE_ROOT` we substitute into the workspace TOML. Confirm every repo
in [`bootstrap.workspace.toml.example`](../../bootstrap.workspace.toml.example)
exists as a git checkout under that root before continuing.

### 2. Author `bootstrap.workspace.toml`

Copy the template, then replace the literal `${HIVE_ROOT}` placeholder
with the local checkout root. The example file deliberately keeps the
placeholder verbatim so a single search-and-replace suffices.

```sh
cp bootstrap.workspace.toml.example bootstrap.workspace.toml
sed -i 's#${HIVE_ROOT}#E:/HiveLLM#g' bootstrap.workspace.toml
```

PowerShell equivalent:

```powershell
(Get-Content bootstrap.workspace.toml.example) `
    -replace '\$\{HIVE_ROOT\}', 'E:/HiveLLM' `
  | Set-Content bootstrap.workspace.toml
```

### 3. Estimate the work

`--estimate` runs the same scan the full bootstrap does but stops
before publishing to Synap. The output sizes the upcoming run so the
operator can carve out the right window.

```sh
cortex-bootstrap --workspace bootstrap.workspace.toml --estimate
```

If pre-flight fails, every offender is printed at once via
`WorkspaceError::Preflight`. Fix all of them in one pass before
re-running — the verifier will not move past the first failure.

### 4. Run the live walk

Drop `--estimate` to publish into Synap. The orchestrator iterates
the entries in declaration order, persists per-repo progress to
`.cortex-bootstrap.state.json`, and skips already-completed repos when
re-running, so a mid-run interruption resumes cleanly.

```sh
cortex-bootstrap --workspace bootstrap.workspace.toml
```

The summary table at the end lists every repo with its outcome
(`completed` / `skipped` / `failed`) and counters. A non-zero exit
means at least one entry failed; the table identifies which.

### 5. Verify in the live stack

Three independent backends should be populated. Run each query and
expect at least one row per repo.

**Vectorizer — collections per repo:**

```sh
curl -s "${CORTEX_VECTORIZER_URL}/v1/collections" \
  | jq -r '.collections[].name' \
  | sort | uniq
```

The expected shape is one of `code` / `docs` (per the spec-06
collection family) suffixed with the repo slug.

**Meilisearch — per-(repo, family) indexes:**

```sh
curl -s "${CORTEX_FULLTEXT_MEILI_URL}/indexes" \
  -H "Authorization: Bearer ${CORTEX_FULLTEXT_MEILI_API_KEY}" \
  | jq -r '.results[].uid' \
  | grep '^cortex-' | sort
```

The expected shape is `cortex-{repo_slug}-{family}` for every repo
that produced events. The phase4f boot-replay defense closes any
post-boot gaps; the bootstrap is the green-field path.

**Nexus — Repo nodes:**

```cypher
MATCH (r:Repo) RETURN r.name ORDER BY r.name;
```

Expected: 17 rows, one per `[[repo]].id` in the workspace TOML.

If a repo is missing from one backend but present in the others,
re-run `cortex-bootstrap` for just that repo — the per-repo
checkpoint covers Vectorizer / Meili / Nexus together, so a single
re-run aligns all three.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `WorkspaceError::Preflight` listing every repo | The `${HIVE_ROOT}` placeholder was never replaced. Re-run the `sed` / `Set-Content` command. |
| `WorkspaceError::Preflight` for a single repo | The repo is not a git checkout, or its `cortex.toml` is missing. Add the missing file and re-run; the orchestrator skips already-completed entries. |
| Vectorizer collections present but Meili indexes empty | The fulltext worker hadn't started during the bootstrap. Either start it now and let it catch up via Synap, or run the [graph-symbol replay](graph-backfill.md) sibling routine. |
| Re-running re-walks a completed repo | `.cortex-bootstrap.state.json` was deleted. Restore it from a recent backup; otherwise the orchestrator re-walks every entry on the next run. |

## Re-running

The orchestrator is idempotent end-to-end. Vectorizer / Meili / Nexus
all key on content hashes derived from the canonical envelope, and
the per-repo checkpoint stops the scan before producing duplicate
events. Re-running on a clean machine after the first successful run
is a no-op.
