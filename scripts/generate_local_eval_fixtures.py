"""Materialize reviewed public sources without querying Cortex.

Requires zstandard only when authoring a new fixture version. The evaluation
runner consumes the committed compressed bytes and uses the standard library.
Existing output files are never overwritten.
"""

import argparse
import copy
import hashlib
import json
from pathlib import Path


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def digest(value):
    return hashlib.sha256(value).hexdigest()


def imported_id(kind, namespace, scope, original_id):
    return kind + "-" + digest(canonical([namespace, scope, original_id]).encode())


def materialize(corpus_bytes, namespace):
    import zstandard

    corpus = json.loads(corpus_bytes)
    if corpus["schema_version"] != 1 or corpus["license"] != "CC0-1.0":
        raise ValueError("Expected reviewed CC0 corpus schema 1")
    memories = {row["key"]: row for row in corpus["memories"]}
    if len(memories) != len(corpus["memories"]):
        raise ValueError("Duplicate memory key")
    identities = {
        key: imported_id(
            "daily", namespace, "repo:" + row["repository"], row["source_id"]
        )
        for key, row in memories.items()
    }
    records = {}
    daily_lines = []
    occurred_at = "2026-02-03T09:00:00Z"
    origin = {"attribution": "unknown"}
    for key, memory in memories.items():
        scope = "repo:" + memory["repository"]
        event_id = "fixture-evidence:" + key
        # DecisionPayload's serde field order is part of the legacy hash contract.
        payload = {
            "decision_id": memory["source_id"],
            "title": memory["title"],
            "status": memory["authority"],
            "body": memory["body"],
        }
        links = []
        if "supersedes_key" in memory:
            prior = memories[memory["supersedes_key"]]
            if prior["repository"] != memory["repository"]:
                raise ValueError("Cross-scope daily supersession")
            payload["supersedes"] = prior["source_id"]
            links.append(
                {"target_id": identities[prior["key"]], "relation": "supersedes"}
            )
        payload["tags"] = ["daily-memory", memory["kind"]]
        payload_bytes = json.dumps(
            payload, ensure_ascii=False, separators=(",", ":")
        ).encode()
        payload_hash = "sha256:" + digest(payload_bytes)
        evidence = {
            "event_id": event_id,
            "kind": "decision",
            "version": 1,
            "payload": payload,
            "origin": origin,
            "sources": [],
            "occurred_at": occurred_at,
            "stale": False,
        }
        evidence_raw = canonical(evidence)
        snapshot = {
            "id": memory["source_id"],
            "scope": {"repository": memory["repository"]},
            "title": memory["title"],
            "body": memory["body"],
            "category": memory["kind"],
            "status": memory["authority"],
            "origin": origin,
            "sources": [],
            "branch": None,
            "path": None,
            "pinned": False,
            "stale": False,
            "version": 1,
            "event_id": event_id,
            "captured_at": occurred_at,
            "content_hash": payload_hash,
            "supersedes": payload.get("supersedes"),
        }
        snapshot_raw = canonical(snapshot)
        daily_lines.append(
            canonical(
                {
                    "scope": scope,
                    "id": memory["source_id"],
                    "snapshot_raw": snapshot_raw,
                    "event_id": event_id,
                    "version": 1,
                    "evidence_raw": evidence_raw,
                }
            )
        )
        records[key] = {
            "id": identities[key],
            "repository": memory["repository"],
            "revision": 1,
            "kind": memory["kind"],
            "authority": memory["authority"],
            "title": memory["title"],
            "body": memory["body"],
            "links": links,
            "source_file": "daily.sqlite",
            "source": {
                "kind": "daily-sqlite",
                "source_id": event_id,
                "captured_at": occurred_at,
                "source_hash": digest(evidence_raw.encode()),
                "payload": {
                    "namespace": namespace,
                    "original_scope": scope,
                    "original_id": memory["source_id"],
                    "initial_evidence": evidence,
                    "initial_evidence_raw": evidence_raw,
                },
            },
            "snapshot_sha256": digest(snapshot_raw.encode()),
            "payload_hash": payload_hash,
        }
    event_lines = []
    for ordinal, event in enumerate(corpus["events"], 1):
        key = event["event_id"]
        if key in records:
            raise ValueError("Duplicate fixture key")
        line = canonical(event)
        repository = event["context"]["repo"]
        records[key] = {
            "id": imported_id(
                "archive", namespace, "repo:" + canonical(repository), key
            ),
            "repository": repository,
            "revision": 1,
            "kind": "event",
            "authority": "observation",
            "title": event["kind"] + ": " + key,
            "links": [],
            "source_file": "archive/events.ndjson.zst",
            "source_line": ordinal,
            "source": {
                "kind": "legacy_archive",
                "source_id": key,
                "captured_at": event["occurred_at"],
                "source_hash": digest(line.encode()),
                "payload": event,
            },
        }
        if event.get("parent_event_id"):
            records[key]["links"].append(
                {
                    "target_id": imported_id(
                        "archive",
                        namespace,
                        "repo:" + canonical(repository),
                        event["parent_event_id"],
                    ),
                    "relation": "derived_from",
                }
            )
        event_lines.append(line)
    graph = copy.deepcopy(corpus["graph_records"])
    for request in graph:
        key = request["id"]
        if key in records:
            raise ValueError("Duplicate fixture key")
        request["source"]["source_hash"] = digest(canonical(request).encode())
        records[key] = {
            field: request[field]
            for field in (
                "id",
                "repository",
                "kind",
                "authority",
                "title",
                "body",
                "source",
                "links",
            )
        }
        records[key]["revision"] = 1
    for query in corpus["queries"]:
        if query["empty"] != (not query["expected_keys"]):
            raise ValueError("Inconsistent empty judgment")
        for key in query["expected_keys"]:
            if records[key]["repository"] != query["repository"]:
                raise ValueError("Cross-scope relevance judgment")
    event_bytes = ("\n".join(event_lines) + "\n").encode()
    files = {
        "daily.ndjson": ("\n".join(daily_lines) + "\n").encode(),
        "events.ndjson": event_bytes,
        "events.ndjson.zst": zstandard.ZstdCompressor(level=3).compress(event_bytes),
        "graph.json": (json.dumps(graph, ensure_ascii=False, indent=2) + "\n").encode(),
    }
    manifest = {
        "schema_version": 1,
        "namespace": namespace,
        "generator": {"zstandard": zstandard.__version__, "compression_level": 3},
        "files": {
            "corpus.json": digest(corpus_bytes),
            **{name: digest(data) for name, data in files.items()},
        },
        "records": records,
    }
    files["manifest.json"] = (
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
    ).encode()
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--namespace", default="local-eval-v1")
    args = parser.parse_args()
    files = materialize(args.corpus.read_bytes(), args.namespace)
    if any(
        (args.output / name).exists() or (args.output / name).is_symlink()
        for name in files
    ):
        raise SystemExit("Output already exists; author a new fixture version instead")
    args.output.mkdir(parents=True, exist_ok=True)
    for name, data in files.items():
        with (args.output / name).open("xb") as output:
            output.write(data)
    print(json.dumps({"files": len(files), "namespace": args.namespace}))


if __name__ == "__main__":
    main()
