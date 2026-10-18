"""Build isolated public stores and score frozen retrieval judgments without providers."""

from __future__ import annotations

import argparse
import copy
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading

DEFAULT_FIXTURES = (
    Path(__file__).resolve().parents[1] / "crates/cortex-local/tests/fixtures/eval-v1"
)
MAX_BYTES = 4 * 1024 * 1024


class EvaluationError(ValueError):
    """A fixture, subprocess or reproducibility contract failed."""


def require(condition, message):
    if not condition:
        raise EvaluationError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def parse_json(data):
    try:
        return json.loads(data, object_pairs_hook=unique_object)
    except (ValueError, UnicodeError) as exc:
        raise EvaluationError(f"invalid JSON: {exc}") from exc


def read_bytes(path):
    require(path.is_file() and not path.is_symlink(), f"not a regular fixture: {path}")
    require(path.stat().st_size <= MAX_BYTES, f"fixture too large: {path}")
    return path.read_bytes()


def unique(items, label):
    require(all(isinstance(item, str) and item for item in items), f"invalid {label}")
    require(len(items) == len(set(items)), f"duplicate {label}")


def expected_archive_body(payload):
    """Project frozen small envelopes in the documented sorted-key readable format."""
    body = ""

    def visit(value):
        nonlocal body
        if isinstance(value, dict):
            for key in sorted(value):
                body += ("\n" if body else "") + key + ":"
                visit(value[key])
        elif isinstance(value, list):
            for item in value:
                visit(item)
        elif isinstance(value, str):
            body += ("\n" if body else "") + value
        else:
            body += canonical(value).decode()

    visit(payload)
    require(
        0 < len(body.encode()) <= 1024 * 1024,
        "fixture readable body outside supported bounds",
    )
    return body


def load_fixtures(path):
    """Validate committed inputs without invoking Cortex or decompressing archives."""
    path = Path(path).resolve()
    manifest_bytes = read_bytes(path / "manifest.json")
    manifest = parse_json(manifest_bytes)
    require(manifest.get("schema_version") == 1, "unsupported manifest schema")
    require(manifest.get("namespace") == "local-eval-v1", "unexpected namespace")
    names = {
        "corpus.json",
        "daily.ndjson",
        "events.ndjson",
        "events.ndjson.zst",
        "graph.json",
    }
    require(set(manifest.get("files", {})) == names, "unexpected fixture files")
    raw = {name: read_bytes(path / name) for name in names}
    for name, data in raw.items():
        require(digest(data) == manifest["files"][name], f"file hash mismatch: {name}")
    corpus = parse_json(raw["corpus.json"])
    require(corpus.get("schema_version") == 1, "unsupported corpus schema")
    require(corpus.get("license") == "CC0-1.0", "unexpected corpus license")
    for field in ("memories", "events", "graph_records", "queries", "graph_queries"):
        require(
            isinstance(corpus.get(field), list) and corpus[field], f"invalid {field}"
        )
    keys = (
        [x["key"] for x in corpus["memories"]]
        + [x["event_id"] for x in corpus["events"]]
        + [x["id"] for x in corpus["graph_records"]]
    )
    unique(keys, "fixture key")
    records = manifest.get("records", {})
    require(set(keys) == set(records), "manifest record coverage mismatch")
    unique([r["id"] for r in records.values()], "record ID")
    for key, record in records.items():
        require(record.get("revision") == 1, f"invalid revision: {key}")
        require(
            isinstance(record.get("repository"), str) and record["repository"],
            f"invalid scope: {key}",
        )
        for link in record["links"]:
            targets = [r for r in records.values() if r["id"] == link["target_id"]]
            require(
                len(targets) == 1 and targets[0]["repository"] == record["repository"],
                f"invalid link scope: {key}",
            )
            require(
                link["relation"]
                in {
                    "related_to",
                    "supports",
                    "contradicts",
                    "derived_from",
                    "supersedes",
                },
                f"invalid relation: {key}",
            )
    for memory in corpus["memories"]:
        expected = records[memory["key"]]
        require(memory["kind"] in {"decision", "lesson"}, "invalid daily kind")
        for field in ("repository", "kind", "authority", "title", "body"):
            require(
                memory[field] == expected[field],
                f"memory {field} mismatch: {memory['key']}",
            )
        prior = memory.get("supersedes_key")
        if prior:
            require(
                prior in records
                and records[prior]["repository"] == memory["repository"],
                "invalid supersession scope",
            )
    daily = [parse_json(line) for line in raw["daily.ndjson"].splitlines()]
    unique([row["id"] for row in daily], "daily source ID")
    require(len(daily) == len(corpus["memories"]), "daily row count mismatch")
    by_source = {m["source_id"]: records[m["key"]] for m in corpus["memories"]}
    for row in daily:
        require(row["id"] in by_source, "unknown daily source ID")
        expected = by_source[row["id"]]
        require(
            row["scope"] == "repo:" + expected["repository"], "daily scope mismatch"
        )
        require(row["version"] == 1, "daily version mismatch")
        require(
            digest(row["snapshot_raw"].encode()) == expected["snapshot_sha256"],
            "snapshot hash mismatch",
        )
        require(
            digest(row["evidence_raw"].encode()) == expected["source"]["source_hash"],
            "evidence hash mismatch",
        )
        snapshot, evidence = (
            parse_json(row["snapshot_raw"]),
            parse_json(row["evidence_raw"]),
        )
        require(
            snapshot["content_hash"] == expected["payload_hash"],
            "payload hash mismatch",
        )
        require(
            snapshot["scope"] == {"repository": expected["repository"]},
            "snapshot scope mismatch",
        )
        require(
            row["event_id"] == evidence["event_id"] == expected["source"]["source_id"],
            "evidence ID mismatch",
        )
        require(
            evidence == expected["source"]["payload"]["initial_evidence"],
            "evidence payload mismatch",
        )
        expected["metadata"] = {
            "applicability": {
                "stale": evidence["stale"],
                "branch": snapshot["branch"],
                "path": snapshot["path"],
                "pinned": snapshot["pinned"],
            },
            "daily_import": {
                "namespace": manifest["namespace"],
                "original_scope": row["scope"],
                "original_id": row["id"],
                "source_version": row["version"],
                "payload_hash": expected["payload_hash"],
                "evidence_sha256": expected["source"]["source_hash"],
                "evidence": evidence,
                "evidence_raw": row["evidence_raw"],
                "snapshot_sha256": expected["snapshot_sha256"],
                "snapshot": snapshot,
                "snapshot_raw": row["snapshot_raw"],
            },
        }
        # Typed MemoryOrigin serializes absent optional identity fields as null.
        for field in ("snapshot", "evidence"):
            typed = copy.deepcopy(expected["metadata"]["daily_import"][field])
            for identity in ("agent", "session_id", "turn_id"):
                typed["origin"].setdefault(identity, None)
            expected["metadata"]["daily_import"][field] = typed
    event_lines = raw["events.ndjson"].splitlines()
    require(
        [parse_json(line) for line in event_lines] == corpus["events"],
        "archive envelopes mismatch",
    )
    for number, (event, line) in enumerate(zip(corpus["events"], event_lines), 1):
        expected = records[event["event_id"]]
        require(
            event["context"]["repo"] == expected["repository"], "event scope mismatch"
        )
        require(event["schema_version"] == "1", "event schema mismatch")
        require(expected["source_line"] == number, "event line mismatch")
        require(expected["source"]["payload"] == event, "event payload mismatch")
        require(
            digest(line) == expected["source"]["source_hash"],
            "event source hash mismatch",
        )
        expected["body"] = expected_archive_body(event["payload"])
        expected["metadata"] = {
            "import": {
                "namespace": manifest["namespace"],
                "original_scope": "repo:"
                + canonical(event["context"]["repo"]).decode(),
                "legacy_missing_content_hash": "content_hash" not in event,
                "duplicate_policy": "first canonical source; all occurrences in archive import ledger",
            }
        }
    graph = parse_json(raw["graph.json"])
    require(len(graph) == len(corpus["graph_records"]), "graph count mismatch")
    for request, original in zip(graph, corpus["graph_records"]):
        expected = copy.deepcopy(original)
        expected["source"]["source_hash"] = digest(canonical(original))
        require(request == expected, "graph source hash or payload mismatch")
        for field, value in request.items():
            if field != "metadata":
                require(
                    value == records[request["id"]][field],
                    f"graph manifest mismatch: {field}",
                )
        records[request["id"]]["metadata"] = copy.deepcopy(request.get("metadata"))
    queries = corpus["queries"] + corpus["graph_queries"]
    unique([q["id"] for q in queries], "query ID")
    for query in corpus["queries"]:
        require(query["split"] in {"development", "held_out"}, "invalid query split")
        require(
            type(query["empty"]) is bool
            and query["empty"] == (not query["expected_keys"]),
            "inconsistent empty judgment",
        )
        unique(query["expected_keys"], "relevant key")
        for key in query["expected_keys"]:
            require(
                key in records and records[key]["repository"] == query["repository"],
                "invalid relevance scope/reference",
            )
    for query in corpus["graph_queries"]:
        require(query["root_key"] in records, "invalid graph root")
        require(
            0 <= query["depth"] <= 5 and 1 <= query["limit"] <= 200,
            "invalid graph bounds",
        )
        for key in query["expected_nodes"]:
            require(
                key in records and records[key]["repository"] == query["repository"],
                "invalid graph judgment scope",
            )
        for edge in query["expected_edges"]:
            require(
                edge["from"] in query["expected_nodes"]
                and edge["to"] in query["expected_nodes"],
                "invalid expected edge",
            )
    return {
        "path": path,
        "manifest_sha256": digest(manifest_bytes),
        "manifest": manifest,
        "corpus": corpus,
        "daily": daily,
        "graph": graph,
        "raw": raw,
    }


def tree_hashes(root):
    return {
        str(p.relative_to(root)): digest(p.read_bytes())
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


def assert_preserved(root, before):
    require(tree_hashes(root) == before, "source files changed during evaluation")


def materialize_sources(fixtures, root):
    require(not root.exists(), "source destination must be new")
    root.mkdir(parents=True)
    db = root / "daily.sqlite"
    with closing(sqlite3.connect(db)) as connection, connection:
        connection.executescript("""
            CREATE TABLE daily_memory_records(scope TEXT, id TEXT, data TEXT, PRIMARY KEY(scope,id));
            CREATE TABLE daily_memory_evidence(event_id TEXT PRIMARY KEY, scope TEXT, memory_id TEXT, version INTEGER, data TEXT);
        """)
        for row in fixtures["daily"]:
            connection.execute(
                "INSERT INTO daily_memory_records VALUES(?,?,?)",
                (row["scope"], row["id"], row["snapshot_raw"]),
            )
            connection.execute(
                "INSERT INTO daily_memory_evidence VALUES(?,?,?,?,?)",
                (
                    row["event_id"],
                    row["scope"],
                    row["id"],
                    row["version"],
                    row["evidence_raw"],
                ),
            )
    (root / "archive").mkdir()
    (root / "archive/events.ndjson.zst").write_bytes(
        fixtures["raw"]["events.ndjson.zst"]
    )
    return tree_hashes(root)


class CortexCli:
    def __init__(self, binary, home, timeout=10, output_limit=MAX_BYTES):
        self.binary = Path(binary).resolve()
        self.home = Path(home).resolve()
        self.timeout = timeout
        self.output_limit = output_limit

    def environment(self):
        env = os.environ.copy()
        env["CORTEX_HOME"] = str(self.home)
        env.pop("TYPESAFE_API_KEY", None)
        return env

    def run(self, *args):
        env = self.environment()
        command = [str(self.binary), "--home", str(self.home), *map(str, args)]
        buffers, overflow = [bytearray(), bytearray()], threading.Event()
        with subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            cwd=self.home.parent,
        ) as process:

            def drain(stream, buffer):
                while chunk := stream.read(4096):
                    if len(buffer) + len(chunk) > self.output_limit:
                        overflow.set()
                        process.kill()
                        return
                    buffer.extend(chunk)

            threads = [
                threading.Thread(target=drain, args=(stream, buffer), daemon=True)
                for stream, buffer in zip((process.stdout, process.stderr), buffers)
            ]
            for thread in threads:
                thread.start()
            try:
                process.wait(timeout=self.timeout)
            except subprocess.TimeoutExpired as exc:
                process.kill()
                process.wait()
                raise EvaluationError(
                    f"CLI deadline exceeded ({self.timeout}s): {args[0]}"
                ) from exc
            finally:
                for thread in threads:
                    thread.join(timeout=1)
            require(
                not any(t.is_alive() for t in threads),
                "CLI output drain did not terminate",
            )
            require(not overflow.is_set(), "CLI output limit exceeded")
            require(
                process.returncode == 0,
                f"CLI failed ({process.returncode}): {bytes(buffers[1]).decode('utf-8', 'replace')[:2000]}",
            )
        return parse_json(bytes(buffers[0]))


def normal_path(value):
    # Rust canonicalize emits extended Windows paths; remove only that OS prefix.
    if value.startswith("\\\\?\\UNC\\"):
        value = "\\\\" + value[8:]
    elif value.startswith("\\\\?\\"):
        value = value[4:]
    return os.path.normcase(os.path.normpath(value))


def payload_strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for nested in value.values():
            yield from payload_strings(nested)
    elif isinstance(value, list):
        for nested in value:
            yield from payload_strings(nested)


def validate_record(record, expected, sources):
    require(
        isinstance(record, dict)
        and set(record)
        == {
            "schema_version",
            "id",
            "repository",
            "revision",
            "kind",
            "authority",
            "title",
            "body",
            "source",
            "links",
            "metadata",
            "created_at",
            "updated_at",
            "content_sha256",
        },
        "record fields differ from exact schema",
    )
    for field in (
        "id",
        "repository",
        "revision",
        "kind",
        "authority",
        "title",
        "links",
    ):
        require(
            record.get(field) == expected[field],
            f"record {field} mismatch: {expected['id']}",
        )
    if "body" in expected:
        require(record.get("body") == expected["body"], "record body mismatch")
    else:
        for text in payload_strings(expected["source"]["payload"]["payload"]):
            require(text in record.get("body", ""), "archive readable text missing")
    source = record.get("source", {})
    require(isinstance(source, dict), "source evidence is not an object")
    require(
        set(source)
        == {"kind", "locator", "source_id", "captured_at", "source_hash", "payload"},
        "source fields differ from exact schema",
    )
    for field, value in expected["source"].items():
        require(
            source.get(field) == value, f"source {field} mismatch: {expected['id']}"
        )
    if "source_file" in expected:
        locator = source.get("locator", "")
        suffix = (
            "#line=" + str(expected["source_line"]) if "source_line" in expected else ""
        )
        if suffix:
            require(locator.endswith(suffix), "source locator line mismatch")
            locator = locator[: -len(suffix)]
        require(
            normal_path(locator)
            == normal_path(str((sources / expected["source_file"]).resolve())),
            "source locator mismatch",
        )
        require(
            Path(locator).samefile(sources / expected["source_file"]),
            "source locator does not identify the source file",
        )
    require(record.get("schema_version") == 1, "record schema mismatch")
    if "metadata" in expected:
        require(record["metadata"] == expected["metadata"], "record metadata mismatch")
    if expected["kind"] == "event":
        for text in payload_strings(expected["source"]["payload"]["payload"]):
            require(text in record["body"], "archive readable text missing")
    hash_input = copy.deepcopy(record)
    content_hash = hash_input.get("content_sha256")
    hash_input["content_sha256"] = ""
    require(
        isinstance(content_hash, str) and len(content_hash) == 64,
        "invalid content hash",
    )
    require(
        digest(canonical(hash_input)) == content_hash, "record content hash mismatch"
    )
    if "snapshot_sha256" in expected:
        metadata = record["metadata"]["daily_import"]
        require(
            metadata["snapshot_sha256"] == expected["snapshot_sha256"],
            "snapshot provenance mismatch",
        )
        require(
            metadata["evidence_sha256"] == expected["source"]["source_hash"],
            "evidence provenance mismatch",
        )


def stable_projection(records, manifest, sources):
    result = copy.deepcopy(records)
    for key, record in result.items():
        expected = manifest["records"][key]
        validate_record(record, expected, sources)
        for field in ("created_at", "updated_at", "content_sha256"):
            record.pop(field)
        if "source_file" in expected:
            record["source"]["locator"] = expected["source_file"] + (
                "#line=" + str(expected["source_line"])
                if "source_line" in expected
                else ""
            )
    return result


def assert_rebuild(before, after):
    require(before == after, "reindex changed canonical records")


def import_checked(cli, kind, source, count, repeat=False):
    report = cli.run("import", kind, source, "--namespace", "local-eval-v1")
    require(
        report.get("complete") is True
        and not report.get("errors")
        and report.get("rejected") == 0,
        f"incomplete {kind} import: {report}",
    )
    require(
        report["imported"] == (0 if repeat else count),
        f"unexpected {kind} import count",
    )
    if repeat:
        require(
            report["duplicates"] >= count
            or report["unchanged_files"] >= 1
            or report["records_seen"] == 0,
            f"{kind} repeat lacked checkpoint/duplicate evidence",
        )
    return report


def build_home(binary, fixtures, root):
    """Build one fresh home; callers own the temporary root's lifetime."""
    require(not root.exists(), "evaluation root must be new")
    root.mkdir(parents=True)
    home, sources = root / "home", root / "sources"
    before_sources = materialize_sources(fixtures, sources)
    cli = CortexCli(binary, home)
    imports = [
        import_checked(cli, "daily", sources / "daily.sqlite", len(fixtures["daily"])),
        import_checked(
            cli, "archive", sources / "archive", len(fixtures["corpus"]["events"])
        ),
    ]
    for ordinal, request in enumerate(fixtures["graph"]):
        capture = root / f"graph-{ordinal}.json"
        capture.write_bytes(canonical(request))
        outcome = cli.run("capture", "--input", capture)
        require(outcome.get("created") is True, "graph capture was not new")
    manifest = fixtures["manifest"]

    def get_records():
        records = {}
        for key, expected in manifest["records"].items():
            records[key] = cli.run(
                "--repository",
                expected["repository"],
                "get",
                expected["id"],
                "--revision",
                "1",
            )
            validate_record(records[key], expected, sources)
        return records

    records = get_records()
    repeats = [
        import_checked(
            cli, "daily", sources / "daily.sqlite", len(fixtures["daily"]), True
        ),
        import_checked(
            cli, "archive", sources / "archive", len(fixtures["corpus"]["events"]), True
        ),
    ]
    require(get_records() == records, "repeat import changed canonical records")
    reindex = cli.run("reindex")
    assert_rebuild(records, get_records())
    assert_preserved(sources, before_sources)
    return {
        "cli": cli,
        "sources": sources,
        "records": records,
        "projection": stable_projection(records, manifest, sources),
        "report": {
            "source_hashes": before_sources,
            "record_ids": {k: v["id"] for k, v in records.items()},
            "imports": imports,
            "repeat_imports": repeats,
            "reindex": reindex,
            "rebuild_identical": True,
            "sources_preserved": True,
        },
    }


def score_query(query, hits, manifest, sources):
    """Score fixed judgments; invalid evidence earns no credit and fails integrity."""
    expected_ids = [manifest["records"][key]["id"] for key in query["expected_keys"]]
    by_id = {record["id"]: record for record in manifest["records"].values()}
    retrieved, diagnostics, relevant_ranks, seen = [], [], [], set()
    if not isinstance(hits, list):
        hits = []
        diagnostics.append("search response is not a list")
    if len(hits) > 10:
        diagnostics.append("search returned more than the requested ten hits")
    for rank, hit in enumerate(hits, 1):
        record = hit.get("record") if isinstance(hit, dict) else None
        error = None
        try:
            require(isinstance(record, dict), "missing record evidence")
            identity = record.get("id")
            require(
                isinstance(identity, str) and identity in by_id,
                "unknown or missing record ID",
            )
            require(identity not in seen, "duplicate record ID")
            seen.add(identity)
            require(
                record.get("repository") == query["repository"],
                "wrong repository scope",
            )
            require(
                record.get("authority") not in {"superseded", "deprecated"},
                "inactive record returned",
            )
            validate_record(record, by_id[identity], sources)
            if identity in expected_ids:
                relevant_ranks.append(rank)
        except (EvaluationError, KeyError, TypeError, OSError) as exc:
            error = str(exc)
            diagnostics.append(f"rank {rank}: {error}")
        retrieved.append(
            {
                "rank": rank,
                "valid": error is None,
                "error": error,
                "id": record.get("id") if isinstance(record, dict) else None,
                "revision": record.get("revision")
                if isinstance(record, dict)
                else None,
                "content_sha256": record.get("content_sha256")
                if isinstance(record, dict)
                else None,
                "source": record.get("source") if isinstance(record, dict) else None,
            }
        )
    return {
        "id": query["id"],
        "split": query["split"],
        "intent": query["intent"],
        "repository": query["repository"],
        "text": query["text"],
        "empty": query["empty"],
        "expected_keys": query["expected_keys"],
        "expected_ids": expected_ids,
        "expected_evidence": [
            {
                "id": manifest["records"][key]["id"],
                "revision": manifest["records"][key]["revision"],
                "source": manifest["records"][key]["source"],
            }
            for key in query["expected_keys"]
        ],
        "retrieved": retrieved,
        "diagnostics": diagnostics,
        "integrity_ok": not diagnostics,
        "recall_at_5": sum(rank <= 5 for rank in relevant_ranks) / len(expected_ids)
        if expected_ids
        else None,
        "reciprocal_rank_at_10": next(
            (1 / rank for rank in relevant_ranks if rank <= 10), 0
        )
        if expected_ids
        else None,
    }


def evaluate_quality(cli, fixtures, sources):
    results = []
    for query in fixtures["corpus"]["queries"]:
        failure = None
        try:
            hits = cli.run(
                "--repository",
                query["repository"],
                "search",
                query["text"],
                "--limit",
                "10",
            )
        except (EvaluationError, OSError) as exc:
            hits, failure = [], str(exc)
        result = score_query(query, hits, fixtures["manifest"], sources)
        if failure:
            result["diagnostics"].append("search execution failed: " + failure)
            result["integrity_ok"] = False
        results.append(result)
    splits = {}
    for split in ("development", "held_out", "all"):
        selected = [q for q in results if split == "all" or q["split"] == split]
        answerable = [q for q in selected if not q["empty"]]
        empty = [q for q in selected if q["empty"]]
        splits[split] = {
            "answerable_queries": len(answerable),
            "empty_queries": len(empty),
            "recall_at_5": sum(q["recall_at_5"] for q in answerable) / len(answerable)
            if answerable
            else None,
            "mrr_at_10": sum(q["reciprocal_rank_at_10"] for q in answerable)
            / len(answerable)
            if answerable
            else None,
            "empty_queries_without_hits": sum(
                q["integrity_ok"] and not q["retrieved"] for q in empty
            ),
            "integrity_failed_queries": sum(not q["integrity_ok"] for q in selected),
            "integrity_ok": all(q["integrity_ok"] for q in selected),
        }
    return {
        "queries": results,
        "splits": splits,
        "integrity_ok": all(q["integrity_ok"] for q in results),
        "metric_policy": "Macro averages over nonempty judgments only; empty queries reported separately. Invalid hits receive zero credit; integrity failures invalidate acceptance. No quality floor set.",
    }


def binary_hash(binary):
    with Path(binary).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def scoped_status(manifest, repository):
    records = [r for r in manifest["records"].values() if r["repository"] == repository]
    events = sum(r["kind"] == "event" for r in records)
    return {
        "records": len(records),
        "revisions": len(records),
        "repositories": int(bool(records)),
        "events": events,
        "memories": len(records) - events,
        "links": sum(len(r["links"]) for r in records),
    }


def validate_scope_hits(hits, repository, manifest, sources):
    require(isinstance(hits, list), "scope search response is not an array")
    known = {
        r["id"]: r
        for r in manifest["records"].values()
        if r["repository"] == repository
    }
    seen = set()
    for hit in hits:
        require(
            isinstance(hit, dict) and isinstance(hit.get("record"), dict),
            "scope search record missing",
        )
        record = hit["record"]
        identity = record.get("id")
        require(
            isinstance(identity, str) and identity in known,
            "foreign or unknown scope identity",
        )
        require(identity not in seen, "duplicate scope identity")
        seen.add(identity)
        validate_record(record, known[identity], sources)
        require(hit.get("score") in (1.0, 2.0), "unexpected standalone ranking input")
        for link in record["links"]:
            require(link["target_id"] in known, "foreign link target")


def validate_recall_response(result, expected):
    fields = {
        "repository",
        "context",
        "query_id",
        "delivery_id",
        "audit_persisted",
        "delivery_consumption",
    }
    require(type(result) is dict and set(result) == fields, "invalid recall envelope")
    require(result["audit_persisted"] is True, "recall audit was not persisted")
    for key in ("query_id", "delivery_id"):
        require(
            isinstance(result[key], str) and bool(result[key].strip()),
            "invalid recall " + key,
        )
    require(result["query_id"] != result["delivery_id"], "recall IDs must differ")
    require(
        {key: result[key] for key in ("repository", "context", "delivery_consumption")}
        == expected,
        "recall context/provenance differs from scoped evidence",
    )


def expected_recall_context(hits):
    context = "Cortex local evidence (advisory; source authority unchanged):\n"
    records = [h["record"] for h in hits if h["record"]["kind"] != "event"][:5]
    for record in records:
        title = record["title"].replace("\r", " ").replace("\n", " ")
        locator = record["source"]["locator"].replace("\r", " ").replace("\n", " ")
        context += (
            f"[{record['id']}@{record['revision']} {record['content_sha256']}] "
            f"{title} | repository={record['repository']} "
            f"authority={record['authority'].title()} source={locator}\n"
        )
        body = record["body"]
        context += body + ("\n" if body and not body.endswith("\n") else "")
    require(
        len(context.encode()) + 64 <= 6144,
        "scope recall fixture exceeds default budget",
    )
    return context


def evaluate_scope(cli, fixtures, sources, session_factory=None):
    if __package__:
        from .local_eval_transport import CortexMcp, TransportError
    else:
        from local_eval_transport import CortexMcp, TransportError

    checks = []
    manifest = fixtures["manifest"]
    scopes = ("Atlas", "atlas", "AtlasLab", "AtlasProduction")
    search_texts = ("offline desktop notes", "export")
    baselines = {}

    def check(name, operation):
        try:
            operation()
            checks.append({"name": name, "passed": True})
        except (EvaluationError, TransportError, OSError, KeyError, TypeError) as exc:
            checks.append({"name": name, "passed": False, "error": str(exc)})

    def equality(actual, expected, label):
        require(actual == expected, label)

    for repository in scopes:
        check(
            f"cli-status:{repository}",
            lambda repository=repository: equality(
                cli.run("--repository", repository, "status"),
                scoped_status(manifest, repository),
                "CLI statistics disclose another scope",
            ),
        )
        for text in search_texts:

            def baseline(repository=repository, text=text):
                hits = cli.run(
                    "--repository", repository, "search", text, "--limit", "10"
                )
                validate_scope_hits(hits, repository, manifest, sources)
                baselines[(repository, text)] = hits

            check(f"cli-search:{repository}:{text}", baseline)

    session_factory = session_factory or CortexMcp
    try:
        with session_factory(cli.binary, cli.home) as mcp:
            # One process alternates exact scopes, including a missing repository.
            for round_number in range(2):
                for repository in scopes:
                    for text in search_texts:

                        def search(repository=repository, text=text):
                            hits = mcp.call(
                                "cortex_search",
                                {"repository": repository, "query": text, "limit": 10},
                            )
                            validate_scope_hits(hits, repository, manifest, sources)
                            equality(
                                hits,
                                baselines[(repository, text)],
                                "warm MCP results/ranking differ from CLI",
                            )

                        check(f"mcp-search:{round_number}:{repository}:{text}", search)
                    check(
                        f"mcp-status:{round_number}:{repository}",
                        lambda repository=repository: equality(
                            mcp.call("cortex_status", {"repository": repository}),
                            scoped_status(manifest, repository),
                            "MCP statistics disclose another scope",
                        ),
                    )
                    foreign = next(
                        r
                        for r in manifest["records"].values()
                        if r["repository"] != repository
                    )
                    check(
                        f"mcp-foreign-get:{round_number}:{repository}",
                        lambda repository=repository, foreign=foreign: equality(
                            mcp.call(
                                "cortex_get",
                                {"repository": repository, "id": foreign["id"]},
                            ),
                            None,
                            "foreign ID resolved outside requested scope",
                        ),
                    )
                    check(
                        f"mcp-foreign-graph:{round_number}:{repository}",
                        lambda repository=repository, foreign=foreign: equality(
                            mcp.call(
                                "cortex_graph",
                                {
                                    "repository": repository,
                                    "id": foreign["id"],
                                    "depth": 2,
                                    "limit": 10,
                                },
                            ),
                            {"records": [], "edges": [], "truncated": False},
                            "foreign graph root disclosed evidence",
                        ),
                    )

                    def recall(repository=repository):
                        result = mcp.call(
                            "cortex_memory_read",
                            {
                                "repository": repository,
                                "query": "offline desktop notes",
                            },
                        )
                        validate_recall_response(
                            result,
                            {
                                "repository": repository,
                                "context": expected_recall_context(
                                    baselines[(repository, "offline desktop notes")]
                                ),
                                "delivery_consumption": "not_observed",
                            },
                        )

                    check(f"mcp-recall:{round_number}:{repository}", recall)

            # Synthetic probes are appended only after quality measurement. They
            # are not part of the frozen relevance corpus or its metric denominators.
            primary = {
                "repository": "Atlas",
                "id": "scope-probe-shared",
                "kind": "note",
                "authority": "accepted",
                "title": "scopeprobe search",
                "body": "Primary scope evidence.",
                "source": {
                    "kind": "fixture",
                    "locator": "fiction://scope-probe/Atlas",
                    "source_id": "primary",
                    "captured_at": "2026-01-01T00:00:00Z",
                    "payload": {"fictional": True},
                },
                "links": [
                    {"target_id": "scope-probe-foreign-only", "relation": "related_to"}
                ],
                "metadata": {},
            }
            capture_path = cli.home.parent / "scope-probe.json"

            def capture(request):
                capture_path.write_bytes(canonical(request))
                return cli.run("capture", "--input", capture_path)["record"]

            def perturbation():
                primary_record = capture(primary)
                secondary = copy.deepcopy(primary)
                secondary["id"] = "scope-probe-secondary"
                secondary["source"]["source_id"] = "secondary"
                secondary["source"]["captured_at"] = "2025-01-01T00:00:00Z"
                secondary["links"] = []
                capture(secondary)
                baseline_hits = mcp.call(
                    "cortex_search",
                    {"repository": "Atlas", "query": "scopeprobe search", "limit": 10},
                )
                require(
                    [h["record"]["id"] for h in baseline_hits]
                    == [primary["id"], secondary["id"]],
                    "primary ranking probe missing",
                )
                before_status = mcp.call("cortex_status", {"repository": "Atlas"})
                for repository in ("atlas", "AtlasLab"):
                    other = copy.deepcopy(primary)
                    other["repository"] = repository
                    other["body"] = "Foreign scope evidence: " + repository
                    other["source"]["source_id"] = repository
                    other["source"]["locator"] = "fiction://scope-probe/" + repository
                    other["source"]["captured_at"] = "2030-01-01T00:00:00Z"
                    other["links"] = []
                    other_record = capture(other)
                    equality(
                        mcp.call(
                            "cortex_get", {"repository": repository, "id": other["id"]}
                        ),
                        other_record,
                        "same ID resolved to another repository",
                    )
                foreign_target = copy.deepcopy(other)
                foreign_target["id"] = "scope-probe-foreign-only"
                foreign_target["source"]["source_id"] = "foreign-only"
                capture(foreign_target)
                cli.run("reindex")
                equality(
                    mcp.call(
                        "cortex_get", {"repository": "Atlas", "id": primary["id"]}
                    ),
                    primary_record,
                    "shared ID contaminated primary evidence",
                )
                equality(
                    mcp.call(
                        "cortex_search",
                        {
                            "repository": "Atlas",
                            "query": "scopeprobe search",
                            "limit": 10,
                        },
                    ),
                    baseline_hits,
                    "foreign corpus changed ranking inputs/order",
                )
                equality(
                    mcp.call("cortex_status", {"repository": "Atlas"}),
                    before_status,
                    "foreign corpus changed scoped statistics",
                )
                equality(
                    mcp.call(
                        "cortex_graph",
                        {
                            "repository": "Atlas",
                            "id": primary["id"],
                            "depth": 2,
                            "limit": 10,
                        },
                    ),
                    {"records": [primary_record], "edges": [], "truncated": False},
                    "dangling link resolved through foreign repository",
                )

            check("mcp-ranking-and-same-id-perturbation", perturbation)
    except (TransportError, OSError) as exc:
        checks.append({"name": "persistent-mcp", "passed": False, "error": str(exc)})
    return {
        "passed": all(c["passed"] for c in checks),
        "checks": checks,
        "scope_sequence": list(scopes) * 2,
        "ranking_contract": "Fixed title-match score and scoped recency/ID order; no global BM25 or exposed ranking-input API.",
        "probe_policy": "Five fictional controls (two primary results, two foreign same-ID records and one foreign-only link target) added after frozen-corpus quality measurement in temporary home only.",
    }


def validate_graph_result(
    result, repository, expected_records, expected_edges, sources, truncated=False
):
    """Validate exact BFS evidence, retaining original directed edge orientation."""
    require(isinstance(result, dict), "graph result is not an object")
    records, edges = result.get("records"), result.get("edges")
    require(
        isinstance(records, list) and isinstance(edges, list), "graph lists missing"
    )
    require(
        all(isinstance(record, dict) for record in records), "graph record malformed"
    )
    identities = [record.get("id") for record in records]
    unique(identities, "graph node ID")
    require(set(identities) == set(expected_records), "graph node identities differ")
    for record in records:
        require(record.get("repository") == repository, "graph node scope differs")
        validate_record(record, expected_records[record["id"]], sources)
    actual_edges = []
    for edge in edges:
        require(
            isinstance(edge, dict)
            and set(edge) == {"source_id", "target_id", "relation"},
            "graph edge malformed",
        )
        require(
            edge["source_id"] in identities and edge["target_id"] in identities,
            "graph edge endpoint absent",
        )
        actual_edges.append((edge["source_id"], edge["target_id"], edge["relation"]))
    require(len(actual_edges) == len(set(actual_edges)), "duplicate graph edge")
    expected_tuples = {
        (edge["source_id"], edge["target_id"], edge["relation"])
        for edge in expected_edges
    }
    require(set(actual_edges) == expected_tuples, "graph typed edges differ")
    require(
        type(result.get("truncated")) is bool and result["truncated"] == truncated,
        "graph truncation differs",
    )
    return {
        "nodes": [
            {
                "id": r["id"],
                "revision": r["revision"],
                "content_sha256": r["content_sha256"],
                "source": r["source"],
            }
            for r in records
        ],
        "edges": edges,
        "truncated": result["truncated"],
    }


def validate_graph_bounds_error(result, depth, limit):
    expected = (
        "invalid local record: numeric argument exceeds limit"
        if depth > 5 or limit > 100
        else "invalid local record: graph depth must be 0..5, result limit 1..200"
    )
    require(
        result == {"isError": True, "content": [{"type": "text", "text": expected}]},
        "unexpected MCP graph bounds rejection",
    )


def evaluate_graph(cli, fixtures, sources, quality):
    if __package__:
        from .local_eval_transport import CortexMcp, TransportError
    else:
        from local_eval_transport import CortexMcp, TransportError

    checks = []
    manifest = fixtures["manifest"]["records"]

    def check(name, work):
        try:
            evidence = work()
            checks.append({"name": name, "passed": True, "evidence": evidence})
        except (
            EvaluationError,
            TransportError,
            OSError,
            KeyError,
            TypeError,
            ValueError,
        ) as exc:
            checks.append({"name": name, "passed": False, "error": str(exc)})

    def cli_graph(repository, identity, depth, limit):
        return cli.run(
            "--repository",
            repository,
            "graph",
            identity,
            "--depth",
            str(depth),
            "--limit",
            str(limit),
        )

    try:
        with CortexMcp(cli.binary, cli.home) as mcp:

            def both(
                name,
                repository,
                identity,
                depth,
                limit,
                expected,
                edges,
                truncated=False,
            ):
                for transport in ("cli", "mcp"):

                    def run(transport=transport):
                        result = (
                            cli_graph(repository, identity, depth, limit)
                            if transport == "cli"
                            else mcp.call(
                                "cortex_graph",
                                {
                                    "repository": repository,
                                    "id": identity,
                                    "depth": depth,
                                    "limit": limit,
                                },
                            )
                        )
                        return validate_graph_result(
                            result, repository, expected, edges, sources, truncated
                        )

                    check(name + "-" + transport, run)

            for query in fixtures["corpus"]["graph_queries"]:
                expected = {
                    manifest[key]["id"]: manifest[key]
                    for key in query["expected_nodes"]
                }
                edges = [
                    {
                        "source_id": manifest[e["from"]]["id"],
                        "target_id": manifest[e["to"]]["id"],
                        "relation": e["relation"],
                    }
                    for e in query["expected_edges"]
                ]
                both(
                    "frozen-" + query["id"],
                    query["repository"],
                    manifest[query["root_key"]]["id"],
                    query["depth"],
                    query["limit"],
                    expected,
                    edges,
                )

            for event in fixtures["corpus"]["events"]:
                expected = manifest[event["event_id"]]

                def history(expected=expected):
                    record = cli.run(
                        "--repository",
                        expected["repository"],
                        "get",
                        expected["id"],
                        "--revision",
                        "1",
                    )
                    via_mcp = mcp.call(
                        "cortex_get",
                        {
                            "repository": expected["repository"],
                            "id": expected["id"],
                            "revision": 1,
                        },
                    )
                    validate_record(record, expected, sources)
                    validate_record(via_mcp, expected, sources)
                    require(record == via_mcp, "historical CLI/MCP evidence differs")
                    return {
                        "id": record["id"],
                        "revision": record["revision"],
                        "content_sha256": record["content_sha256"],
                        "body": record["body"],
                        "source": record["source"],
                    }

                check("historical-" + event["event_id"], history)

            # Controls live only in this temporary home after frozen quality scoring.
            controls = {}
            for ordinal in range(7):
                identity = f"graph-control-{ordinal}"
                links = (
                    [
                        {
                            "target_id": f"graph-control-{ordinal + 1}",
                            "relation": "supports"
                            if ordinal % 2 == 0
                            else "derived_from",
                        }
                    ]
                    if ordinal < 6
                    else []
                )
                request = {
                    "id": identity,
                    "repository": "Atlas",
                    "kind": "note",
                    "authority": "accepted",
                    "title": "Fictional bounded chain",
                    "body": f"Graph control node {ordinal}",
                    "source": {
                        "kind": "fixture",
                        "locator": "fiction://graph-control/" + identity,
                        "source_id": identity,
                        "source_hash": digest(identity.encode()),
                        "captured_at": None,
                        "payload": {"fictional": True},
                    },
                    "links": links,
                    "metadata": {},
                }
                controls[identity] = request
            controls["graph-control-inactive"] = {
                **copy.deepcopy(controls["graph-control-6"]),
                "id": "graph-control-inactive",
                "authority": "superseded",
            }
            controls["graph-control-dangling"] = {
                **copy.deepcopy(controls["graph-control-6"]),
                "id": "graph-control-dangling",
                "links": [
                    {"target_id": "graph-absent-local", "relation": "related_to"},
                    {"target_id": "graph-foreign-only", "relation": "derived_from"},
                    {"target_id": "graph-control-inactive", "relation": "supports"},
                ],
            }
            controls["graph-foreign-only"] = {
                **copy.deepcopy(controls["graph-control-6"]),
                "id": "graph-foreign-only",
                "repository": "atlas",
            }
            for identity, request in controls.items():
                path = cli.home.parent / (identity + ".json")
                path.write_bytes(canonical(request))
                outcome = cli.run("capture", "--input", path)
                expected = {**request, "revision": 1}
                validate_record(outcome["record"], expected, sources)
                controls[identity] = expected
            all_edges = [
                {
                    "source_id": f"graph-control-{n}",
                    "target_id": f"graph-control-{n + 1}",
                    "relation": "supports" if n % 2 == 0 else "derived_from",
                }
                for n in range(6)
            ]
            for depth in range(6):
                expected = {
                    f"graph-control-{n}": controls[f"graph-control-{n}"]
                    for n in range(depth + 1)
                }
                both(
                    f"depth-{depth}",
                    "Atlas",
                    "graph-control-0",
                    depth,
                    100,
                    expected,
                    all_edges[:depth],
                )
            reverse = {
                f"graph-control-{n}": controls[f"graph-control-{n}"] for n in (4, 5, 6)
            }
            both(
                "reverse-orientation",
                "Atlas",
                "graph-control-6",
                2,
                100,
                reverse,
                all_edges[4:],
            )
            for limit in (1, 2):
                expected = {
                    f"graph-control-{n}": controls[f"graph-control-{n}"]
                    for n in range(limit)
                }
                both(
                    f"truncation-{limit}",
                    "Atlas",
                    "graph-control-0",
                    5,
                    limit,
                    expected,
                    all_edges[: limit - 1],
                    True,
                )
            both(
                "depth-zero-is-not-truncated",
                "Atlas",
                "graph-control-0",
                0,
                1,
                {"graph-control-0": controls["graph-control-0"]},
                [],
            )
            for identity in (
                "graph-control-inactive",
                "graph-absent-local",
                "graph-foreign-only",
            ):
                both("empty-root-" + identity, "Atlas", identity, 5, 100, {}, [])
            both(
                "dangling-and-inactive-targets",
                "Atlas",
                "graph-control-dangling",
                5,
                100,
                {"graph-control-dangling": controls["graph-control-dangling"]},
                [],
            )
            check(
                "cli-limit-200",
                lambda: validate_graph_result(
                    cli_graph("Atlas", "graph-control-0", 5, 200),
                    "Atlas",
                    {
                        f"graph-control-{n}": controls[f"graph-control-{n}"]
                        for n in range(6)
                    },
                    all_edges[:5],
                    sources,
                ),
            )
            for transport, depth, limit in (
                ("cli", 6, 10),
                ("cli", 1, 0),
                ("cli", 1, 201),
                ("mcp", 6, 10),
                ("mcp", 1, 0),
                ("mcp", 1, 101),
            ):

                def invalid(transport=transport, depth=depth, limit=limit):
                    if transport == "cli":
                        try:
                            cli_graph("Atlas", "graph-control-0", depth, limit)
                        except EvaluationError as exc:
                            require(
                                "graph depth must be 0..5" in str(exc),
                                "unexpected CLI rejection",
                            )
                            return {"rejected": True, "depth": depth, "limit": limit}
                        raise EvaluationError("CLI accepted invalid graph bounds")
                    result = mcp.call_raw(
                        "cortex_graph",
                        {
                            "repository": "Atlas",
                            "id": "graph-control-0",
                            "depth": depth,
                            "limit": limit,
                        },
                    )
                    require(
                        result.get("isError") is True,
                        "MCP accepted invalid graph bounds",
                    )
                    validate_graph_bounds_error(result, depth, limit)
                    return {
                        "rejected": True,
                        "depth": depth,
                        "limit": limit,
                        "error_result": result,
                    }

                check(f"bounds-{transport}-{depth}-{limit}", invalid)
    except (EvaluationError, TransportError, OSError, KeyError, TypeError) as exc:
        checks.append(
            {"name": "graph-transport-or-controls", "passed": False, "error": str(exc)}
        )
    archive_ids = {
        manifest[event["event_id"]]["id"] for event in fixtures["corpus"]["events"]
    }
    observed_search = [
        {"query_id": query["id"], "hit": hit}
        for query in quality["queries"]
        for hit in query["retrieved"]
        if hit["id"] in archive_ids
    ]
    return {
        "passed": all(check["passed"] for check in checks),
        "checks": checks,
        "historical_search_evidence": observed_search,
        "contract": "Repository-scoped bidirectional BFS, depth 0..5; CLI node limit 1..200 and MCP 1..100. Directed edge labels preserved. No shortest-path or induced-subgraph claim.",
        "control_policy": "Synthetic chain, inactive, missing and dangling-link controls captured only after frozen quality scoring in a temporary home.",
    }


def evaluate(binary, fixtures_path=DEFAULT_FIXTURES):
    fixtures = load_fixtures(fixtures_path)
    binary = Path(binary).resolve()
    original_binary_hash = binary_hash(binary)
    with tempfile.TemporaryDirectory(prefix="cortex-public-eval-") as scratch:
        root = Path(scratch)
        build_info = CortexCli(binary, root / "build-info-home").run("build-info")
        first = build_home(binary, fixtures, root / "first")
        second = build_home(binary, fixtures, root / "second")
        require(
            first["projection"] == second["projection"],
            "cross-home stable projection differs",
        )
        require(
            binary_hash(binary) == original_binary_hash,
            "binary changed during evaluation",
        )
        quality = evaluate_quality(first["cli"], fixtures, first["sources"])
        isolation = evaluate_scope(first["cli"], fixtures, first["sources"])
        graph = evaluate_graph(second["cli"], fixtures, second["sources"], quality)
        assert_preserved(second["sources"], second["report"]["source_hashes"])
        assert_preserved(first["sources"], first["report"]["source_hashes"])
        require(
            binary_hash(binary) == original_binary_hash,
            "binary changed during evaluation",
        )
        return {
            "schema_version": 1,
            "binary": {
                "path": str(binary),
                "sha256": original_binary_hash,
                "build_info": build_info,
            },
            "fixture_manifest_sha256": fixtures["manifest_sha256"],
            "fixture_hashes": fixtures["manifest"]["files"],
            "namespace": "local-eval-v1",
            "homes": [first["report"], second["report"]],
            "reproducible": True,
            "quality": quality,
            "isolation": isolation,
            "graph": graph,
            "stable_projection_sha256": digest(canonical(first["projection"])),
            "scope": "fixture construction, provenance and fixed offline retrieval judgments; no latency measurement",
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, default=DEFAULT_FIXTURES)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = evaluate(args.binary, args.fixtures)
    except (EvaluationError, OSError, KeyError, TypeError, sqlite3.Error) as exc:
        parser.exit(1, f"evaluation failed: {exc}\n")
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if (
        not report["quality"]["integrity_ok"]
        or not report["isolation"]["passed"]
        or not report["graph"]["passed"]
    ):
        parser.exit(
            1, f"retrieval integrity failed; evidence preserved in {args.report}\n"
        )
    print(
        "Construction and retrieval integrity passed; quality scores recorded without a floor."
    )


if __name__ == "__main__":
    main()
