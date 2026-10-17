"""Migration and rollback of legacy Cortex client entries against a synthetic home."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "activate_daily_memory_clients.py"
spec = importlib.util.spec_from_file_location("migrate", SCRIPT)
migrate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migrate)

CODEX_TOML = """model = "gpt"

[mcp_servers.other]
command = "other"

[mcp_servers.cortex]
command = "E:/x/cortex-mcp-server.exe"
args = ["serve"]

[mcp_servers.cortex.env]
CORTEX_API_URL = "http://127.0.0.1:17000"

[projects.'e:\\\\repo']
trust_level = "trusted"
"""


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2), encoding="utf-8")


class MigrationTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = Path(self.tmp.name)
        legacy = {"type": "command", "command": '"E:/x/cortex-hook.exe" Stop --pipe "//./pipe/p" --fire-forget'}
        other = {"type": "command", "command": "other-tool stop"}
        write_json(self.home / ".claude/settings.json", {
            "theme": "dark",
            "hooks": {"Stop": [{"hooks": [legacy, other]}],
                      "SessionStart": [{"hooks": [{"command": "cortex-hook SessionStart"}]}]},
            "enabledPlugins": {migrate.PLUGIN_ID: True},
            "extraKnownMarketplaces": {migrate.MARKETPLACE: {"source": {
                "source": "directory", "path": str(self.home / "gone")}}}})
        write_json(self.home / ".claude/plugins/installed_plugins.json",
                   {"version": 2, "plugins": {migrate.PLUGIN_ID: [{"scope": "user"}]}})
        write_json(self.home / ".claude.json", {"numStartups": 3, "mcpServers": {
            "cortex": {"command": "E:/x/cortex-mcp-server.exe", "args": ["serve"]},
            "unreal": {"command": "unreal"}}})
        write_json(self.home / ".codex/hooks.json", {"hooks": {"Stop": [{"hooks": [
            {"command": "cortex-hook Stop --fire-forget"}]}]}})
        (self.home / ".codex/config.toml").write_text(CODEX_TOML, encoding="utf-8")
        self.native = self.home / ".claude/projects/memory/note.md"
        self.native.parent.mkdir(parents=True)
        self.native.write_text("native memory", encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()

    def manifest_file(self):
        _, manifest = migrate.plan(self.home)
        path = self.home / "manifest.json"
        path.write_text(json.dumps(manifest), encoding="utf-8")
        return path

    def test_dry_run_removes_only_cortex_entries_and_writes_nothing(self):
        before = {p: p.read_bytes() for p in self.home.rglob("*") if p.is_file()}
        changes, manifest = migrate.plan(self.home)
        self.assertEqual(before, {p: p.read_bytes() for p in self.home.rglob("*") if p.is_file()})
        by_name = {Path(c["path"]).name: c for c in changes}
        settings = json.loads(by_name["settings.json"]["after"])
        self.assertEqual(settings["hooks"], {"Stop": [{"hooks": [{"type": "command", "command": "other-tool stop"}]}]})
        self.assertNotIn("extraKnownMarketplaces", settings)
        self.assertEqual(settings["theme"], "dark")
        claude = json.loads(by_name[".claude.json"]["after"])
        self.assertEqual(claude, {"numStartups": 3, "mcpServers": {"unreal": {"command": "unreal"}}})
        self.assertEqual(json.loads(by_name["hooks.json"]["after"]), {})
        toml = by_name["config.toml"]["after"].decode()
        self.assertNotIn("cortex", toml)
        self.assertIn("[mcp_servers.other]", toml)
        self.assertIn("trust_level", toml)
        self.assertTrue(all(row["removed"] for row in manifest["files"]))

    def test_apply_then_rollback_restores_bytes(self):
        originals = {p: p.read_bytes() for p in self.home.rglob("*") if p.is_file()}
        manifest = self.manifest_file()
        self.assertEqual(migrate.main(["--home", str(self.home), "--apply",
                                       "--expected-manifest", str(manifest)]), 0)
        records = list((self.home / ".cortex/migrations").glob("clients-*.json"))
        self.assertEqual(len(records), 1)
        self.assertNotIn("cortex-hook", (self.home / ".claude/settings.json").read_text())
        _, again = migrate.plan(self.home)
        self.assertTrue(all(row["before_sha256"] == row["after_sha256"] for row in again["files"]))
        self.assertEqual(self.native.read_text(encoding="utf-8"), "native memory")

        self.assertEqual(migrate.main(["--rollback", str(records[0])]), 0)
        for path, raw in originals.items():
            self.assertEqual(path.read_bytes(), raw, path)

    def test_apply_refuses_without_verified_plugin_or_matching_manifest(self):
        manifest = self.manifest_file()
        settings = self.home / ".claude/settings.json"
        data = json.loads(settings.read_text())
        data["enabledPlugins"][migrate.PLUGIN_ID] = False
        write_json(settings, data)
        with self.assertRaisesRegex(ValueError, "Expected state differs"):
            migrate.main(["--home", str(self.home), "--apply", "--expected-manifest", str(manifest)])
        manifest.unlink()
        manifest = self.manifest_file()
        with self.assertRaisesRegex(ValueError, "plugin"):
            migrate.main(["--home", str(self.home), "--apply", "--expected-manifest", str(manifest)])
        self.assertFalse((self.home / ".cortex/migrations").exists())

    def test_rollback_refuses_files_changed_after_migration(self):
        manifest = self.manifest_file()
        migrate.main(["--home", str(self.home), "--apply", "--expected-manifest", str(manifest)])
        record = next((self.home / ".cortex/migrations").glob("clients-*.json"))
        (self.home / ".claude.json").write_text("{}", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "changed since migration"):
            migrate.rollback(record)
        migrate.rollback(record, force=True)
        self.assertIn("cortex", json.loads((self.home / ".claude.json").read_text())["mcpServers"])


if __name__ == "__main__":
    unittest.main()
