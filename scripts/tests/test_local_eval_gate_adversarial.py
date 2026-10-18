"""Independent regressions for missing evidence and changed benchmark work."""

import unittest

from scripts import gate_local_memory as gate
from scripts.tests import test_local_eval_gate as fixtures


class EvidenceCoverageTests(unittest.TestCase):
    def setUp(self):
        reference = fixtures.GateTests()
        reference.setUp()
        self.quality = reference.q
        self.latency = reference.l
        self.policy = reference.policy
        self.assertTrue(self.assess()["passed"])

    def assess(self):
        return gate.assess(self.quality, self.latency, self.policy)

    def test_empty_matching_source_snapshots_fail(self):
        for home in self.quality["homes"]:
            home["source_hashes"] = {}
        self.assertFalse(self.assess()["passed"])

    def test_changed_archive_hash_in_both_homes_fails(self):
        for home in self.quality["homes"]:
            home["source_hashes"]["archive/events.ndjson.zst"] = "f" * 64
        self.assertFalse(self.assess()["passed"])

    def test_source_path_normalization_collision_fails(self):
        for home in self.quality["homes"]:
            home["source_hashes"]["archive\\events.ndjson.zst"] = "f" * 64
        self.assertFalse(self.assess()["passed"])

    def test_wrong_latency_scope_fails(self):
        self.latency["latency"]["repository"] = "Foreign"
        self.assertFalse(self.assess()["passed"])

    def test_empty_latency_query_fails(self):
        self.latency["latency"]["query"] = ""
        self.assertFalse(self.assess()["passed"])

    def test_latency_top_level_error_overrides_passed(self):
        self.latency["error"] = "Setup failed"
        self.assertFalse(self.assess()["passed"])

    def test_latency_section_error_overrides_passed(self):
        self.latency["latency"]["error"] = "Measurement failed"
        self.assertFalse(self.assess()["passed"])

    def test_quality_error_overrides_integrity_passed(self):
        self.quality["error"] = "Evaluation failed"
        self.assertFalse(self.assess()["passed"])


if __name__ == "__main__":
    unittest.main()
