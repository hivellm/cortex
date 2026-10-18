"""Construction, subprocess boundaries and fixed retrieval scoring contracts."""

import copy
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

MODULE = Path(__file__).resolve().parents[1] / "evaluate_local_memory.py"
spec = importlib.util.spec_from_file_location("scripts.evaluate_local_memory", MODULE)
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)


class FixtureValidationTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="cortex-eval-test-")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.fixtures = self.root / "fixtures"
        shutil.copytree(evaluation.DEFAULT_FIXTURES, self.fixtures)

    def modify_json(self, name, mutate, repin=False):
        path = self.fixtures / name
        value = json.loads(path.read_text(encoding="utf-8"))
        mutate(value)
        path.write_text(json.dumps(value), encoding="utf-8")
        if repin:
            self.modify_json(
                "manifest.json",
                lambda manifest: manifest["files"].__setitem__(
                    name, evaluation.digest(path.read_bytes())
                ),
            )

    def test_changed_fixture_bytes_rejected_before_import(self):
        with (self.fixtures / "events.ndjson.zst").open("ab") as stream:
            stream.write(b"altered")
        with self.assertRaisesRegex(evaluation.EvaluationError, "file hash mismatch"):
            evaluation.load_fixtures(self.fixtures)

    def test_unknown_schema_rejected(self):
        self.modify_json(
            "manifest.json", lambda value: value.__setitem__("schema_version", 2)
        )
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "unsupported manifest schema"
        ):
            evaluation.load_fixtures(self.fixtures)

    def test_duplicate_json_keys_not_silently_overwritten(self):
        with self.assertRaisesRegex(evaluation.EvaluationError, "duplicate JSON key"):
            evaluation.parse_json('{"schema_version":1,"schema_version":2}')

    def test_duplicate_imported_ids_rejected(self):
        def mutate(value):
            values = list(value["records"].values())
            values[1]["id"] = values[0]["id"]

        self.modify_json("manifest.json", mutate)
        with self.assertRaisesRegex(evaluation.EvaluationError, "duplicate record ID"):
            evaluation.load_fixtures(self.fixtures)

    def test_cross_scope_judgment_rejected_even_with_rehashed_corpus(self):
        self.modify_json(
            "corpus.json",
            lambda value: value["queries"][0].__setitem__("repository", "atlas"),
            repin=True,
        )
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "invalid relevance scope"
        ):
            evaluation.load_fixtures(self.fixtures)

    def test_manifest_evidence_hash_checked_independently(self):
        self.modify_json(
            "manifest.json",
            lambda value: value["records"]["atlas-storage"]["source"].__setitem__(
                "source_hash", "0" * 64
            ),
        )
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "evidence hash mismatch"
        ):
            evaluation.load_fixtures(self.fixtures)

    def test_source_materialization_preserves_exact_json_strings(self):
        fixtures = evaluation.load_fixtures(self.fixtures)
        root = self.root / "sources"
        before = evaluation.materialize_sources(fixtures, root)
        with closing(sqlite3.connect(root / "daily.sqlite")) as connection:
            snapshots = dict(
                connection.execute("SELECT id,data FROM daily_memory_records")
            )
            evidence = dict(
                connection.execute("SELECT event_id,data FROM daily_memory_evidence")
            )
        for row in fixtures["daily"]:
            self.assertEqual(snapshots[row["id"]], row["snapshot_raw"])
            self.assertEqual(evidence[row["event_id"]], row["evidence_raw"])
        evaluation.assert_preserved(root, before)
        (root / "archive/events.ndjson.zst").write_bytes(b"changed")
        with self.assertRaisesRegex(evaluation.EvaluationError, "source files changed"):
            evaluation.assert_preserved(root, before)

    def test_rebuild_guard_rejects_changed_provenance(self):
        before = {"record": {"source": {"source_hash": "first"}, "body": "same"}}
        after = copy.deepcopy(before)
        after["record"]["source"]["source_hash"] = "second"
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "reindex changed canonical"
        ):
            evaluation.assert_rebuild(before, after)

    def test_existing_destination_not_overwritten(self):
        fixtures = evaluation.load_fixtures(self.fixtures)
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "source destination must be new"
        ):
            evaluation.materialize_sources(fixtures, self.root)

    def test_materialization_uses_validated_snapshot_after_disk_changes(self):
        fixtures = evaluation.load_fixtures(self.fixtures)
        expected = fixtures["raw"]["events.ndjson.zst"]
        (self.fixtures / "events.ndjson.zst").write_bytes(
            b"replacement after validation"
        )
        destination = self.root / "sources"
        evaluation.materialize_sources(fixtures, destination)
        self.assertEqual(
            (destination / "archive/events.ndjson.zst").read_bytes(), expected
        )

    def test_changed_binary_rejected_with_runtime_only_mocked(self):
        binary = self.root / "candidate"
        binary.write_bytes(b"original binary")

        def fake_build(*_args):
            binary.write_bytes(b"replaced binary")
            return {"projection": {}, "report": {}}

        with (
            mock.patch.object(
                evaluation.CortexCli, "run", return_value={"standalone": True}
            ),
            mock.patch.object(evaluation, "build_home", side_effect=fake_build),
        ):
            with self.assertRaisesRegex(evaluation.EvaluationError, "binary changed"):
                evaluation.evaluate(binary, self.fixtures)


class ReportFailureTests(unittest.TestCase):
    def test_integrity_failure_report_written_before_nonzero_exit(self):
        with tempfile.TemporaryDirectory(prefix="cortex-eval-report-") as scratch:
            report_path = Path(scratch) / "report.json"
            report = {
                "quality": {
                    "integrity_ok": False,
                    "queries": [{"diagnostics": ["wrong source"]}],
                }
            }
            argv = ["evaluate", "--binary", "unused", "--report", str(report_path)]
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(evaluation, "evaluate", return_value=report),
            ):
                with self.assertRaises(SystemExit) as raised:
                    evaluation.main()
            self.assertEqual(raised.exception.code, 1)
            self.assertEqual(json.loads(report_path.read_text()), report)


class SubprocessBoundaryTests(unittest.TestCase):
    def run_controlled(self, code, **options):
        # Adapt only the command prefix; real pipes, process lifetime and limits run.
        real_popen = subprocess.Popen

        def adapted(_command, **kwargs):
            return real_popen([sys.executable, "-c", code], **kwargs)

        with tempfile.TemporaryDirectory(prefix="cortex-eval-process-") as scratch:
            cli = evaluation.CortexCli(
                sys.executable, Path(scratch) / "home", **options
            )
            with mock.patch.object(evaluation.subprocess, "Popen", side_effect=adapted):
                return cli.run("build-info")

    def test_timeout_kills_controlled_process(self):
        with self.assertRaisesRegex(evaluation.EvaluationError, "deadline exceeded"):
            self.run_controlled("import time; time.sleep(5)", timeout=0.1)

    def test_nonzero_exit_preserves_bounded_diagnostic(self):
        with self.assertRaisesRegex(
            evaluation.EvaluationError, r"CLI failed \(7\): fixture error"
        ):
            self.run_controlled(
                "import sys; sys.stderr.write('fixture error'); sys.exit(7)"
            )

    def test_stdout_overflow_is_rejected(self):
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "output limit exceeded"
        ):
            self.run_controlled(
                "import sys; sys.stdout.write('x' * 20000); sys.stdout.flush()",
                output_limit=1024,
            )

    def test_stderr_overflow_is_rejected(self):
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "output limit exceeded"
        ):
            self.run_controlled(
                "import sys; sys.stderr.write('x' * 20000); sys.stderr.flush()",
                output_limit=1024,
            )


@unittest.skipUnless(
    os.environ.get("CORTEX_EVAL_BINARY"),
    "set CORTEX_EVAL_BINARY for real CLI integration",
)
class RealBinaryTests(unittest.TestCase):
    def test_two_real_homes_import_repeat_rebuild_and_match(self):
        report = evaluation.evaluate(Path(os.environ["CORTEX_EVAL_BINARY"]))
        self.assertTrue(report["reproducible"])
        self.assertEqual(len(report["homes"]), 2)
        for home in report["homes"]:
            self.assertEqual(len(home["record_ids"]), 30)
            self.assertTrue(home["rebuild_identical"])
            self.assertTrue(home["sources_preserved"])
            self.assertEqual([r["imported"] for r in home["imports"]], [18, 8])
            self.assertEqual([r["imported"] for r in home["repeat_imports"]], [0, 0])


class RetrievalScoringTests(unittest.TestCase):
    def setUp(self):
        self.records = []
        self.manifest = {"records": {}}
        for n in range(8):
            record = {
                "schema_version": 1,
                "id": f"r{n}",
                "repository": "Atlas",
                "revision": 1,
                "kind": "note",
                "authority": "accepted",
                "title": f"Note {n}",
                "body": "Fixture text",
                "links": [],
                "metadata": {},
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "content_sha256": "",
                "source": {
                    "kind": "fixture",
                    "locator": f"fiction://r{n}",
                    "source_id": f"s{n}",
                    "captured_at": None,
                    "source_hash": str(n) * 64,
                    "payload": {},
                },
            }
            record["content_sha256"] = evaluation.digest(evaluation.canonical(record))
            self.records.append(record)
            self.manifest["records"][f"r{n}"] = copy.deepcopy(record)

    def score(self, expected, records):
        query = {
            "id": "test",
            "split": "development",
            "intent": "test",
            "repository": "Atlas",
            "text": "fixture",
            "expected_keys": expected,
            "empty": not expected,
        }
        return evaluation.score_query(
            query, [{"record": r} for r in records], self.manifest, Path(".")
        )

    def test_recall_denominator_includes_relevant_items_beyond_five(self):
        result = self.score([f"r{n}" for n in range(8)], self.records[:5])
        self.assertEqual(result["recall_at_5"], 5 / 8)
        self.assertEqual(result["reciprocal_rank_at_10"], 1)

    def test_duplicates_never_move_sixth_hit_into_top_five(self):
        result = self.score(["r0", "r1"], [self.records[0]] * 5 + [self.records[1]])
        self.assertEqual(result["recall_at_5"], 0.5)
        self.assertEqual(result["reciprocal_rank_at_10"], 1)
        self.assertFalse(result["integrity_ok"])
        self.assertEqual(result["retrieved"][-1]["rank"], 6)

    def test_missing_or_wrong_evidence_gets_no_credit(self):
        for mode in ("missing", "wrong"):
            with self.subTest(mode=mode):
                record = copy.deepcopy(self.records[0])
                if mode == "missing":
                    del record["source"]
                else:
                    record["source"]["source_hash"] = "wrong"
                result = self.score(["r0"], [record])
                self.assertEqual(result["recall_at_5"], 0)
                self.assertEqual(result["reciprocal_rank_at_10"], 0)
                self.assertFalse(result["integrity_ok"])

    def test_wrong_scope_invalid_even_when_identity_is_relevant(self):
        record = copy.deepcopy(self.records[0])
        record["repository"] = "atlas"
        result = self.score(["r0"], [record])
        self.assertFalse(result["integrity_ok"])
        self.assertEqual(result["recall_at_5"], 0)

    def test_empty_expectations_excluded_from_recall_and_mrr(self):
        for records in ([], self.records[:1]):
            result = self.score([], records)
            self.assertIsNone(result["recall_at_5"])
            self.assertIsNone(result["reciprocal_rank_at_10"])
            self.assertTrue(result["integrity_ok"])
            self.assertEqual(len(result["retrieved"]), len(records))

    def test_proposal_can_be_relevant_but_superseded_is_invalid(self):
        for authority in ("proposed", "superseded"):
            record = copy.deepcopy(self.records[0])
            record["authority"] = authority
            record["content_sha256"] = ""
            record["content_sha256"] = evaluation.digest(evaluation.canonical(record))
            self.manifest["records"]["r0"] = copy.deepcopy(record)
            result = self.score(["r0"], [record])
            self.assertEqual(result["integrity_ok"], authority == "proposed")
            self.assertEqual(result["recall_at_5"], 1 if authority == "proposed" else 0)

    def test_rank_ten_has_reciprocal_credit_but_rank_eleven_does_not(self):
        # Repeated irrelevant entries are integrity failures but retain original ranks.
        result = self.score(["r1"], [self.records[0]] * 9 + [self.records[1]])
        self.assertEqual(result["recall_at_5"], 0)
        self.assertEqual(result["reciprocal_rank_at_10"], 0.1)
        result = self.score(["r1"], [self.records[0]] * 10 + [self.records[1]])
        self.assertEqual(result["reciprocal_rank_at_10"], 0)
        self.assertFalse(result["integrity_ok"])

    def test_failed_empty_query_not_counted_as_success(self):
        query = {
            "id": "empty",
            "split": "held_out",
            "intent": "empty",
            "repository": "Atlas",
            "text": "missing",
            "expected_keys": [],
            "empty": True,
        }
        cli = mock.Mock()
        cli.run.side_effect = evaluation.EvaluationError("timeout")
        result = evaluation.evaluate_quality(
            cli, {"corpus": {"queries": [query]}, "manifest": self.manifest}, Path(".")
        )
        self.assertEqual(result["splits"]["held_out"]["empty_queries"], 1)
        self.assertEqual(result["splits"]["held_out"]["empty_queries_without_hits"], 0)
        self.assertFalse(result["integrity_ok"])

    def test_malformed_source_object_preserves_diagnostic(self):
        for source in (None, 7, [], "invalid"):
            record = copy.deepcopy(self.records[0])
            record["source"] = source
            result = self.score(["r0"], [record])
            self.assertFalse(result["integrity_ok"])
            self.assertEqual(result["recall_at_5"], 0)


if __name__ == "__main__":
    unittest.main()
