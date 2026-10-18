"""Frozen-corpus contract/non-regression gate; never promotes semantic readiness."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import tempfile

if __package__:
    from . import evaluate_local_memory as quality
    from . import evaluate_local_latency as latency
else:
    import evaluate_local_memory as quality
    import evaluate_local_latency as latency


DEFAULT_BASELINE = (
    Path(__file__).resolve().parents[1]
    / "crates/cortex-local/baselines/local-eval-v1.json"
)


def exact(actual, expected, message):
    quality.require(type(actual) is type(expected) and actual == expected, message)


def numeric(value):
    quality.require(
        type(value) in (int, float) and math.isfinite(value),
        "Nonfinite or nonnumeric metric",
    )
    return value


def metric_equal(actual, expected):
    if expected is None:
        exact(actual, None, "Empty judgment has numeric metric")
    else:
        quality.require(
            abs(numeric(actual) - expected) <= 1e-12,
            "Reported metric differs from raw evidence",
        )


def digest_string(value):
    quality.require(
        isinstance(value, str)
        and len(value) == 64
        and all(c in "0123456789abcdef" for c in value),
        "Invalid digest",
    )


def validate_quality(report, policy, fixtures):
    quality.require(not report.get("error"), "Quality report contains error")
    exact(report["schema_version"], 1, "Quality report schema")
    exact(
        report["fixture_manifest_sha256"],
        policy["fixture_manifest_sha256"],
        "Corpus manifest changed",
    )
    exact(
        report["fixture_hashes"],
        policy["fixture_hashes"],
        "Corpus source hashes changed",
    )
    exact(
        report["stable_projection_sha256"],
        policy["stable_projection_sha256"],
        "Canonical projection changed",
    )
    exact(report["reproducible"], True, "Cross-home reproduction failed")
    exact(report["namespace"], "local-eval-v1", "Namespace changed")
    homes = report["homes"]
    quality.require(isinstance(homes, list) and len(homes) == 2, "Two homes required")
    expected_ids = {k: r["id"] for k, r in fixtures["manifest"]["records"].items()}
    for home in homes:
        exact(home["sources_preserved"], True, "Sources not preserved")
        exact(home["rebuild_identical"], True, "Rebuild changed evidence")
        exact(home["record_ids"], expected_ids, "Home record coverage changed")
        exact(
            len(home["record_ids"]),
            policy["expected_record_count"],
            "Record count mismatch",
        )
        quality.require(
            len(home["imports"]) == 2 and len(home["repeat_imports"]) == 2,
            "Missing import reports",
        )
        for imported, repeated, count in zip(
            home["imports"], home["repeat_imports"], (18, 8)
        ):
            for item in (imported, repeated):
                exact(item["complete"], True, "Incomplete import")
                exact(item["rejected"], 0, "Rejected import")
                exact(item["errors"], [], "Import errors")
            exact(imported["imported"], count, "Import count mismatch")
            exact(repeated["imported"], 0, "Repeat import not idempotent")
        exact(home["reindex"]["records"], 30, "Reindex count mismatch")
        exact(home["reindex"]["revisions"], 30, "Reindex revision mismatch")
        for digest in home["source_hashes"].values():
            digest_string(digest)
    normalized_sources = [
        {k.replace("\\", "/"): v for k, v in h["source_hashes"].items()} for h in homes
    ]
    for original, normalized in zip(homes, normalized_sources):
        exact(len(original["source_hashes"]), 2, "Unexpected source snapshot count")
        exact(
            set(normalized),
            {"daily.sqlite", "archive/events.ndjson.zst"},
            "Source snapshot coverage differs",
        )
        exact(
            normalized["archive/events.ndjson.zst"],
            policy["fixture_hashes"]["events.ndjson.zst"],
            "Archive source changed",
        )
    exact(normalized_sources[0], normalized_sources[1], "Home source snapshots differ")
    for field, names_field in (
        ("isolation", "scope_check_names"),
        ("graph", "graph_check_names"),
    ):
        section = report[field]
        quality.require(not section.get("error"), "Contract suite error")
        exact(section["passed"], True, "Contract suite failed")
        checks = section["checks"]
        names = [c["name"] for c in checks]
        quality.require(
            len(names) == len(set(names)) and set(names) == set(policy[names_field]),
            "Contract check coverage differs",
        )
        for check in checks:
            exact(check["passed"], True, "Contract check failed")
            quality.require(not check.get("error"), "Contract check error")
    result = report["quality"]
    quality.require(not result.get("error"), "Retrieval section contains error")
    exact(result["integrity_ok"], True, "Retrieval integrity failed")
    queries = result["queries"]
    expected = {q["id"]: q for q in policy["query_floors"]}
    ids = [q["id"] for q in queries]
    quality.require(
        len(ids) == len(set(ids)) == 20 and set(ids) == set(expected),
        "Query coverage differs",
    )
    by_id = {r["id"]: r for r in fixtures["manifest"]["records"].values()}
    recomputed = []
    for query in queries:
        floor = expected[query["id"]]
        for field in (
            "id",
            "split",
            "repository",
            "text",
            "empty",
            "expected_keys",
            "expected_ids",
        ):
            exact(query[field], floor[field], "Frozen query contract differs")
        exact(query["integrity_ok"], True, "Query integrity failed")
        exact(query["diagnostics"], [], "Query diagnostics present")
        evidence = [
            {
                "id": fixtures["manifest"]["records"][k]["id"],
                "revision": fixtures["manifest"]["records"][k]["revision"],
                "source": fixtures["manifest"]["records"][k]["source"],
            }
            for k in query["expected_keys"]
        ]
        exact(query["expected_evidence"], evidence, "Expected source evidence differs")
        hits = query["retrieved"]
        quality.require(
            isinstance(hits, list) and len(hits) <= 10, "Invalid search result count"
        )
        seen, relevant = set(), []
        for rank, hit in enumerate(hits, 1):
            exact(hit["rank"], rank, "Rank sequence differs")
            exact(hit["valid"], True, "Invalid search evidence")
            exact(hit["error"], None, "Search evidence error")
            identity = hit["id"]
            quality.require(
                identity in by_id and identity not in seen, "Unknown or duplicate hit"
            )
            seen.add(identity)
            record = by_id[identity]
            exact(record["repository"], query["repository"], "Cross-scope hit")
            quality.require(
                record["authority"] not in ("superseded", "deprecated"), "Inactive hit"
            )
            exact(hit["revision"], record["revision"], "Wrong revision")
            digest_string(hit["content_sha256"])
            source = hit["source"]
            quality.require(
                set(source) == set(record["source"]) | {"locator"},
                "Source shape changed",
            )
            for field, value in record["source"].items():
                exact(source[field], value, "Source evidence differs")
            quality.require(
                isinstance(source["locator"], str) and source["locator"],
                "Missing source locator",
            )
            if identity in query["expected_ids"]:
                relevant.append(rank)
        if query["empty"]:
            exact(hits, [], "Empty query returned hits")
            recall, reciprocal = None, None
        else:
            recall = sum(rank <= 5 for rank in relevant) / len(query["expected_ids"])
            reciprocal = 1 / relevant[0] if relevant else 0.0
            quality.require(
                recall + 1e-12 >= numeric(floor["recall_at_5"])
                and reciprocal + 1e-12 >= numeric(floor["reciprocal_rank_at_10"]),
                "Per-query retrieval regressed",
            )
        metric_equal(query["recall_at_5"], recall)
        metric_equal(query["reciprocal_rank_at_10"], reciprocal)
        recomputed.append(
            dict(query, recall_at_5=recall, reciprocal_rank_at_10=reciprocal)
        )
    splits = {}
    exact(
        set(result["splits"]),
        {"development", "held_out", "all"},
        "Split coverage differs",
    )
    for split in ("development", "held_out", "all"):
        selected = [q for q in recomputed if split == "all" or q["split"] == split]
        answerable = [q for q in selected if not q["empty"]]
        empty_count = len(selected) - len(answerable)
        actual = result["splits"][split]
        for field, value in {
            "answerable_queries": len(answerable),
            "empty_queries": empty_count,
            "empty_queries_without_hits": empty_count,
            "integrity_failed_queries": 0,
            "integrity_ok": True,
        }.items():
            exact(actual[field], value, "Aggregate denominator/integrity differs")
        values = {
            "recall_at_5": sum(q["recall_at_5"] for q in answerable) / len(answerable),
            "mrr_at_10": sum(q["reciprocal_rank_at_10"] for q in answerable)
            / len(answerable),
        }
        for metric, value in values.items():
            metric_equal(actual[metric], value)
            quality.require(
                value + 1e-12 >= numeric(policy["split_floors"][split][metric]),
                "Aggregate retrieval regressed",
            )
        splits[split] = values
    return splits


def validate_latency(report, policy):
    quality.require(not report.get("error"), "Latency report contains error")
    environment = report["environment"]
    quality.require(isinstance(environment, dict), "Latency environment missing")
    for field in ("os", "python"):
        quality.require(
            isinstance(environment[field], str) and environment[field].strip(),
            "Latency software identity missing",
        )
    for field in ("machine", "processor"):
        value = environment[field]
        quality.require(
            value is None or isinstance(value, str) and value.strip(),
            "Invalid hardware availability",
        )
    identifier = environment.get("processor_identifier")
    quality.require(
        identifier is None or isinstance(identifier, str) and identifier.strip(),
        "Invalid processor identifier",
    )
    cpus = environment["logical_cpus"]
    quality.require(
        cpus is None or type(cpus) is int and cpus > 0, "Invalid CPU availability"
    )
    started, finished = (
        datetime.fromisoformat(report[field]) for field in ("started_at", "finished_at")
    )
    quality.require(
        all(
            value.utcoffset() is not None and value.utcoffset().total_seconds() == 0
            for value in (started, finished)
        )
        and started <= finished,
        "Latency timestamps must be ordered UTC instants",
    )
    exact(report["schema_version"], 1, "Latency report schema")
    exact(report["passed"], True, "Latency report failed")
    exact(
        report["corpus"]["manifest_sha256"],
        policy["fixture_manifest_sha256"],
        "Latency corpus differs",
    )
    exact(
        report["corpus"]["files"],
        policy["fixture_hashes"],
        "Latency source hashes differ",
    )
    section = report["latency"]
    quality.require(not section.get("error"), "Latency section contains error")
    exact(section["repository"], "Atlas", "Latency scope differs")
    exact(section["query"], "offline desktop notes", "Latency workload differs")
    exact(section["passed"], True, "Latency measurement failed")
    exact(
        section["binary_sources_and_original_records_preserved"],
        True,
        "Latency modified original evidence",
    )
    exact(section["concurrency"], 1, "Concurrency differs")
    exact(section["clock"], "time.perf_counter_ns", "Clock differs")
    exact(
        set(section["series"]),
        set(policy["latency"]["series"]),
        "Latency series coverage differs",
    )
    for name, series in section["series"].items():
        samples = series["samples"]
        quality.require(
            isinstance(samples, list)
            and policy["latency"]["min_samples"] <= len(samples) <= 100,
            "Insufficient or excessive latency samples",
        )
        quality.require(
            not any(
                series.get(k)
                for k in ("error", "setup_error", "cleanup_error", "warmup_error")
            ),
            "Latency setup/cleanup failed",
        )
        for ordinal, sample in enumerate(samples, 1):
            exact(sample["ordinal"], ordinal, "Sample order differs")
            exact(sample["success"], True, "Latency sample failed")
            quality.require(
                type(sample["elapsed_ns"]) is int and sample["elapsed_ns"] > 0,
                "Invalid elapsed time",
            )
            quality.require(not sample.get("error"), "Sample contains error")
        for field in ("planned", "measured", "successes"):
            exact(series[field], len(samples), "Latency sample count inconsistent")
        exact(
            section["samples_per_series"], len(samples), "Series sample counts differ"
        )
        exact(series["failures"], 0, "Latency failures")
        exact(series["unmeasured"], 0, "Unmeasured samples")
        durations = sorted(s["elapsed_ns"] for s in samples)
        for field, percentile in (("p50_ns", 0.5), ("p95_ns", 0.95)):
            exact(
                series[field],
                durations[math.ceil(percentile * len(durations)) - 1],
                "Latency percentile inconsistent",
            )
        quality.require(
            series["p95_ns"] <= policy["latency"]["series"][name]["p95_max_ns"],
            "Latency ceiling exceeded",
        )


def assess(quality_report, latency_report, policy):
    result = {
        "passed": False,
        "checks": [],
        "natural_language_ready": False,
        "scope": "Contract/non-regression only; a green gate does not establish natural-language readiness.",
    }

    def check(name, operation):
        try:
            value = operation()
            result["checks"].append({"name": name, "passed": True})
            return value
        except Exception:
            result["checks"].append(
                {
                    "name": name,
                    "passed": False,
                    "error": "Missing, malformed or regressed evidence",
                }
            )
            return None

    def contracts():
        exact(policy["schema_version"], 1, "Policy schema")
        exact(
            policy["required_build"],
            {
                "runtime": "standalone",
                "release": True,
                "install_protocol": 1,
                "required_services": [],
            },
            "Policy build contract missing",
        )
        for field, count in (("scope_check_names", 61), ("graph_check_names", 49)):
            quality.require(
                len(policy[field]) == len(set(policy[field])) == count,
                "Policy check coverage differs",
            )
        quality.require(
            type(policy["latency"]["min_samples"]) is int
            and 20 <= policy["latency"]["min_samples"] <= 100,
            "Policy latency samples invalid",
        )
        exact(policy["latency"]["concurrency"], 1, "Policy concurrency differs")
        fixtures = quality.load_fixtures(quality.DEFAULT_FIXTURES)
        exact(
            fixtures["manifest_sha256"],
            policy["fixture_manifest_sha256"],
            "Policy corpus changed",
        )
        exact(
            fixtures["manifest"]["files"],
            policy["fixture_hashes"],
            "Policy hashes differ",
        )
        exact(len(policy["query_floors"]), 20, "Policy query count")
        frozen_queries = {q["id"]: q for q in fixtures["corpus"]["queries"]}
        exact(
            {q["id"] for q in policy["query_floors"]},
            set(frozen_queries),
            "Policy query IDs differ",
        )
        for floor in policy["query_floors"]:
            for field in (
                "id",
                "split",
                "repository",
                "text",
                "empty",
                "expected_keys",
            ):
                exact(
                    floor[field],
                    frozen_queries[floor["id"]][field],
                    "Policy query changed",
                )
            exact(
                floor["expected_ids"],
                [
                    fixtures["manifest"]["records"][k]["id"]
                    for k in floor["expected_keys"]
                ],
                "Policy expected IDs differ",
            )
        target = policy["natural_language_readiness"]
        exact(target["split"], "held_out", "Readiness split differs")
        exact(
            target["required_for_non_regression_gate"],
            False,
            "Readiness conflated with non-regression",
        )
        for field in ("recall_at_5_min", "mrr_at_10_min"):
            quality.require(
                0 <= numeric(target[field]) <= 1, "Invalid readiness target"
            )
        for report in (quality_report, latency_report):
            digest_string(report["binary"]["sha256"])
            for field, value in policy["required_build"].items():
                exact(
                    report["binary"]["build_info"][field],
                    value,
                    "Non-release standalone build",
                )
        exact(
            quality_report["binary"]["sha256"],
            latency_report["binary"]["sha256"],
            "Candidate binaries differ",
        )
        return fixtures

    fixtures = check("policy-corpus-and-build", contracts)
    splits = (
        check(
            "retrieval-reproducibility-and-contracts",
            lambda: validate_quality(quality_report, policy, fixtures),
        )
        if fixtures
        else None
    )
    check(
        "latency-contract-and-ceilings",
        lambda: validate_latency(latency_report, policy),
    )
    if splits:
        target = policy["natural_language_readiness"]
        observed = splits[target["split"]]
        result["readiness"] = {
            "observed": observed,
            "targets": target,
            "limitation": "Held-out zero floors preserve the existing literal-search limitation; they do not prove useful natural-language retrieval.",
        }
        result["natural_language_ready"] = (
            observed["recall_at_5"] >= target["recall_at_5_min"]
            and observed["mrr_at_10"] >= target["mrr_at_10_min"]
        )
    result["passed"] = (
        bool(fixtures)
        and len(result["checks"]) == 3
        and all(c["passed"] for c in result["checks"])
    )
    return result


def read_report(path):
    quality.require(
        Path(path).stat().st_size <= 16 * 1024 * 1024, "Report exceeds bound"
    )
    return quality.parse_json(Path(path).read_bytes())


def evaluate(
    binary,
    report_path,
    baseline_path=DEFAULT_BASELINE,
    *,
    quality_report_path=None,
    latency_report_path=None,
):
    report = {
        "schema_version": 1,
        "started_at": datetime.now(timezone.utc).isoformat(),
        "passed": False,
    }
    report["mode"] = (
        "historical-report assessment"
        if quality_report_path or latency_report_path
        else "fresh execution"
    )
    report["evidence_scope"] = (
        "Supplied reports are assessed claims, not independent execution proof; default fresh mode runs both evaluators."
    )
    try:
        policy = read_report(baseline_path)
        report["baseline_sha256"] = quality.digest(Path(baseline_path).read_bytes())
        quality.require(
            bool(quality_report_path) == bool(latency_report_path),
            "Supply both reports or neither",
        )
        if quality_report_path:
            report["input_reports"] = {
                "quality": str(quality_report_path),
                "latency": str(latency_report_path),
            }
            qreport = read_report(quality_report_path)
            report["quality_report"] = qreport
            lreport = read_report(latency_report_path)
        else:
            qreport = quality.evaluate(binary)
            report["quality_report"] = qreport
            with tempfile.TemporaryDirectory(prefix="cortex-gate-report-") as scratch:
                lreport = latency.evaluate(
                    binary,
                    Path(scratch) / "latency.json",
                    samples=policy["latency"]["min_samples"],
                )
        report["quality_report"], report["latency_report"] = qreport, lreport
        candidate_hash = quality.binary_hash(Path(binary))
        quality.require(
            qreport["binary"]["sha256"]
            == candidate_hash
            == lreport["binary"]["sha256"],
            "Reports do not match requested binary",
        )
        report["assessment"] = assess(qreport, lreport, policy)
        report["passed"] = report["assessment"]["passed"]
    except Exception:
        report["error"] = (
            "Gate setup or report validation failed; no policy changes performed"
        )
    finally:
        report["finished_at"] = datetime.now(timezone.utc).isoformat()
        Path(report_path).write_text(
            json.dumps(report, indent=2) + "\n", encoding="utf-8"
        )
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--quality-report", type=Path)
    parser.add_argument("--latency-report", type=Path)
    args = parser.parse_args()
    report = evaluate(
        args.binary,
        args.report,
        args.baseline,
        quality_report_path=args.quality_report,
        latency_report_path=args.latency_report,
    )
    assessment = report.get("assessment", {})
    ready = (
        assessment.get("natural_language_ready") if "readiness" in assessment else None
    )
    readiness = (
        "ready" if ready is True else "not ready" if ready is False else "unknown"
    )
    status = "passed" if report["passed"] else "failed"
    print(
        f"Contract/non-regression: {status}. Natural-language readiness: {readiness}."
    )
    print(f"Report: {args.report}")
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
