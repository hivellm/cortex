"""Publication safety tests independent of the optional zstandard dependency."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock


MODULE = Path(__file__).resolve().parents[1] / "generate_local_eval_fixtures.py"
spec = importlib.util.spec_from_file_location("generate_local_eval_fixtures", MODULE)
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class PublicationTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="cortex-generator-test-")
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.corpus = self.root / "corpus.json"
        self.corpus.write_bytes(b"reviewed corpus")
        self.output = self.root / "output"
        self.output.mkdir()
        self.files = {"daily.ndjson": b"new daily", "manifest.json": b"new manifest"}

    def generate(self):
        argv = [
            str(MODULE),
            "--corpus",
            str(self.corpus),
            "--output",
            str(self.output),
        ]
        with (
            mock.patch("sys.argv", argv),
            mock.patch.object(generator, "materialize", return_value=self.files),
        ):
            generator.main()

    def test_existing_destination_preserves_all_files(self):
        existing = self.output / "manifest.json"
        existing.write_bytes(b"reviewed manifest")
        unrelated = self.output / "notes.txt"
        unrelated.write_bytes(b"keep notes")
        with self.assertRaisesRegex(SystemExit, "Output already exists"):
            self.generate()
        self.assertEqual(existing.read_bytes(), b"reviewed manifest")
        self.assertEqual(unrelated.read_bytes(), b"keep notes")
        self.assertFalse((self.output / "daily.ndjson").exists())

    def test_destination_created_after_precheck_is_not_overwritten(self):
        destination = self.output / "daily.ndjson"
        original_open = Path.open
        raced = []

        def race_open(path, mode="r", *args, **kwargs):
            if path == destination and not raced:
                raced.append(True)
                with original_open(path, "xb") as other_writer:
                    other_writer.write(b"concurrent author")
            return original_open(path, mode, *args, **kwargs)

        with mock.patch.object(Path, "open", race_open):
            with self.assertRaises(FileExistsError):
                self.generate()
        self.assertEqual(raced, [True])
        self.assertEqual(destination.read_bytes(), b"concurrent author")
        self.assertFalse((self.output / "manifest.json").exists())

    def test_dangling_symlink_is_preserved_without_creating_target(self):
        destination = self.output / "manifest.json"
        missing_target = self.root / "missing-target"
        try:
            destination.symlink_to(missing_target)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"Symlinks unavailable: {error}")
        with self.assertRaisesRegex(SystemExit, "Output already exists"):
            self.generate()
        self.assertTrue(destination.is_symlink())
        self.assertEqual(destination.readlink(), missing_target)
        self.assertFalse(missing_target.exists())
        self.assertFalse((self.output / "daily.ndjson").exists())


if __name__ == "__main__":
    unittest.main()
