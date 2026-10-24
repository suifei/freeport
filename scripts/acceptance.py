"""Strict native acceptance. Uses only newly created fixtures and retains all evidence.

Run on an elevated test machine or disposable VM with a debug fp.exe. Never use real caches.
--second-volume points at an EXISTING mounted test NTFS volume; this script never formats volumes.
Process-crash checks do not simulate VM power loss.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

p = argparse.ArgumentParser()
p.add_argument("--cli", required=True)
p.add_argument("--second-volume", help="Existing local test NTFS directory on another volume")
p.add_argument("--out", default=".build/native-acceptance.json")
p.add_argument("--test",choices=["all","handles"],default="all")
args = p.parse_args()
CLI = str(Path(args.cli).resolve())
BASE = Path(tempfile.mkdtemp(prefix="native-", dir=Path(".build").resolve()))
ENV = dict(os.environ, LOCALAPPDATA=str(BASE / "local"), USERPROFILE=str(BASE / "home"))
PHASES = ["prepared", "copying", "verified", "target_commit_intent", "target_ready",
          "source_rename_intent", "source_held", "link_intent", "linked", "completed"]
RESTORE_PHASES = ["prepared", "copying", "verified", "source_rename_intent", "source_held",
                  "materialize_intent", "materialized", "restored"]


def run(*values, phase=None, success=True):
    env = ENV.copy()
    if phase:
        env["FP_TEST_FAIL_AT"] = phase
    else:
        env.pop("FP_TEST_FAIL_AT", None)
    result = subprocess.run([CLI, "--lang", "en", *map(str, values)], env=env,
                            capture_output=True, timeout=60, creationflags=subprocess.CREATE_NO_WINDOW)
    if success and result.returncode:
        raise AssertionError(result.stderr.decode("utf-8", errors="replace"))
    return result


def fixture(name):
    root = BASE / name
    root.mkdir()
    (root / "freeport-fixture.marker").write_bytes(b"FreePort test fixture\n")
    return root


def populate(source):
    source.mkdir()
    (source / "empty").mkdir()
    (source / "日本語-한국어.txt").write_bytes(b"verified data" * 1000)
    Path(str(source / "日本語-한국어.txt") + ":metadata").write_bytes(b"alternate stream")


def verify(source):
    assert (source / "日本語-한국어.txt").read_bytes() == b"verified data" * 1000
    assert Path(str(source / "日本語-한국어.txt") + ":metadata").read_bytes() == b"alternate stream"
    assert (source / "empty").is_dir()


class NativeAcceptance(unittest.TestCase):
    def test_migration_crash_at_every_durable_phase(self):
        for phase in PHASES:
            with self.subTest(phase=phase):
                root = fixture("migrate-" + phase)
                source, target, state = root / "source", root / "target", root / "state"
                populate(source)
                result = run("migrate", "--source", source, "--target", target, "--state", state,
                             "--yes", phase=phase, success=False)
                self.assertNotEqual(result.returncode, 0, "Use a debug build with working fault injection")
                records = json.loads(run("history", "--state", state).stdout)
                self.assertEqual(len(records), 1, "Preflight failed before the injection point")
                self.assertEqual(records[0]["phase"], phase)
                record = json.loads(run("recover", "--transaction", records[0]["id"], "--state", state, "--yes").stdout)
                self.assertIn(record["phase"], ("completed", "rolled_back"))
                verify(source)
                # Replaying recovery is idempotent.
                replay = json.loads(run("recover", "--transaction", record["id"], "--state", state, "--yes").stdout)
                self.assertEqual(record["phase"], replay["phase"])

    def test_restore_handmade_shared_target_and_current_data(self):
        root = fixture("shared-target")
        source, second, target, state = root / "source", root / "second", root / "target", root / "state"
        populate(target)
        for link in [source, second]:
            run("__junction-test", "--source", link, "--target", target)
        attempted=run("migrate","--source",source,"--target",root/"must-not-create","--state",state,"--yes",success=False)
        self.assertNotEqual(attempted.returncode,0)
        self.assertFalse((root/"must-not-create").exists())
        (target / "new-after-link.txt").write_bytes(b"current")
        report = json.loads(run("scan", "--path", root, "--mode", "restore", "--state", state).stdout)
        found = {r["candidate"]["source"].lower() for r in report["rows"]}
        self.assertIn(str(source).lower(), found)
        run("restore", "--source", source, "--state", state, "--yes")
        verify(source)
        verify(target)
        self.assertEqual((source / "new-after-link.txt").read_bytes(), b"current")
        self.assertEqual((second / "new-after-link.txt").read_bytes(), b"current")
        (source / "independent.txt").write_bytes(b"restored")
        self.assertFalse((target / "independent.txt").exists())

    def test_restore_crash_at_every_durable_phase(self):
        for phase in RESTORE_PHASES:
            with self.subTest(phase=phase):
                root = fixture("restore-" + phase)
                source, target, state = root / "source", root / "target", root / "state"
                populate(target)
                run("__junction-test", "--source", source, "--target", target)
                result = run("restore", "--source", source, "--state", state, "--yes", phase=phase, success=False)
                self.assertNotEqual(result.returncode, 0)
                records = json.loads(run("history", "--state", state).stdout)
                self.assertEqual(records[0]["phase"], phase)
                run("recover", "--transaction", records[0]["id"], "--state", state, "--yes")
                verify(source)
                verify(target)

    def test_force_closes_only_own_test_holder_selected_handles(self):
        root = fixture("handles")
        selected = root / "selected"
        selected.mkdir()
        occupied, unrelated = selected / "occupied", root / "unrelated"
        for f in (occupied, unrelated):
            f.write_bytes(b"fixture")
        holder = subprocess.Popen([CLI, "--lang", "en", "__lock-test", "--path", str(occupied),
                                   "--target", str(unrelated)], env=ENV, stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            self.assertEqual(holder.stdout.readline().strip(), b"locked")
            with self.assertRaises(PermissionError):
                occupied.write_bytes(b"blocked")
            count = int(run("__force-test", "--path", selected, "--yes", "--force-close-handles").stdout)
            self.assertGreaterEqual(count, 1)
            self.assertIsNone(holder.poll(), "The holder must remain alive; no process killing")
            occupied.write_bytes(b"released")
            with self.assertRaises(PermissionError):
                unrelated.write_bytes(b"must remain locked")
        finally:
            if holder.poll() is None:
                holder.communicate(b"exit\n", timeout=10)

    def test_cross_volume_migrate_and_restore(self):
        if not args.second_volume:
            self.fail("Cross-volume acceptance requires --second-volume; not verified")
        second = Path(args.second_volume).resolve()
        self.assertNotEqual(second.drive.lower(), BASE.drive.lower())
        target_root = Path(tempfile.mkdtemp(prefix="freeport-test-", dir=second))
        root = fixture("cross-volume")
        source, target, state = root / "source", target_root / "target", root / "state"
        populate(source)
        run("migrate", "--source", source, "--target", target, "--state", state, "--yes")
        verify(source)
        verify(target)
        run("restore", "--source", source, "--state", state, "--yes")
        verify(source)
        verify(target)


if __name__ == "__main__":
    suite=unittest.TestSuite([NativeAcceptance("test_force_closes_only_own_test_holder_selected_handles")]) if args.test=="handles" else unittest.defaultTestLoader.loadTestsFromTestCase(NativeAcceptance)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    Path(args.out).write_text(json.dumps({"tests": result.testsRun, "failures": len(result.failures),
        "errors": len(result.errors), "passed": result.wasSuccessful(), "power_loss_tested": False,
        "windows7_runtime_tested": False}, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if result.wasSuccessful() else 1)
