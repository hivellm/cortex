"""Probe the event archive: count envelopes per (repo, family) tuple."""
import collections
import glob
import json
import zstandard

CODE_EXT = {"rs","ts","tsx","js","jsx","mjs","cjs","py","go","rb","java","kt","scala","c","cc","cpp","h","hpp","cs","swift","php","lua","sh","bash","zsh","ps1","fish","sql","proto"}
DOC_EXT = {"md","mdx","markdown","rst","adoc","asciidoc","txt","rtf","tex","org"}

def family_for(env):
    kind = env.get("kind") or ""
    if kind == "tool_call":  return "code"
    if kind == "decision":   return "decisions"
    if kind in ("turn", "agent_call"): return "turns"
    if kind == "law_violation": return "governance"
    if kind == "analysis":   return "analyses"
    if kind == "memory":     return "misc"
    if kind == "artifact":
        ctx = env.get("context") or {}
        path = (ctx.get("path") or "") + ""
        ext = path.rsplit(".",1)[-1].lower() if "." in path else ""
        if ext in CODE_EXT: return "code"
        if ext in DOC_EXT:  return "docs"
        topics = ((env.get("classifier") or {}).get("topics") or [])
        if "code" in topics: return "code"
        if "doc" in topics or "documentation" in topics: return "docs"
        return "misc"
    return "misc"

def slug_for_repo(repo: str) -> str:
    if not repo: return "unknown"
    s = repo.lower()
    out = []
    for c in s:
        if c.isalnum(): out.append(c)
        elif c in ("-","_"): out.append(c)
        else: out.append("-")
    s = "".join(out).strip("-")
    return s or "unknown"

paths = sorted(glob.glob("C:/Users/Bolado/.cortex/archive/events/**/raw-*.parquet", recursive=True))
counts = collections.Counter()
total = 0
parsed = 0
for path in paths:
    with open(path, "rb") as f:
        try:
            data = zstandard.ZstdDecompressor().stream_reader(f).read().decode("utf-8","replace")
        except Exception:
            continue
    for line in data.splitlines():
        total += 1
        try: env = json.loads(line)
        except: continue
        parsed += 1
        repo = ((env.get("context") or {}).get("repo") or "")
        slug = slug_for_repo(repo)
        family = family_for(env)
        counts[(slug, family)] += 1

print(f"files: {len(paths)}  lines: {total}  parsed: {parsed}")
print(f"\n(repo_slug, family) -> count:")
for (slug, fam), n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
    print(f"  {slug:20s} {fam:12s} {n:7d}   -> cortex-{slug}-{fam}")
