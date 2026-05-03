# TML repo bootstrap excludes audit — 2026-05-03

> Phase11p §3 — read-only audit. Output is a recommended `cortex.toml` to upstream into the `hivellm/tml` repo.

## Why

Live Meili stack on 2026-05-03 reports `cortex-tml-code` carrying **189,872 documents** — 65 % of the entire Cortex corpus. TML is a single-product compiler / VM repo; that volume is a clear bootstrap leak.

## Findings

`../Tml/` carries **no `cortex.toml`**. The bootstrap walker falls back to defaults (no excludes beyond the global `target/` / `node_modules/` baseline). TML's filesystem reveals the leak:

| Path | Size | File count | Nature |
|---|---|---|---|
| `src/llvm-project/` | **5.5 G** | tens of thousands of `.cpp/.h` | **Vendored entire LLVM source tree** |
| `src/gcc/` | **1.1 G** | tens of thousands of `.cpp/.h` | **Vendored entire GCC source tree** |
| `build/` | **5.4 G** | CMake outputs | Build artefacts (`.o`, generated headers) |
| `vcpkg_installed/` | 95 M | third-party libs | vcpkg installed packages |
| `src/tracy/` | 21 M | Tracy profiler source | Vendored profiler |
| `vscode-tml/` | 450 M | editor extension assets | Should be opt-in only |

Confirmed file count: `find ../Tml/src -type f` → **322,075 files**, of which **57,958 are `.cpp/.h/.hpp`**.

The cortex bootstrap walker honours `.gitignore` first (which TML's `.gitignore` likely covers `build/` already) and then per-repo `cortex.toml` excludes. Without `cortex.toml`, the walker happily indexed the LLVM + GCC + Tracy vendor trees as TML's own code. That's the 189k row source.

## Recommendation

Add the file `cortex.toml` at the **TML repo root** with the following body, then re-bootstrap TML with `cortex-bootstrap --force --estimate-only` to confirm the doc count drops as expected before flipping to `--apply`.

```toml
# Cortex bootstrap configuration for the TML repo.
# Phase11p audit (Cortex-side, 2026-05-03) — addresses the 189k
# cortex-tml-code bloat caused by vendored toolchain trees.

[cortex]
id = "Tml"

[cortex.exclude]
paths = [
    # Vendored toolchains — these are upstream LLVM / GCC source
    # trees pulled in for cross-compilation; not TML's own code
    # and never something the agent would recall about TML.
    "src/llvm-project/",
    "src/gcc/",
    "src/tracy/",
    "src/vcpkg/",

    # Build outputs.
    "build/",
    "out/",

    # Package manager state.
    "vcpkg_installed/",
    "vcpkg/",

    # Editor extension distribution bundles.
    "vscode-tml/dist/",
    "vscode-tml/node_modules/",

    # Generated / intermediate.
    ".cache/",
    "*.tmp",
]
extensions = [
    # Build / lock / binary leftovers that the global exclusion
    # set might miss on a Windows checkout.
    "obj",
    "pdb",
    "ilk",
    "exp",
    "lib",
    "dll",
    "exe",
    "pyc",
]

[cortex.git]
# TML's git history is huge (gcc / llvm vendoring history); cap
# the bootstrap walk to commits within the last year so the turn
# corpus doesn't pull in a decade of unrelated commits.
include_commits = true
include_prs = false
since = "2025-05-01"
```

Expected impact:
- `cortex-tml-code`: **189,872 → ~5,000-10,000** (TML's own `compiler/`, `compiler-tml/`, `tools/`, `samples/`).
- `cortex-tml-docs`: 14,349 → ~1,000-2,000 (drop vendored `docs/` from LLVM/GCC).
- Total Meili doc reduction across the corpus: **~190,000 docs**, ~65 % of today's total.

### Verified pre-state via `cortex-bootstrap --estimate`

Captured 2026-05-03 against the current TML checkout (no `cortex.toml` present):

```
Repo: Tml
  Files (after excludes):     327,309
  Files dropped:                  817
  Est. events:                700,371
  Est. classifier tokens (in/out): 311,790,753 / 245,129,850
  Est. embedding storage:       2.9 GB
  Est. fulltext index:          2.1 GB
  Est. one-time runtime:        1,401 s
```

Of the 327k surfaced files, **~322k live under the four vendored trees** (`src/llvm-project/`, `src/gcc/`, `src/tracy/`, `src/vcpkg/` — verified via `find ../Tml/src -type f | wc -l = 322,075`). Excluding those four paths is expected to drop the file count to **~5,000** and the estimated events to **~12,000** — a ~98 % reduction on TML alone. Re-run the estimate after the upstream `cortex.toml` PR lands to confirm.

## Action

1. Open an issue in the upstream `hivellm/tml` repo titled "Add `cortex.toml` to exclude vendored LLVM / GCC source trees from Cortex bootstrap" linking to this audit.
2. Attach the recommended `cortex.toml` body verbatim.
3. Once the upstream PR merges, run from the host:
   ```sh
   cd ../Tml
   cortex-bootstrap --force --estimate-only
   ```
   Confirm the predicted drop. Then run without `--estimate-only` to materialise.
4. Optional follow-up: drop the orphaned `cortex-tml-{code,docs}` Meili indexes after the re-bootstrap completes (the new write replaces the old document keys, but stale rows for paths the new walker no longer surfaces will linger).

## DO NOT modify TML's `cortex.toml` from the Cortex repo

This audit is read-only here. The `cortex.toml` recommendation is upstream-PR territory because TML owns its repo configuration; pushing the file from the Cortex side would split ownership of the same file across two repos.
