"""Independent ranking attacks using real source validation and fixed judgments."""

import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest


MODULE = Path(__file__).resolve().parents[1] / "evaluate_local_memory.py"
spec = importlib.util.spec_from_file_location("scripts.local_eval_adversarial", MODULE)
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)


class AdversarialScoringTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="cortex-score-adversarial-")
        self.addCleanup(scratch.cleanup)
        self.fixtures = evaluation.load_fixtures(evaluation.DEFAULT_FIXTURES)
        self.manifest = self.fixtures["manifest"]
        self.sources = Path(scratch.name) / "sources"
        evaluation.materialize_sources(self.fixtures, self.sources)
        self.queries = {q["id"]: q for q in self.fixtures["corpus"]["queries"]}

    @staticmethod
    def seal(record):
        record["content_sha256"] = ""
        record["content_sha256"] = evaluation.digest(evaluation.canonical(record))

    def hit(self, key):
        expected = self.manifest["records"][key]
        record = {
            field: copy.deepcopy(expected[field])
            for field in (
                "id",
                "repository",
                "revision",
                "kind",
                "authority",
                "title",
                "links",
                "source",
            )
        }
        record.update(
            schema_version=1,
            created_at="2026-02-03T09:00:00Z",
            updated_at="2026-02-03T09:00:00Z",
            metadata={},
        )
        record["body"] = expected.get("body", "")
        if "source_file" in expected:
            record["source"]["locator"] = str(
                (self.sources / expected["source_file"]).resolve()
            )
        if "source_line" in expected:
            record["source"]["locator"] += "#line=" + str(expected["source_line"])
            record["body"] = expected["body"]
        record["metadata"] = copy.deepcopy(expected["metadata"])
        self.seal(record)
        evaluation.validate_record(record, expected, self.sources)
        return {"record": record, "score": 1.0}

    def score(self, query_id, hits):
        return evaluation.score_query(
            self.queries[query_id], hits, self.manifest, self.sources
        )

    def test_rank_five_included_six_excluded_without_compressing_duplicates(self):
        irrelevant = self.hit("atlas-auth")
        for rank, recall in ((5, 1.0), (6, 0.0)):
            with self.subTest(rank=rank):
                result = self.score(
                    "d01", [irrelevant] * (rank - 1) + [self.hit("atlas-storage")]
                )
                self.assertEqual(result["recall_at_5"], recall)
                self.assertEqual(result["reciprocal_rank_at_10"], 1 / rank)
                self.assertEqual(result["retrieved"][-1]["rank"], rank)
                self.assertFalse(result["integrity_ok"])

    def test_rank_ten_included_eleven_excluded(self):
        irrelevant = self.hit("atlas-auth")
        for rank, reciprocal in ((10, 0.1), (11, 0.0)):
            with self.subTest(rank=rank):
                result = self.score(
                    "d01", [irrelevant] * (rank - 1) + [self.hit("atlas-storage")]
                )
                self.assertEqual(result["recall_at_5"], 0.0)
                self.assertEqual(result["reciprocal_rank_at_10"], reciprocal)
                self.assertFalse(result["integrity_ok"])

    def test_duplicate_relevant_hits_cannot_inflate_recall(self):
        relevant = self.hit("atlas-timeout-fix")
        result = self.score("d04", [relevant] * 5)
        self.assertEqual(result["recall_at_5"], 0.5)
        self.assertEqual(result["reciprocal_rank_at_10"], 1.0)
        self.assertFalse(result["integrity_ok"])
        self.assertEqual(sum(hit["valid"] for hit in result["retrieved"]), 1)

    def test_expected_id_with_wrong_source_earns_no_credit_even_if_resealed(self):
        for field, value in (("source_hash", "0" * 64), ("source_id", "wrong-source")):
            with self.subTest(field=field):
                forged = self.hit("atlas-storage")
                forged["record"]["source"][field] = value
                self.seal(forged["record"])
                result = self.score("d01", [forged])
                self.assertEqual(result["recall_at_5"], 0.0)
                self.assertEqual(result["reciprocal_rank_at_10"], 0.0)
                self.assertFalse(result["integrity_ok"])
                self.assertIn("source " + field + " mismatch", result["diagnostics"][0])

    def test_explicit_proposal_lookup_accepts_valid_proposed_record(self):
        hit = self.hit("atlas-search-proposal")
        self.assertEqual(hit["record"]["authority"], "proposed")
        result = self.score("d08", [hit])
        self.assertTrue(result["integrity_ok"], result["diagnostics"])
        self.assertEqual(result["recall_at_5"], 1.0)
        self.assertEqual(result["reciprocal_rank_at_10"], 1.0)

    def test_three_empty_judgments_excluded_from_both_macro_denominators(self):
        owner = self

        class FixtureCli:
            def run(self, *args):
                repository, text = args[1], args[3]
                query = next(
                    q
                    for q in owner.queries.values()
                    if q["repository"] == repository and q["text"] == text
                )
                # Missing one answerable query distinguishes denominator choices.
                if query["empty"] or query["id"] == "d01":
                    return []
                return [owner.hit(key) for key in query["expected_keys"]]

        result = evaluation.evaluate_quality(FixtureCli(), self.fixtures, self.sources)
        self.assertTrue(result["integrity_ok"])
        empty = [q for q in result["queries"] if q["empty"]]
        self.assertEqual(len(empty), 3)
        self.assertTrue(all(q["recall_at_5"] is None for q in empty))
        self.assertTrue(all(q["reciprocal_rank_at_10"] is None for q in empty))
        development = result["splits"]["development"]
        held_out = result["splits"]["held_out"]
        self.assertEqual(development["answerable_queries"], 10)
        self.assertEqual(development["empty_queries"], 2)
        # d05 has eight relevant records, only five within Recall@5.
        self.assertEqual(development["recall_at_5"], (8 + 5 / 8) / 10)
        self.assertEqual(development["mrr_at_10"], 9 / 10)
        self.assertEqual(held_out["answerable_queries"], 7)
        self.assertEqual(held_out["empty_queries"], 1)
        self.assertEqual(held_out["recall_at_5"], 1.0)
        self.assertEqual(held_out["mrr_at_10"], 1.0)


if __name__ == "__main__":
    unittest.main()
