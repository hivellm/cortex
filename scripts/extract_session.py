"""Extract every Turn user_message + assistant_message from one session."""
import io, json, sys
from pathlib import Path
import zstandard as zstd

SID = sys.argv[1] if len(sys.argv) > 1 else "0MEZ8TKQMEXJEFCXAQE9ZSF1D8"
HOME = Path.home() / ".cortex" / "archive" / "events"

def iter_envelopes(path):
    raw = path.read_bytes()
    d = zstd.ZstdDecompressor()
    try:
        body = d.decompress(raw)
    except zstd.ZstdError:
        body = d.stream_reader(io.BytesIO(raw)).read()
    for line in body.split(b"\n"):
        if not line.strip(): continue
        try: yield json.loads(line)
        except: continue

turns = []
tools = []
event_ids = []
for p in sorted(HOME.rglob("*.parquet")):
    for env in iter_envelopes(p):
        if env.get("session_id") != SID: continue
        kind = env.get("kind", "")
        event_ids.append(env["event_id"])
        if kind == "turn":
            payload = env.get("payload", {})
            u = (payload.get("user_message") or "").strip()
            a = (payload.get("assistant_message") or "").strip()
            if u or a:
                turns.append((env["occurred_at"], env["event_id"], u, a))
        elif kind == "tool_call":
            payload = env.get("payload", {})
            tools.append((env["occurred_at"], env["event_id"], payload.get("tool_name",""), str(payload.get("input",""))[:100]))

turns.sort()
tools.sort()
print(f"session: {SID}")
print(f"total event_ids: {len(event_ids)}")
print(f"turns: {len(turns)}")
print(f"tool_calls: {len(tools)}")
print(f"first turn: {turns[0][0]}, last turn: {turns[-1][0]}")
print()
print("=== TURNS (chronological) ===")
for ts, eid, u, a in turns:
    print(f"--- {ts} | {eid} ---")
    if u: print(f"USER: {u}")
    if a: print(f"ASST: {a[:1500]}")
    print()
print()
print("=== TOOL CALLS (first 30) ===")
for ts, eid, tn, ipt in tools[:30]:
    print(f"  {ts} {tn:20} {ipt[:80]}")
print(f"  ... ({len(tools)-30} more)" if len(tools)>30 else "")
