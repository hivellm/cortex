#!/usr/bin/env python3
"""phase15c_archived-tasks-inventory-cleanup §1 — inventory pass.

Walk .rulebook/archive/*/proposal.md, extract the "Affected code:" paths
(brace-expanded), check each against the tracked-file set, and emit a CSV:
  task_id, status, affected_files, status_reason

status taxonomy (§1.4):
  still-live           — every resolved path is present in the working tree
  dead-code-candidate  — no resolved path is present (work's files are gone)
  partial-live         — some present, some gone (needs review)
  no-affected-code     — proposal carried no parseable code path

superseded-by-X / redundant are judgment overlays applied after this pass.
"""
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ARCHIVE = os.path.join(REPO, ".rulebook", "archive")


def tracked_files():
    out = subprocess.run(
        ["git", "ls-files"], cwd=REPO, capture_output=True, text=True, check=True
    ).stdout
    return set(p.strip().replace("\\", "/") for p in out.splitlines() if p.strip())


def brace_expand(s):
    """Expand a single level of {a,b,c}, recursing for nesting.
    `crates/x/{a.rs,b/{c,d}.rs}` -> [crates/x/a.rs, crates/x/b/c.rs, crates/x/b/d.rs]
    """
    i = s.find("{")
    if i == -1:
        return [s]
    # find matching close brace for the first open
    depth = 0
    j = i
    while j < len(s):
        if s[j] == "{":
            depth += 1
        elif s[j] == "}":
            depth -= 1
            if depth == 0:
                break
        j += 1
    if j >= len(s) or depth != 0:
        return [s]  # unbalanced — leave as-is
    pre, body, post = s[:i], s[i + 1 : j], s[j + 1 :]
    # split body on top-level commas
    parts, d, cur = [], 0, ""
    for ch in body:
        if ch == "{":
            d += 1
            cur += ch
        elif ch == "}":
            d -= 1
            cur += ch
        elif ch == "," and d == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += ch
    parts.append(cur)
    results = []
    for p in parts:
        for expanded in brace_expand(pre + p + post):
            results.append(expanded)
    return results


# path-ish token: starts with crates/ or docs/ or a few known roots, or contains a slash + dotted ext
PATHISH = re.compile(r"(?:crates|docs|packages|scripts|\.rulebook|\.claude|\.github)/[^\s`]*")


def extract_affected(proposal_path):
    """Return the list of backtick/inline code spans on the Affected-code line(s)."""
    spans = []
    with open(proposal_path, encoding="utf-8", errors="replace") as f:
        lines = f.readlines()
    grabbing = False
    for ln in lines:
        low = ln.lower()
        if "affected code" in low:
            grabbing = True
            payload = ln.split(":", 1)[1] if ":" in ln else ln
        elif grabbing and ln.strip().startswith("-"):
            grabbing = False
            continue
        elif grabbing and ln.strip() and not ln.startswith(" "):
            grabbing = False
            continue
        elif grabbing:
            payload = ln
        else:
            continue
        # backtick spans first
        bt = re.findall(r"`([^`]+)`", payload)
        if bt:
            spans.extend(bt)
        else:
            spans.extend(PATHISH.findall(payload))
    return spans


PREFIX = re.compile(r"^(crates|docs|packages|scripts|\.rulebook|\.claude|\.github)/")


def resolve_paths(spans):
    paths = []
    for span in spans:
        # Brace-expand the WHOLE span first so nested groups with internal
        # commas (`graph/{projection.rs,extractors/{a,b}.rs}`) expand
        # correctly; only AFTER that split the expansion on top-level
        # commas / whitespace into individual paths. Splitting first would
        # truncate a brace group at its inner comma -> unbalanced -> a live
        # file mis-read as gone.
        for expanded in brace_expand(span):
            for tok in re.split(r"[,\s]+", expanded):
                tok = tok.strip().strip("`").rstrip(".,;:)")
                if not tok or "{" in tok or "}" in tok:
                    continue
                if not PREFIX.match(tok):
                    continue
                paths.append(tok.replace("\\", "/").strip())
    # dedupe preserve order
    seen, out = set(), []
    for p in paths:
        if p not in seen:
            seen.add(p)
            out.append(p)
    return out


def present(path, files):
    path = path.rstrip("/")
    if "*" in path or "?" in path:
        # glob path (e.g. scripts/seed-*.sh, crates/cortex-graph/*) — match
        # against the tracked set so a relocated-but-present file still counts.
        import fnmatch

        return any(fnmatch.fnmatch(f, path) or fnmatch.fnmatch(f, path + "/*") for f in files)
    if path in files:
        return True
    # directory: any tracked file under it
    pref = path + "/"
    return any(f.startswith(pref) for f in files)


# Crate consolidations that happened after these tasks archived: the old
# top-level crate's code was relocated under cortex-workers / cortex-cli.
# A gone path under one of these crates is "superseded-by-restructure"
# (code relocated, nothing to delete) when the relocation target exists.
CONSOLIDATION = {
    "crates/cortex-graph/": "crates/cortex-workers/src/graph",
    "crates/cortex-retention/": "crates/cortex-workers/src/retention",
    "crates/cortex-fulltext/": "crates/cortex-workers/src/fulltext",
    "crates/cortex-classifier/": "crates/cortex-workers/src/classifier",
    "crates/cortex-ops/": "crates/cortex-cli/src/bin/cortex-ops",
    "crates/cortex-relevance-eval/": "crates/cortex-cli/src/relevance_eval",
}
# Old cortex-api query-lane files folded into cortex-api/src/search/.
API_LANE_FILES = {
    "meili_lane.rs",
    "vectorizer_lane.rs",
    "nexus_graph_lane.rs",
    "lane_contract.rs",
    "orchestrator.rs",
    "strategies.rs",
    "fusion.rs",
}


def superseded_reason(gone, files):
    reasons = []
    for p in gone:
        for old, new in CONSOLIDATION.items():
            if p.startswith(old) and present(new, files):
                reasons.append(f"{old.rstrip('/')} -> {new}")
                break
        else:
            base = p.rsplit("/", 1)[-1]
            if (
                p.startswith("crates/cortex-api/src/")
                and base in API_LANE_FILES
                and present("crates/cortex-api/src/search", files)
            ):
                reasons.append(f"{p} -> cortex-api/src/search/")
    if not reasons:
        return None
    # dedupe preserve order
    seen, out = set(), []
    for r in reasons:
        if r not in seen:
            seen.add(r)
            out.append(r)
    return "; ".join(out)


def orphaned_rs(files):
    """True-dead-code pass: tracked crates/*/src .rs files that no `mod`
    declaration references and that are not a compilation entrypoint.
    These are the only genuine deletion candidates (code present on disk
    but not in any module tree). A grep-based heuristic — `#[path=...]`
    includes can false-positive, so the output is a review list, not an
    auto-delete list."""
    ENTRY = {"main.rs", "lib.rs", "mod.rs", "build.rs"}
    rs = [
        f
        for f in files
        if f.startswith("crates/")
        and f.endswith(".rs")
        and "/src/" in f
        and "/tests/" not in f
        and "/benches/" not in f
    ]
    # collect every `mod <name>` token across the repo's rust sources
    declared = set()
    pathincl = []
    mod_re = re.compile(r"\bmod\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*[;{]")
    path_re = re.compile(r'#\[path\s*=\s*"([^"]+)"\]')
    for f in rs:
        try:
            with open(os.path.join(REPO, f), encoding="utf-8", errors="replace") as fh:
                txt = fh.read()
        except OSError:
            continue
        declared.update(mod_re.findall(txt))
        pathincl.extend(path_re.findall(txt))
    orphans = []
    for f in rs:
        base = f.rsplit("/", 1)[-1]
        stem = base[:-3]
        if base in ENTRY:
            continue
        if "/src/bin/" in f:  # bin entrypoints are auto-discovered
            continue
        if stem in declared:
            continue
        # referenced via an explicit #[path="...stem.rs"] include anywhere?
        if any(base in inc or stem in inc for inc in pathincl):
            continue
        orphans.append(f)
    return sorted(orphans)


def main():
    files = tracked_files()
    rows = []
    for d in sorted(os.listdir(ARCHIVE)):
        full = os.path.join(ARCHIVE, d)
        prop = os.path.join(full, "proposal.md")
        if not os.path.isdir(full) or not os.path.isfile(prop):
            continue
        # task_id = dir minus leading date prefix YYYY-MM-DD-
        task_id = re.sub(r"^\d{4}-\d{2}-\d{2}-", "", d)
        spans = extract_affected(prop)
        paths = resolve_paths(spans)
        if not paths:
            rows.append((task_id, "no-affected-code", "", "no parseable code path in proposal"))
            continue
        live = [p for p in paths if present(p, files)]
        gone = [p for p in paths if not present(p, files)]
        if not gone:
            status, reason = "still-live", f"{len(live)}/{len(paths)} present"
        elif not live:
            # All affected files are gone. Distinguish a crate
            # consolidation (code relocated, nothing to delete) from a
            # genuine removal.
            sup = superseded_reason(gone, files)
            if sup:
                status, reason = "superseded-by-restructure", sup
            else:
                status, reason = "dead-code-candidate", f"0/{len(paths)} present; all gone"
        else:
            status, reason = "partial-live", f"{len(live)}/{len(paths)} present; gone: {';'.join(gone[:5])}"
        rows.append((task_id, status, ";".join(paths), reason))

    # CSV out
    out_dir = os.path.join(REPO, "docs", "analysis", "rework", "opus5.7", "appendix")
    os.makedirs(out_dir, exist_ok=True)
    csv_path = os.path.join(out_dir, "archived-tasks-audit.csv")

    def esc(s):
        s = str(s)
        if '"' in s or "," in s or "\n" in s:
            return '"' + s.replace('"', '""') + '"'
        return s

    with open(csv_path, "w", encoding="utf-8", newline="") as f:
        f.write("task_id,status,affected_files,status_reason\n")
        for r in rows:
            f.write(",".join(esc(c) for c in r) + "\n")

    # ---- MD report (§2.2) ----
    from collections import Counter

    c = Counter(r[1] for r in rows)
    orphans = orphaned_rs(files)
    md_path = os.path.join(out_dir, "archived-tasks-audit.md")
    with open(md_path, "w", encoding="utf-8") as f:
        f.write("# Archived-tasks inventory audit\n\n")
        f.write(
            "_Generated by `scripts/archived_tasks_inventory.py` "
            "(phase15c_archived-tasks-inventory-cleanup §1–§2)._\n\n"
        )
        f.write(
            "Maps every archived task's proposal `Affected code:` paths to the "
            "current working tree.\n\n"
        )
        f.write("## Counts per status\n\n")
        f.write("| status | count |\n|---|---|\n")
        for k in sorted(c):
            f.write(f"| {k} | {c[k]} |\n")
        f.write(f"| **total** | **{len(rows)}** |\n\n")
        f.write("### Status taxonomy\n\n")
        f.write(
            "- **still-live** — every resolved affected path is present; the task's code is current.\n"
            "- **partial-live** — some affected paths present, some gone (mixed refactor / rename).\n"
            "- **superseded-by-restructure** — all affected paths gone, but the code was *relocated* by a crate consolidation (nothing to delete).\n"
            "- **dead-code-candidate** — all affected paths gone with no detected relocation (work already removed; informational).\n"
            "- **no-affected-code** — proposal carried no parseable current-layout path (mostly pre-`crates/` migration tasks).\n\n"
        )
        for status in ["dead-code-candidate", "superseded-by-restructure", "partial-live"]:
            sub = [r for r in rows if r[1] == status]
            if not sub:
                continue
            f.write(f"## {status} ({len(sub)})\n\n")
            f.write("| task_id | reason | affected_files |\n|---|---|---|\n")
            for tid, st, af, rsn in sub:
                af_short = af if len(af) < 90 else af[:87] + "…"
                f.write(f"| `{tid}` | {rsn} | {af_short} |\n")
            f.write("\n")
        f.write("## True deletion candidates — orphaned `.rs` files\n\n")
        f.write(
            "Tracked `crates/*/src/**/*.rs` files that **no `mod` declaration "
            "references** and that are not a compilation entrypoint (`main.rs` / "
            "`lib.rs` / `mod.rs` / `build.rs` / `src/bin/*`). These — not the "
            "already-gone files above — are the genuine dead-code candidates. "
            "Heuristic (grep for `mod <stem>`); `#[path=...]` includes are "
            "excluded but verify before deleting.\n\n"
        )
        if orphans:
            for o in orphans:
                f.write(f"- `{o}`\n")
        else:
            f.write("_None found — every `crates/*/src` module is referenced._\n")
        f.write("\n")

    print(f"wrote {csv_path}", file=sys.stderr)
    print(f"wrote {md_path}", file=sys.stderr)
    print(f"total tasks: {len(rows)}", file=sys.stderr)
    for k in sorted(c):
        print(f"  {k:26} {c[k]}", file=sys.stderr)
    print(f"orphaned .rs files: {len(orphans)}", file=sys.stderr)


if __name__ == "__main__":
    main()
