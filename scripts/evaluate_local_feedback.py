"""Locked synthetic feedback comparison; never evidence of real productivity."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import statistics
import tempfile
import time

if __package__:
    from . import evaluate_local_memory as base
    from .local_eval_transport import CortexMcp
else:
    import evaluate_local_memory as base
    from local_eval_transport import CortexMcp

ROOT = Path(__file__).resolve().parents[1]
LOCKS = {
    "tests/fixtures/eval-v1/corpus.json": "dd4c38d60a66f36004aa440a68279adb1043dd72c0723caeef13b510c0ad1521",
    "tests/fixtures/eval-v1/manifest.json": "a661af9be17b6148a19b65fd5568f4787c6b3681d24e2caa3c08005a47b90734",
    "baselines/local-eval-v1.json": "7a82c121075cf0f0c2a6be1ebc7115044fc77c80055e76af7e563efb6a65dcc5",
}


def verify_locks(root=ROOT / "crates/cortex-local"):
    actual = {name: base.digest((root / name).read_bytes()) for name in LOCKS}
    base.require(actual == LOCKS, "frozen evaluation inputs changed")
    return actual


def search(session, query, enabled=False, as_of=None):
    args = {"repository": query["repository"], "query": query["text"], "limit": 10}
    if enabled:
        args.update(feedback=True, as_of=as_of)
    return session.call("cortex_search", args)


def projection(hits):
    return [
        {key: value for key, value in hit.items() if key != "feedback"} for hit in hits
    ]


def seed_query(session, query, manifest):
    # Guard before consulting labels or issuing any request, including recall.
    base.require(query["split"] == "development", "held-out feedback seeding forbidden")
    expected = {manifest["records"][key]["id"] for key in query["expected_keys"]}
    by_id = {record["id"]: record for record in manifest["records"].values()}
    created = []
    for sample in range(3):
        recall = session.call(
            "cortex_memory_read",
            {
                "repository": query["repository"],
                "query": query["text"],
                "limit": 5,
            },
        )
        base.require(
            recall["audit_persisted"] is True, "synthetic delivery not persisted"
        )
        receipt = session.call(
            "cortex_audit_get",
            {
                "repository": query["repository"],
                "id": recall["delivery_id"],
            },
        )
        draft = receipt["payload"]["content"]
        for ordinal, reference in enumerate(draft["references"]):
            base.require(
                by_id[reference["id"]]["kind"] != "event",
                "event cannot receive feedback",
            )
            request = {
                "repository": query["repository"],
                "id": f"fictional-feedback-{query['id']}-{sample}-{ordinal}",
                "source": {
                    "kind": "synthetic_evaluation",
                    "reference": f"eval-v1/development/{query['id']}/{sample}",
                    "actor": "fictional-evaluation-driver",
                },
                "evidence": "explicit",
                "judgment": "helpful" if reference["id"] in expected else "unhelpful",
                "query_id": recall["query_id"],
                "delivery_id": recall["delivery_id"],
                "reference": reference,
                "comment": "Fictional development fixture judgment; not observed human or model benefit.",
            }
            result = session.call("cortex_feedback_record", request)
            base.require(result["created"] is True, "synthetic feedback was not new")
            created.append(
                {
                    "query": query["id"],
                    "split": query["split"],
                    "feedback_id": request["id"],
                    "record_id": reference["id"],
                    "judgment": request["judgment"],
                }
            )
    return created


def distribution(values):
    ordered = sorted(values)
    return {
        "samples_ms": values,
        "median_ms": statistics.median(values),
        "min_ms": ordered[0],
        "max_ms": ordered[-1],
        "p95_ms": ordered[math.ceil(len(ordered) * 0.95) - 1],
        "percentile_method": "nearest_rank_ceil_p_times_n",
    }


def canonical_snapshot(home):
    result = {}
    for directory in ("records", "segments", "audit", "feedback"):
        root = home / "local" / directory
        for path in sorted(root.rglob("*")):
            if path.is_file():
                result[str(path.relative_to(home))] = base.digest(path.read_bytes())
    base.require(
        any("feedback" in path for path in result),
        "feedback preservation snapshot empty",
    )
    base.require(
        any("records" in Path(path).parts for path in result),
        "memory preservation snapshot empty",
    )
    base.require(
        any("segments" in Path(path).parts for path in result),
        "event preservation snapshot empty",
    )
    return result


def summarize(results):
    splits = {}
    for split in ("development", "held_out", "all"):
        selected = [row for row in results if split == "all" or row["split"] == split]
        answerable = [row for row in selected if row["disabled"]["expected_ids"]]
        summary = {"queries": len(selected), "answerable": len(answerable)}
        for mode in ("disabled", "enabled"):
            summary[mode] = {
                "recall_at_5": statistics.mean(
                    row[mode]["recall_at_5"] for row in answerable
                ),
                "mrr_at_10": statistics.mean(
                    row[mode]["reciprocal_rank_at_10"] for row in answerable
                ),
            }
        summary["regressions"] = [
            row["id"]
            for row in answerable
            if any(
                row["enabled"][key] < row["disabled"][key]
                for key in ("recall_at_5", "reciprocal_rank_at_10")
            )
        ]
        summary["unanswered"] = [
            row["id"] for row in answerable if not row["enabled"]["retrieved"]
        ]
        summary["empty_query_false_positives"] = {
            mode: [
                row["id"]
                for row in selected
                if not row[mode]["expected_ids"] and row[mode]["retrieved"]
            ]
            for mode in ("disabled", "enabled")
        }
        summary["changed_ranks"] = [
            row["id"]
            for row in selected
            if [hit["id"] for hit in row["disabled"]["retrieved"]]
            != [hit["id"] for hit in row["enabled"]["retrieved"]]
        ]
        summary["latency"] = {
            mode: distribution(
                [
                    sample
                    for row in selected
                    for sample in row["latency"][mode]["samples_ms"]
                ]
            )
            for mode in ("disabled", "enabled")
        }
        splits[split] = summary
    return splits


def evaluate(binary):
    locks = verify_locks()
    executable_hash = base.binary_hash(binary)
    fixtures = base.load_fixtures(base.DEFAULT_FIXTURES)
    queries = fixtures["corpus"]["queries"]
    with tempfile.TemporaryDirectory(prefix="cortex-feedback-eval-") as temporary:
        built = base.build_home(binary, fixtures, Path(temporary) / "evaluation")
        cli, sources = built["cli"], built["sources"]
        source_before = base.tree_hashes(sources)
        with CortexMcp(binary, cli.home, timeout=30) as session:
            before = {query["id"]: search(session, query) for query in queries}
            pre_time = datetime.now(timezone.utc).isoformat()
            for query in queries:
                base.require(
                    projection(search(session, query, True, pre_time))
                    == before[query["id"]],
                    "no-feedback enabled baseline differs",
                )
            seeded = []
            for query in queries:
                if query["split"] == "development":
                    seeded.extend(seed_query(session, query, fixtures["manifest"]))
            as_of = datetime.now(timezone.utc).isoformat()
            snapshot = canonical_snapshot(cli.home)
            results = []
            enabled_results = {}
            for query in queries:
                samples = {"disabled": [], "enabled": []}
                responses = {}
                for trial in range(5):
                    order = (False, True) if trial % 2 == 0 else (True, False)
                    for enabled in order:
                        mode = "enabled" if enabled else "disabled"
                        started = time.perf_counter()
                        hits = search(session, query, enabled, as_of)
                        samples[mode].append((time.perf_counter() - started) * 1000)
                        if mode in responses:
                            base.require(
                                responses[mode] == hits,
                                "fixed as_of query changed across trials",
                            )
                        responses[mode] = hits
                base.require(
                    responses["disabled"] == before[query["id"]],
                    "disabled rollback differs from baseline",
                )
                cli_hits = cli.run(
                    "--repository",
                    query["repository"],
                    "search",
                    query["text"],
                    "--limit",
                    "10",
                )
                base.require(
                    cli_hits == before[query["id"]], "CLI and MCP disabled mismatch"
                )
                cli_enabled = cli.run(
                    "--repository",
                    query["repository"],
                    "search",
                    query["text"],
                    "--limit",
                    "10",
                    "--feedback",
                    "--as-of",
                    as_of,
                )
                base.require(
                    cli_enabled == responses["enabled"], "CLI and MCP enabled mismatch"
                )
                enabled_results[query["id"]] = responses["enabled"]
                scores = {
                    mode: base.score_query(query, hits, fixtures["manifest"], sources)
                    for mode, hits in responses.items()
                }
                base.require(
                    all(score["integrity_ok"] for score in scores.values()),
                    "retrieval evidence integrity failed",
                )
                results.append(
                    {
                        "id": query["id"],
                        "split": query["split"],
                        **scores,
                        "latency": {
                            mode: distribution(values)
                            for mode, values in samples.items()
                        },
                        "explanations": [
                            hit.get("feedback") for hit in responses["enabled"]
                        ],
                    }
                )
            base.require(
                canonical_snapshot(cli.home) == snapshot,
                "search mutated canonical source or feedback",
            )
            cli.run("reindex")
            base.require(
                canonical_snapshot(cli.home) == snapshot,
                "reindex mutated canonical source or feedback",
            )
            for query in queries:
                base.require(
                    search(session, query) == before[query["id"]],
                    "reindex rollback differs",
                )
                base.require(
                    search(session, query, True, as_of) == enabled_results[query["id"]],
                    "reindex enabled result differs",
                )
            base.assert_preserved(sources, source_before)
        base.require(
            base.binary_hash(binary) == executable_hash,
            "binary changed during evaluation",
        )
        verify_locks()
        return {
            "schema_version": 1,
            "passed": True,
            "natural_language_ready": False,
            "ranking_default_enabled": False,
            "benefit_conclusion": "inconclusive_synthetic_only",
            "policy": "local-feedback-v1",
            "as_of": as_of,
            "fixture_hashes": locks,
            "executable_sha256": executable_hash,
            "harness_sha256": base.digest(Path(__file__).read_bytes()),
            "source_hashes": source_before,
            "canonical_hashes": snapshot,
            "synthetic_feedback": seeded,
            "queries": results,
            "splits": summarize(results),
            "checks": {
                "no_feedback_baseline": True,
                "rollback_exact": True,
                "reindex_preserved": True,
                "sources_preserved": True,
                "held_out_seeded": False,
            },
            "limitations": [
                "Fictional development-only samples; no productivity or independent-user evidence.",
                "Literal candidate generation remains unchanged; absent matches cannot be recovered.",
                "Passed checks validate contracts, not retrieval readiness or benefit.",
                "Five paired alternating MCP trials; CLI cross-check not included in latency distribution.",
            ],
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        report = evaluate(args.binary)
    except Exception as error:
        report = {
            "passed": False,
            "natural_language_ready": False,
            "error": f"{type(error).__name__}: {error}",
        }
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
