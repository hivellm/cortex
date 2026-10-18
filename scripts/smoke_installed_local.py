"""Source-preserving installed smoke with isolated fixtures and audit receipts."""

from datetime import datetime, timezone
import argparse
import json
from pathlib import Path
import re
import tempfile

if __package__:
    from . import evaluate_local_memory as evaluation
    from .evaluate_local_latency import hook_expected, run_hook
    from .local_eval_transport import CortexMcp
else:
    import evaluate_local_memory as evaluation
    from evaluate_local_latency import hook_expected, run_hook
    from local_eval_transport import CortexMcp


ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs/operations/local-first-memory.md"
DISPATCH = ROOT / "crates/cortex-local/src/mcp.rs"
TOOLS = {
    "cortex_feedback_record",
    "cortex_feedback_get",
    "cortex_feedback_list",
    "cortex_audit_list",
    "cortex_audit_get",
    "cortex_audit_metrics",
    "cortex_audit_retain",
    "cortex_search",
    "cortex_query",
    "cortex_get",
    "cortex_capture_memory",
    "cortex_revise",
    "cortex_graph",
    "cortex_status",
    "cortex_pre_thinking",
    "cortex_memory_read",
    "cortex_classification_enqueue",
    "cortex_classification_jobs",
}
READONLY = TOOLS - {
    "cortex_feedback_record",
    "cortex_audit_retain",
    "cortex_capture_memory",
    "cortex_revise",
    "cortex_classification_enqueue",
}
SCOPES = ("Atlas", "atlas", "AtlasLab", "AtlasProduction")
QUERY = "offline desktop notes"


def tool_contract(document, source):
    documented = re.findall(r"(?m)^\| `(cortex_[a-z_]+)` \|", document)
    body = (
        source.split("pub fn call(", 1)[1]
        .split("pub fn run(", 1)[0]
        .rsplit("match name {", 1)[1]
    )
    arms = re.findall(r'(?m)^\s*((?:"cortex_[a-z_]+"(?:\s*\|\s*)?)+)\s*=>', body)
    implemented = [
        name for arm in arms for name in re.findall(r'"(cortex_[a-z_]+)"', arm)
    ]
    evaluation.require(
        len(documented) == len(set(documented)) == len(TOOLS)
        and set(documented) == TOOLS,
        "Documented toolset differs",
    )
    evaluation.require(
        len(implemented) == len(set(implemented)) == len(TOOLS)
        and set(implemented) == TOOLS,
        "Dispatch toolset differs",
    )
    return {"documented": sorted(documented), "implemented": sorted(implemented)}


def tool_names(descriptors):
    evaluation.require(
        isinstance(descriptors, list), "Tool descriptors must be an array"
    )
    names = [item["name"] for item in descriptors]
    evaluation.require(
        len(names) == len(set(names)) == len(TOOLS) and set(names) == TOOLS,
        "Advertised toolset differs",
    )
    return sorted(names)


def canonical_snapshot(home):
    local = home / "local"
    result = {}
    for subtree in ("records", "segments", "feedback"):
        for path in sorted((local / subtree).rglob("*")):
            evaluation.require(not path.is_symlink(), "Canonical symlink unexpected")
            if path.is_file():
                result[path.relative_to(local).as_posix()] = evaluation.digest(
                    path.read_bytes()
                )
    for path in sorted((local / "classification/jobs").rglob("*.json")):
        evaluation.require(not path.is_symlink(), "Queue symlink unexpected")
        result[path.relative_to(local).as_posix()] = evaluation.digest(
            path.read_bytes()
        )
    return result


def validate_graph_pair(
    cli_result, mcp_result, repository, expected_records, expected_edges, sources
):
    for result in (cli_result, mcp_result):
        evaluation.validate_graph_result(
            result, repository, expected_records, expected_edges, sources
        )
    evaluation.require(cli_result == mcp_result, "Installed CLI/MCP graph mismatch")


def evaluate(installed_binary, reference_binary, report_path):
    report = {
        "schema_version": 1,
        "started_at": datetime.now(timezone.utc).isoformat(),
        "passed": False,
        "checks": [],
        "delivery_consumption": "not_observed",
        "scope": "Installed runtime with explicit temporary home. Source-preserving tools, retention preview and SessionStart preserve memory sources and classification jobs; recall may publish local audit receipts. No real memory store, provider or client-consumption claim.",
        "source_paths": {"documentation": str(DOCS), "dispatch": str(DISPATCH)},
        "snapshot_scope": "Canonical records/segments and queue JSON plus source fixture bytes; derived SQLite files and lock files excluded because opening readers may change them.",
    }

    def check(name, action):
        try:
            action()
            report["checks"].append({"name": name, "passed": True})
        except Exception:
            report["checks"].append(
                {
                    "name": name,
                    "passed": False,
                    "error": "Installed contract, transport or evidence validation failed",
                }
            )

    def equal(actual, expected):
        evaluation.require(actual == expected, "Installed smoke mismatch")

    installed, reference = (
        Path(installed_binary).resolve(),
        Path(reference_binary).resolve(),
    )
    before = None
    try:
        before = {
            "installed": evaluation.binary_hash(installed),
            "reference": evaluation.binary_hash(reference),
        }
        report["binary_hashes_before"] = before
        equal(before["installed"], before["reference"])
        report["tool_contract"] = tool_contract(
            DOCS.read_text(encoding="utf-8"), DISPATCH.read_text(encoding="utf-8")
        )
        with tempfile.TemporaryDirectory(prefix="cortex-installed-smoke-") as scratch:
            root = Path(scratch)
            installed_info = evaluation.CortexCli(
                installed, root / "info-installed"
            ).run("build-info")
            reference_info = evaluation.CortexCli(
                reference, root / "info-reference"
            ).run("build-info")
            equal(installed_info, reference_info)
            for key, value in {
                "runtime": "standalone",
                "release": True,
                "install_protocol": 1,
                "required_services": [],
            }.items():
                equal(installed_info[key], value)
            report["build_info_before"] = {
                "installed": installed_info,
                "reference": reference_info,
            }
            fixtures = evaluation.load_fixtures(evaluation.DEFAULT_FIXTURES)
            report["fixture_manifest_sha256"] = fixtures["manifest_sha256"]
            built = evaluation.build_home(installed, fixtures, root / "fixture")
            cli = built["cli"]
            snapshot = canonical_snapshot(cli.home)
            evaluation.require(bool(snapshot), "Canonical snapshot empty")
            report["canonical_before"] = snapshot
            report["sources_before"] = built["report"]["source_hashes"]
            touched = set()
            records = fixtures["manifest"]["records"]
            by_id = {r["id"]: r for r in records.values()}
            try:
                cli_tools = cli.run("mcp", "--list-tools")
                report["advertised_tools"] = tool_names(cli_tools)
                with CortexMcp(installed, cli.home) as mcp:
                    wire_tools = mcp.list_tools()["tools"]
                    tool_names(wire_tools)
                    equal(cli_tools, wire_tools)

                    def call(name, args):
                        evaluation.require(
                            name in READONLY, "Mutation prohibited after fixture setup"
                        )
                        touched.add(name)
                        return mcp.call(name, args)

                    for repository in SCOPES:

                        def scoped(repository=repository):
                            expected_status = evaluation.scoped_status(
                                fixtures["manifest"], repository
                            )
                            equal(
                                cli.run("--repository", repository, "status"),
                                expected_status,
                            )
                            equal(
                                call("cortex_status", {"repository": repository}),
                                expected_status,
                            )
                            equal(
                                call(
                                    "cortex_feedback_list", {"repository": repository}
                                ),
                                {"entries": [], "next_cursor": None},
                            )
                            equal(
                                call(
                                    "cortex_feedback_get",
                                    {"repository": repository, "id": "absent-feedback"},
                                ),
                                None,
                            )
                            hits = cli.run(
                                "--repository",
                                repository,
                                "search",
                                QUERY,
                                "--limit",
                                "10",
                            )
                            evaluation.validate_scope_hits(
                                hits, repository, fixtures["manifest"], built["sources"]
                            )
                            if repository != "AtlasProduction":
                                evaluation.require(
                                    bool(hits), "Positive search missing"
                                )
                            else:
                                equal(hits, [])
                            for name in ("cortex_search", "cortex_query"):
                                equal(
                                    call(
                                        name,
                                        {
                                            "repository": repository,
                                            "query": QUERY,
                                            "limit": 10,
                                        },
                                    ),
                                    hits,
                                )
                            expected_context = {
                                "repository": repository,
                                "context": evaluation.expected_recall_context(hits),
                                "delivery_consumption": "not_observed",
                            }
                            for name in ("cortex_memory_read", "cortex_pre_thinking"):
                                evaluation.validate_recall_response(
                                    call(
                                        name, {"repository": repository, "query": QUERY}
                                    ),
                                    expected_context,
                                )
                            page = call("cortex_audit_list", {"repository": repository})
                            evaluation.require(
                                bool(page["entries"]), "Recall receipts missing"
                            )
                            for entry in page["entries"]:
                                record = entry["record"]
                                equal(record["repository"], repository)
                                equal(
                                    call(
                                        "cortex_audit_get",
                                        {"repository": repository, "id": record["id"]},
                                    ),
                                    record,
                                )
                            metrics = call(
                                "cortex_audit_metrics", {"repository": repository}
                            )
                            evaluation.require(
                                metrics["deliveries"]["attempts"] >= 2,
                                "Recall denominator missing",
                            )
                            preview = mcp.call(
                                "cortex_audit_retain",
                                {
                                    "repository": repository,
                                    "cutoff": "2100-01-01T00:00:00Z",
                                },
                            )
                            evaluation.require(
                                preview["control"] is None,
                                "Retention preview mutated archive state",
                            )
                            evaluation.require(
                                preview["canonical_files_retained"] is True,
                                "Retention preview lost sources",
                            )
                            equal(
                                call("cortex_audit_list", {"repository": repository}),
                                page,
                            )
                            foreign_scope = (
                                "Atlas" if repository != "Atlas" else "atlas"
                            )
                            equal(
                                call(
                                    "cortex_audit_get",
                                    {
                                        "repository": foreign_scope,
                                        "id": page["entries"][0]["record"]["id"],
                                    },
                                ),
                                None,
                            )
                            own = next(
                                (
                                    r
                                    for r in records.values()
                                    if r["repository"] == repository
                                ),
                                None,
                            )
                            if own:
                                record = cli.run(
                                    "--repository",
                                    repository,
                                    "get",
                                    own["id"],
                                    "--revision",
                                    "1",
                                )
                                evaluation.validate_record(
                                    record, own, built["sources"]
                                )
                                equal(
                                    call(
                                        "cortex_get",
                                        {
                                            "repository": repository,
                                            "id": own["id"],
                                            "revision": 1,
                                        },
                                    ),
                                    record,
                                )
                            foreign = next(
                                r
                                for r in records.values()
                                if r["repository"] != repository
                            )
                            equal(
                                call(
                                    "cortex_get",
                                    {"repository": repository, "id": foreign["id"]},
                                ),
                                None,
                            )
                            graph = cli.run(
                                "--repository",
                                repository,
                                "graph",
                                "graph-report",
                                "--depth",
                                "2",
                                "--limit",
                                "20",
                            )
                            mcp_graph = call(
                                "cortex_graph",
                                {
                                    "repository": repository,
                                    "id": "graph-report",
                                    "depth": 2,
                                    "limit": 20,
                                },
                            )
                            expected_nodes = (
                                {"graph-report", "graph-log", "graph-test"}
                                if repository == "Atlas"
                                else set()
                            )
                            expected_edges = (
                                [
                                    {
                                        "source_id": "graph-report",
                                        "target_id": "graph-log",
                                        "relation": "derived_from",
                                    },
                                    {
                                        "source_id": "graph-log",
                                        "target_id": "graph-test",
                                        "relation": "supports",
                                    },
                                ]
                                if repository == "Atlas"
                                else []
                            )
                            validate_graph_pair(
                                graph,
                                mcp_graph,
                                repository,
                                {
                                    identity: by_id[identity]
                                    for identity in expected_nodes
                                },
                                expected_edges,
                                built["sources"],
                            )
                            empty_jobs = {
                                "jobs": [],
                                "malformed": 0,
                                "scanned": 0,
                                "incomplete": False,
                            }
                            equal(
                                cli.run("--repository", repository, "classify", "jobs"),
                                empty_jobs,
                            )
                            equal(
                                call(
                                    "cortex_classification_jobs",
                                    {"repository": repository},
                                ),
                                empty_jobs,
                            )

                        check("readonly-scope:" + repository, scoped)
                    report["readonly_tools_exercised"] = sorted(touched)
                    check("readonly-tool-coverage", lambda: equal(touched, READONLY))
                repo = root / "repo"
                repo.mkdir()
                (repo / "cortex.toml").write_text(
                    '[cortex]\nid = "Atlas"\n', encoding="utf-8"
                )
                baseline = cli.run(
                    "--repository", "Atlas", "search", QUERY, "--limit", "10"
                )
                expected = hook_expected(built, baseline)["SessionStart"]
                check(
                    "installed-SessionStart-context",
                    lambda: equal(
                        run_hook(
                            installed,
                            cli.home,
                            {
                                "cwd": str(repo),
                                "session_id": "fictional-read-only-smoke",
                            },
                            "SessionStart",
                        ),
                        {
                            "hookSpecificOutput": {
                                "hookEventName": "SessionStart",
                                "additionalContext": expected,
                            }
                        },
                    ),
                )
            finally:
                report["canonical_after"] = canonical_snapshot(cli.home)
                check(
                    "canonical-and-queue-unchanged",
                    lambda: equal(report["canonical_after"], snapshot),
                )
                check(
                    "source-files-unchanged",
                    lambda: evaluation.assert_preserved(
                        built["sources"], built["report"]["source_hashes"]
                    ),
                )
                for repository in SCOPES:
                    check(
                        "final-status:" + repository,
                        lambda repository=repository: equal(
                            cli.run("--repository", repository, "status"),
                            evaluation.scoped_status(fixtures["manifest"], repository),
                        ),
                    )
                after_info = {
                    "installed": evaluation.CortexCli(
                        installed, root / "info-installed"
                    ).run("build-info"),
                    "reference": evaluation.CortexCli(
                        reference, root / "info-reference"
                    ).run("build-info"),
                }
                report["build_info_after"] = after_info
                check(
                    "build-identity-unchanged",
                    lambda: equal(after_info, report["build_info_before"]),
                )
    except Exception:
        report["error"] = (
            "Installed smoke setup, advertised surface or bounded transport failed"
        )
    finally:
        if before is not None:
            try:
                report["binary_hashes_after"] = {
                    "installed": evaluation.binary_hash(installed),
                    "reference": evaluation.binary_hash(reference),
                }
                check(
                    "binary-bytes-unchanged",
                    lambda: equal(report["binary_hashes_after"], before),
                )
            except Exception:
                report["checks"].append(
                    {"name": "binary-bytes-unchanged", "passed": False}
                )
        report["passed"] = (
            not report.get("error")
            and bool(report["checks"])
            and all(c["passed"] for c in report["checks"])
        )
        report["finished_at"] = datetime.now(timezone.utc).isoformat()
        Path(report_path).write_text(
            json.dumps(report, indent=2) + "\n", encoding="utf-8"
        )
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installed-binary", required=True, type=Path)
    parser.add_argument("--reference-binary", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    result = evaluate(args.installed_binary, args.reference_binary, args.report)
    print(
        "Installed smoke report written; "
        + ("checks passed." if result["passed"] else "checks failed.")
    )
    raise SystemExit(0 if result["passed"] else 1)


if __name__ == "__main__":
    main()
