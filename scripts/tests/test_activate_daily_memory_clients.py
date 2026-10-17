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
        # The script resolves --home; match it so recorded paths compare equal.
        self.home = Path(self.tmp.name).resolve()
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

    def apply_migration(self):
        manifest = self.manifest_file()
        self.assertEqual(migrate.main(["--home", str(self.home), "--apply",
                                       "--expected-manifest", str(manifest)]), 0)
        return next((self.home / ".cortex/migrations").glob("clients-*.json"))

    def client_files(self):
        return {p: p.read_bytes() for p in self.home.rglob("*")
                if p.is_file() and ".cortex-migrate-" not in p.name and ".cortex" not in p.parts}

    def test_should_restore_codex_hooks_bytes_when_rolling_back(self):
        hooks = self.home / ".codex/hooks.json"
        plugin = {"command": 'sh "${PLUGIN_ROOT}/scripts/cortex-hook-launcher" Stop'}
        other = {"command": "other-tool stop"}
        legacy = {"command": '"E:/x/cortex-hook.exe" Stop --fire-forget'}
        # Compact, non-default formatting: rollback must restore bytes, not re-serialize.
        original = json.dumps({"hooks": {"Stop": [{"hooks": [legacy, plugin, other]}]}},
                              separators=(",", ":")).encode("utf-8")
        hooks.write_bytes(original)
        record_path = self.apply_migration()

        self.assertEqual(json.loads(hooks.read_bytes()),
                         {"hooks": {"Stop": [{"hooks": [plugin, other]}]}})
        record = json.loads(record_path.read_text(encoding="utf-8"))
        row = next(r for r in record["files"] if Path(r["path"]) == hooks)
        self.assertEqual(row["before_sha256"], migrate.digest(original))
        self.assertEqual(Path(row["backup"]).read_bytes(), original)

        self.assertEqual(migrate.main(["--rollback", str(record_path)]), 0)
        self.assertEqual(hooks.read_bytes(), original)

    def test_should_leave_kimi_entry_untouched_when_applying_and_rolling_back(self):
        kimi = self.home / ".kimi-code/mcp.json"
        write_json(kimi, {"mcpServers": {"cortex": {
            "command": "E:/x/cortex-mcp-server.exe", "args": ["serve"]}}})
        raw = kimi.read_bytes()
        _, manifest = migrate.plan(self.home)
        self.assertNotIn(str(kimi), [row["path"] for row in manifest["files"]])

        record_path = self.apply_migration()
        self.assertEqual(kimi.read_bytes(), raw)
        self.assertEqual(list(kimi.parent.iterdir()), [kimi], "no Kimi backup may be written")
        record = json.loads(record_path.read_text(encoding="utf-8"))
        self.assertNotIn(str(kimi), [row["path"] for row in record["files"]])

        migrate.rollback(record_path)
        self.assertEqual(kimi.read_bytes(), raw)

    def test_should_refuse_rollback_without_writing_when_backup_is_missing_or_corrupt(self):
        for damage in ("missing", "corrupt"):
            with self.subTest(damage=damage):
                self.tearDown()
                self.setUp()
                record_path = self.apply_migration()
                migrated = self.client_files()
                rows = json.loads(record_path.read_text(encoding="utf-8"))["files"]
                # Damage the LAST backup so a partial restore of earlier rows would show.
                backup = Path(rows[-1]["backup"])
                if damage == "missing":
                    backup.unlink()
                    expected = (OSError,)
                else:
                    backup.write_bytes(backup.read_bytes() + b"tampered")
                    expected = (ValueError,)
                with self.assertRaises(expected) as caught:
                    migrate.main(["--rollback", str(record_path)])
                if damage == "corrupt":
                    self.assertIn("no longer matches its recorded hash", str(caught.exception))
                self.assertEqual(self.client_files(), migrated)

    def test_should_refuse_rollback_without_writing_when_record_hash_mismatches_backup(self):
        record_path = self.apply_migration()
        migrated = self.client_files()
        record = json.loads(record_path.read_text(encoding="utf-8"))
        record["files"][-1]["before_sha256"] = "0" * 64
        record_path.write_text(json.dumps(record), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "no longer matches its recorded hash"):
            migrate.rollback(record_path, force=True)
        self.assertEqual(self.client_files(), migrated)

    def test_should_remove_stale_marketplace_on_apply_and_restore_it_on_rollback(self):
        settings = self.home / ".claude/settings.json"
        raw = settings.read_bytes()
        record_path = self.apply_migration()
        self.assertNotIn(migrate.MARKETPLACE,
                         json.loads(settings.read_text()).get("extraKnownMarketplaces", {}))
        migrate.rollback(record_path)
        self.assertEqual(settings.read_bytes(), raw)
        restored = json.loads(settings.read_text())["extraKnownMarketplaces"][migrate.MARKETPLACE]
        self.assertEqual(restored["source"]["path"], str(self.home / "gone"))

    def test_should_keep_marketplace_when_its_directory_still_exists(self):
        live = self.home / "marketplace"
        live.mkdir()
        settings = self.home / ".claude/settings.json"
        data = json.loads(settings.read_text())
        data["extraKnownMarketplaces"][migrate.MARKETPLACE]["source"]["path"] = str(live)
        write_json(settings, data)
        changes, _ = migrate.plan(self.home)
        after = json.loads(next(c for c in changes if c["path"] == settings)["after"])
        self.assertEqual(after["extraKnownMarketplaces"][migrate.MARKETPLACE]["source"]["path"],
                         str(live))


if __name__ == "__main__":
    unittest.main()
