"""
One-shot — walks the parquet archive, groups envelopes by session_id,
prints the top sessions ranked by (turn_count, user_msg_chars,
assistant_msg_chars) so the operator can pick a substantive session
for manual consolidation testing.
"""
import io, json, os, sys
from collections import defaultdict
from pathlib import Path
import zstandard as zstd

HOME = Path.home() / ".cortex" / "archive" / "events"

def iter_envelopes(path):
    raw = path.read_bytes()
    d = zstd.ZstdDecompressor()
    try:
        body = d.decompress(raw)
    except zstd.ZstdError:
        body = d.stream_reader(io.BytesIO(raw)).read()
    for line in body.split(b"\n"):
        if not line.strip():
            continue
        try:
            yield json.loads(line)
        except json.JSONDecodeError:
            continue

def main():
    sessions = defaultdict(lambda: {
        "turns": 0, "tool_calls": 0, "agent_calls": 0,
        "user_chars": 0, "asst_chars": 0,
        "repos": set(), "first_ts": None, "last_ts": None,
        "ids": [],
    })
    for p in sorted(HOME.rglob("*.parquet")):
        for env in iter_envelopes(p):
            sid = env.get("session_id")
            if not sid: continue
            kind = env.get("kind", "")
            s = sessions[sid]
            ts = env.get("occurred_at", "")
            if s["first_ts"] is None or ts < s["first_ts"]:
                s["first_ts"] = ts
            if s["last_ts"] is None or ts > s["last_ts"]:
                s["last_ts"] = ts
            ctx_repo = (env.get("context") or {}).get("repo")
            if ctx_repo:
                s["repos"].add(ctx_repo)
            if kind == "turn":
                payload = env.get("payload", {})
                u = (payload.get("user_message") or "").strip()
                a = (payload.get("assistant_message") or "").strip()
                if u or a:
                    s["turns"] += 1
                    s["user_chars"] += len(u)
                    s["asst_chars"] += len(a)
                    s["ids"].append(env["event_id"])
            elif kind == "tool_call":
                s["tool_calls"] += 1
                s["ids"].append(env["event_id"])
            elif kind == "agent_call":
                s["agent_calls"] += 1
                s["ids"].append(env["event_id"])

    # Rank by total content chars
    ranked = sorted(
        sessions.items(),
        key=lambda kv: (kv[1]["user_chars"] + kv[1]["asst_chars"], kv[1]["turns"]),
        reverse=True,
    )
    print(f"sessions with any usable content: {sum(1 for _,s in ranked if s['turns']>0 or s['tool_calls']>=3)}")
    print()
    print(f"{'session_id':<28} {'turns':>5} {'tools':>5} {'agents':>6} {'chars':>8} {'repos':<25} {'first-last'}")
    for sid, s in ranked[:15]:
        if s["turns"] == 0 and s["tool_calls"] == 0:
            continue
        chars = s["user_chars"] + s["asst_chars"]
        repos = ",".join(sorted(s["repos"])[:3])[:25]
        print(f"{sid:<28} {s['turns']:>5} {s['tool_calls']:>5} {s['agent_calls']:>6} {chars:>8} {repos:<25} {s['first_ts']} → {s['last_ts']}")

if __name__ == "__main__":
    main()
