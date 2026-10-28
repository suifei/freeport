"""Read-only/product-interface checks using fresh disposable fixtures; no external packages."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

parser = argparse.ArgumentParser()
parser.add_argument("--cli", required=True)
parser.add_argument("--gui", required=True)
parser.add_argument("--out", default=".build/smoke-result.json")
args = parser.parse_args()
CLI, GUI = str(Path(args.cli).resolve()), str(Path(args.gui).resolve())
TAGS = ["en", "zh-CN", "zh-TW", "ja", "ko", "de"]
Path(".build").mkdir(exist_ok=True)
ROOT = Path(tempfile.mkdtemp(prefix="interface-", dir=Path(".build").resolve()))
ENV = dict(os.environ, LOCALAPPDATA=str(ROOT / "local"), USERPROFILE=str(ROOT / "home"))
PREF = ROOT / "local" / "FreePort" / "language.json"
PREF.parent.mkdir(parents=True)
FIXTURE = ROOT / "home"
for rel in [".claude", ".cache/huggingface", ".vscode/extensions", "go/pkg/mod"]:
    folder = FIXTURE / rel
    folder.mkdir(parents=True)
    (folder / "日本語-한국어.txt").write_text("original fixture data", encoding="utf-8")
STATE = ROOT / "state"
for rel in [".claude/projects", ".cache/huggingface/hub", "go/pkg/mod/cache/download"]:
    (FIXTURE / rel).mkdir(parents=True, exist_ok=True)
    (FIXTURE / rel / "fixture.bin").write_bytes(b"owned application fixture")
(FIXTURE / ".claude/settings.json").write_text("{}", encoding="utf-8")
(FIXTURE / ".vscode/extensions/extensions.json").write_text("[]", encoding="utf-8")


def invoke(*values, lang=None, code=0):
    command = [CLI] + (["--lang", lang] if lang else []) + list(values)
    p = subprocess.run(command, env=ENV, capture_output=True, timeout=30)
    stdout, stderr = p.stdout.decode("utf-8", errors="strict"), p.stderr.decode("utf-8", errors="strict")
    if p.returncode != code:
        raise AssertionError(f"{values}: exit {p.returncode} != {code}; {stderr}")
    return stdout, stderr


class InterfaceTests(unittest.TestCase):
    def tearDown(self):
        PREF.unlink(missing_ok=True)

    def test_six_help_and_errors(self):
        helps = []
        for tag in TAGS:
            text, _ = invoke("--help", lang=tag)
            self.assertIn("fp scan --volumes", text)
            self.assertIn("--lang", text)
            helps.append(text)
            _, error = invoke("not-a-command", lang=tag, code=1)
            self.assertIn("not-a-command", error)
            self.assertNotIn("Message unavailable", error)
        self.assertEqual(len(set(helps)), 6)

    def test_invalid_language_is_english_before_operations(self):
        PREF.write_text('{"language":"de"}', encoding="utf-8")
        _, error = invoke("scan", "--volumes", "C:", lang="invalid", code=1)
        self.assertIn("Invalid language", error)
        self.assertNotIn("Scanning", error)

    def test_shared_preference_and_explicit_override(self):
        PREF.write_text('{"language":"de"}', encoding="utf-8")
        automatic, _ = invoke("--help")
        explicit, _ = invoke("--help", lang="de")
        english, _ = invoke("--help", lang="en")
        self.assertEqual(automatic, explicit)
        self.assertNotEqual(automatic, english)
        before = PREF.read_bytes()
        invoke("--help", lang="auto")
        self.assertEqual(PREF.read_bytes(), before)

    def test_corrupt_preference_is_english_and_warns(self):
        PREF.write_bytes(b'{"language":"unknown"}')
        text, error = invoke("--help")
        english, _ = invoke("--help", lang="en")
        self.assertEqual(text, english)
        self.assertIn("saved language preference is invalid", error)

    def test_unknown_duplicate_and_missing_options(self):
        for options in [("--unknown",), ("--path",), ("--measure", "--measure")]:
            invoke("scan", *options, lang="ja", code=1)

    def test_readonly_scan_has_stable_machine_fields(self):
        def content():
            return {str(p.relative_to(FIXTURE)): hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in FIXTURE.rglob("*") if p.is_file()}
        before = content()
        baseline = None
        for tag in TAGS:
            text, _ = invoke("scan", "--path", str(FIXTURE), "--measure", "--state", str(STATE), lang=tag)
            report = json.loads(text)
            self.assertEqual(report["mode"], "migrate")
            self.assertEqual([os.path.normcase(p) for p in report["scope_roots"]], [os.path.normcase(str(FIXTURE))])
            rows = [{k: r[k] for k in ("stats", "status", "link_target", "busy")} |
                    {k: r["candidate"][k] for k in ("id", "source", "category", "recommended")}
                    for r in report["rows"]]
            self.assertGreaterEqual(len(rows), 4)
            if baseline is None:
                baseline = rows
            self.assertEqual(rows, baseline)
            self.assertEqual(content(), before)
        self.assertFalse(STATE.exists(), "Scanning must not create a state directory")

    def test_rules_upgrade_is_data_only_and_scan_uses_selected_state(self):
        state = ROOT / "rules-state"
        pack = json.loads(invoke("rules", "--state", str(state), lang="en")[0])
        pack["revision"] += 1
        rule = next(r for r in pack["rules"] if r["id"] == "claude-projects")
        rule.update(id="fixture-rule", name="fixture")
        pack["rules"] = [rule]
        path = ROOT / "pack.json"
        path.write_text(json.dumps(pack), encoding="utf-8")
        invoke("rules", "--import", str(path), "--state", str(state), lang="en")
        original = (state / "rules.json").read_bytes()
        invoke("rules", "--import", str(path), "--state", str(state), lang="en", code=1)
        pack["revision"] += 1
        pack["execute"] = "must not execute"
        path.write_text(json.dumps(pack), encoding="utf-8")
        invoke("rules", "--import", str(path), "--state", str(state), lang="en", code=1)
        self.assertEqual((state / "rules.json").read_bytes(), original)
        report = json.loads(invoke("scan", "--path", str(FIXTURE), "--state", str(state), lang="en")[0])
        self.assertTrue(any(r["candidate"]["name"] == "fixture" for r in report["rows"]))
        self.assertEqual(json.loads(invoke("history", "--state", str(state), lang="en")[0]), [])

    def test_native_gui_defaults_and_six_language_switching(self):
        p = subprocess.run([GUI, "--lang", "ja", "--self-test"], env=ENV, capture_output=True,
                           timeout=45, creationflags=subprocess.CREATE_NO_WINDOW)
        self.assertEqual(p.returncode, 0, p.stderr.decode("utf-8", errors="replace"))
        report = json.loads(p.stdout.decode("utf-8", errors="strict"))
        self.assertTrue(report["window_is_native"])
        self.assertTrue(report["tree_checks"])
        self.assertFalse(report["administrator_option"])
        self.assertEqual(report["language"], "ja")
        self.assertFalse(report["request_app_exit"])
        self.assertFalse(report["force_close_handles"])
        self.assertEqual(report["selected_count"], 0)
        self.assertTrue(report["language_switch_preserves_state"])
        self.assertTrue(report["dialogs_localized"])
        self.assertEqual(set(report["labels"]), set(TAGS))
        self.assertEqual(len(set(report["labels"].values())), 6)

    def test_legacy_pack_preserves_file_and_uses_bundled_application_evidence(self):
        state = ROOT / "legacy-state"
        state.mkdir()
        legacy = {"schema_version": 1, "revision": 1, "rules": [{"id":"legacy", "name":"legacy", "category":"cache", "suffix":".claude"}]}
        path = state / "rules.json"
        original = json.dumps(legacy).encode("utf-8")
        path.write_bytes(original)
        report = json.loads(invoke("scan", "--path", str(FIXTURE), "--state", str(state), lang="en")[0])
        self.assertTrue(any(r["candidate"].get("knowledge", {}).get("application") == "Claude Code" for r in report["rows"]))
        self.assertTrue(any("legacy" in w for w in report["warnings"]))
        self.assertFalse(any(r["candidate"]["name"] == "legacy" for r in report["rows"]))
        self.assertEqual(path.read_bytes(), original)
        path.write_bytes(b"not valid rules JSON")
        restored = json.loads(invoke("scan", "--path", str(FIXTURE), "--mode", "restore", "--state", str(state), lang="en")[0])
        self.assertEqual(restored["mode"], "restore")
        self.assertEqual(path.read_bytes(), b"not valid rules JSON")

    def test_selection_file_rejections_are_readonly(self):
        report = json.loads(invoke("scan", "--path", str(FIXTURE), "--state", str(STATE), lang="en")[0])
        scan = ROOT / "selection-scan.json"
        scan.write_text(json.dumps(report), encoding="utf-8")
        selection = ROOT / "selection.json"
        for value in [
            {"version": 99, "include": [str(FIXTURE)]},
            {"version": 1, "include": []},
            {"version": 1, "include": [str(ROOT)]},
            {"version": 1, "include": [str(FIXTURE)], "execute": "must never execute"},
        ]:
            selection.write_text(json.dumps(value), encoding="utf-8")
            invoke("plan", "--scan", str(scan), "--selection", str(selection),
                   "--destination", str(ROOT / "destination"), "--state", str(STATE), lang="en", code=1)
        selection.write_text(" " * (1024 * 1024 + 1), encoding="utf-8")
        invoke("plan", "--scan", str(scan), "--selection", str(selection),
               "--destination", str(ROOT / "destination"), "--state", str(STATE), lang="en", code=1)
        self.assertFalse((ROOT / "destination").exists())
        self.assertFalse(STATE.exists())

    def test_explicit_elevation_does_not_change_defaults_or_parameter_validation(self):
        version, _ = invoke("--version", "--elevate", lang="en")
        self.assertIn("FreePort", version)
        invoke("scan", "--elevate", "--elevate", lang="en", code=1)
        invoke("plan", "--select", "x", "--selection", "missing.json", lang="en", code=1)


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(InterfaceTests))
    Path(args.out).write_text(json.dumps({"tests": result.testsRun, "failures": len(result.failures),
        "errors": len(result.errors), "passed": result.wasSuccessful(),
        "binaries": {Path(p).name:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in [CLI, GUI]},
        "note": "Interface tests only; does not prove migration, force-close, or power-loss safety."}, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if result.wasSuccessful() else 1)
