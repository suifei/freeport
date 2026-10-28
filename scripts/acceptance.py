"""Strict native acceptance. Uses only newly created fixtures and retains all evidence.

Run on an elevated test machine or disposable VM with a debug fp.exe. Never use real caches.
--second-volume points at an EXISTING mounted test NTFS volume; this script never formats volumes.
Process-crash checks do not simulate VM power loss.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import time
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
          "source_rename_intent", "source_held", "source_cleanup_intent", "source_cleaned",
          "link_intent", "linked", "completed"]
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


def is_reparse(path):
    return bool(os.lstat(path).st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT)


def junction_target(path):
    if not os.path.lexists(path) or not is_reparse(path):
        return None
    return os.path.normcase(os.readlink(path).removeprefix("\\\\?\\"))


def contents(root):
    """Relative path -> bytes for every regular file under root, never following reparse points."""
    found = {}
    for top, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if not is_reparse(os.path.join(top, d))]
        for f in files:
            full = Path(top, f)
            found[str(full.relative_to(root))] = full.read_bytes()
    return found


def lock(path):
    holder = subprocess.Popen([CLI, "--lang", "en", "__lock-test", "--path", str(path)], env=ENV,
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              creationflags=subprocess.CREATE_NO_WINDOW)
    assert holder.stdout.readline().strip() == b"locked"
    return holder


def unlock(holder):
    if holder.poll() is None:
        holder.communicate(b"exit\n", timeout=10)


def write_plan(path, items):
    plan = {"version": 1, "items": [{"id": f"item{i}", "name": Path(s).name, "source": str(s),
                                     "target": str(t), "bytes": 0} for i, (s, t) in enumerate(items)]}
    path.write_text(json.dumps(plan), encoding="utf-8")
    return path


def history(state):
    return json.loads(run("history", "--state", state).stdout)


def powershell(command):
    subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", command], check=True,
                   capture_output=True, timeout=120, creationflags=subprocess.CREATE_NO_WINDOW)


class NativeAcceptance(unittest.TestCase):
    def second_volume(self):
        if not args.second_volume:
            self.fail("Requires --second-volume; not verified")
        second = Path(args.second_volume).resolve()
        self.assertNotEqual(second.drive.lower(), BASE.drive.lower())
        return Path(tempfile.mkdtemp(prefix="freeport-test-", dir=second))

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

    # P01: cross-volume migration keeps content and metadata; the app keeps using the original path.
    def test_p01_metadata_and_use_through_original_path(self):
        root, target = fixture("p01"), self.second_volume() / "target"
        source, state = root / "source", root / "state"
        populate(source)
        (source / "deep" / "deeper").mkdir(parents=True)
        readonly = source / "deep" / "deeper" / "readonly.bin"
        readonly.write_bytes(os.urandom(256 * 1024))
        stamp = 1_600_000_000
        os.utime(readonly, (stamp, stamp))
        os.chmod(readonly, stat.S_IREAD)
        before = contents(source)
        run("migrate", "--source", source, "--target", target, "--state", state, "--yes")
        self.assertEqual(junction_target(source), os.path.normcase(str(target)))
        self.assertEqual(contents(target), before)
        verify(target)
        moved = target / "deep" / "deeper" / "readonly.bin"
        self.assertEqual(int(moved.stat().st_mtime), stamp)
        self.assertFalse(moved.stat().st_mode & stat.S_IWRITE, "read-only attribute lost")
        (source / "written-via-link.txt").write_bytes(b"app write")
        (source / "日本語-한국어.txt").write_bytes(b"app update")
        self.assertEqual((target / "written-via-link.txt").read_bytes(), b"app write")
        self.assertEqual((target / "日本語-한국어.txt").read_bytes(), b"app update")
        self.assertEqual(history(state)[0]["phase"], "completed")

    # P02: a handmade cross-drive junction is found by scanning the whole volume (NTFS MFT path);
    # a same-drive junction frees nothing and is not listed.
    def test_p02_volume_restore_scan_finds_handmade_junction(self):
        holder = self.second_volume()
        (holder / "freeport-fixture.marker").write_bytes(b"FreePort test fixture\n")
        target, link, local = fixture("p02") / "real", holder / "link", holder / "local"
        populate(target)
        populate(holder / "real")
        run("__junction-test", "--source", link, "--target", target)
        run("__junction-test", "--source", local, "--target", holder / "real")
        drive = Path(args.second_volume).drive
        report = json.loads(run("scan", "--volumes", drive, "--mode", "restore",
                                "--state", BASE / "p02-state").stdout)
        self.assertTrue(any("MFT" in b for b in report["backends"]), report["backends"])
        found = {os.path.normcase(r["candidate"]["source"]) for r in report["rows"]}
        self.assertIn(os.path.normcase(str(link)), found)
        self.assertNotIn(os.path.normcase(str(local)), found)

    # P04: robocopy cannot read a locked file -> source untouched, no cleanup, recover rolls back.
    def test_p04_locked_file_keeps_source(self):
        root = fixture("p04")
        source, target, state = root / "source", root / "target", root / "state"
        populate(source)
        locked = source / "locked.db"
        locked.write_bytes(b"in use")
        before = contents(source)
        holder = lock(locked)
        try:
            failed = run("migrate", "--source", source, "--target", target, "--state", state,
                         "--yes", success=False)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIsNone(junction_target(source))
            self.assertFalse(target.exists())
        finally:
            unlock(holder)
        self.assertEqual(contents(source), before)
        records = history(state)
        self.assertEqual(len(records), 1)
        self.assertNotIn(records[0]["phase"], ("completed", "source_cleaned", "linked"))
        recovered = json.loads(run("recover", "--transaction", records[0]["id"], "--state", state,
                                   "--yes").stdout)
        self.assertEqual(recovered["phase"], "rolled_back")
        self.assertEqual(contents(source), before)
        self.assertFalse(target.exists())

    # P05: source files changed or added after verification are never lost.
    def test_p05_changes_after_verification_are_kept(self):
        for phase in ("verified", "source_held"):
            with self.subTest(phase=phase):
                root = fixture("p05-" + phase)
                source, target, state = root / "source", root / "target", root / "state"
                populate(source)
                run("migrate", "--source", source, "--target", target, "--state", state, "--yes",
                    phase=phase, success=False)
                record = history(state)[0]
                hold = Path(record["hold"])
                live = source if phase == "verified" else hold
                (live / "日本語-한국어.txt").write_bytes(b"changed after verify")
                (live / "added-after-verify.txt").write_bytes(b"new file")
                run("recover", "--transaction", record["id"], "--state", state, "--yes", success=False)
                values = []
                for top in (source, target, hold):
                    if top.exists() and junction_target(top) is None:
                        values += contents(top).values()
                self.assertIn(b"changed after verify", values, "modified file was lost")
                self.assertIn(b"new file", values, "file added after verification was lost")

    # P07a: an existing target is never merged or overwritten.
    def test_p07_existing_target_is_rejected(self):
        root = fixture("p07-exists")
        source, target, state = root / "source", root / "target", root / "state"
        populate(source)
        target.mkdir()
        (target / "keep.txt").write_bytes(b"existing")
        before = contents(source)
        self.assertNotEqual(run("migrate", "--source", source, "--target", target, "--state", state,
                                "--yes", success=False).returncode, 0)
        self.assertEqual(contents(target), {"keep.txt": b"existing"})
        self.assertEqual(contents(source), before)
        self.assertIsNone(junction_target(source))

    # P07b: not enough space on the target volume is detected before copying.
    def test_p07_insufficient_space_is_blocked(self):
        root, target = fixture("p07-space"), self.second_volume() / "target"
        source, state = root / "source", root / "state"
        populate(source)
        with open(source / "large.bin", "wb") as f:
            f.truncate(shutil.disk_usage(target.parent).free + 64 * 1024 * 1024)
        try:
            self.assertNotEqual(run("migrate", "--source", source, "--target", target, "--state", state,
                                    "--yes", success=False).returncode, 0)
            self.assertFalse(target.exists())
            self.assertIsNone(junction_target(source))
            verify(source)
            for record in history(state):
                self.assertEqual(record["phase"], "rolled_back", "copy started despite no space")
            verify(source)
        finally:
            (source / "large.bin").unlink()

    # P07c: a missing target drive is rejected.
    def test_p07_missing_drive_is_rejected(self):
        letter = next(c for c in "ZYXWVUTSRQPONMLKJIHG" if not os.path.exists(c + ":\\"))
        root = fixture("p07-drive")
        source, state = root / "source", root / "state"
        populate(source)
        self.assertNotEqual(run("migrate", "--source", source, "--target", letter + ":\\Dev\\x",
                                "--state", state, "--yes", success=False).returncode, 0)
        verify(source)
        self.assertIsNone(junction_target(source))

    # P07d: the target disk disappears after the source was renamed away, then comes back.
    def test_p07_target_disk_removed_mid_transaction(self):
        vhd = os.environ.get("FP_TEST_VHD")
        if not vhd:
            self.fail("Requires FP_TEST_VHD (set by test-native.ps1); not verified")
        root, target = fixture("p07-unplug"), self.second_volume() / "target"
        source, state = root / "source", root / "state"
        populate(source)
        before = contents(source)
        run("migrate", "--source", source, "--target", target, "--state", state, "--yes",
            phase="source_held", success=False)
        record = history(state)[0]
        hold = Path(record["hold"])
        powershell(f"Dismount-DiskImage -ImagePath '{vhd}'")
        try:
            self.assertFalse(target.exists())
            run("recover", "--transaction", record["id"], "--state", state, "--yes", success=False)
            self.assertEqual(contents(hold) if hold.exists() else contents(source), before,
                             "source data must survive while the target disk is missing")
        finally:
            powershell(f"Mount-DiskImage -ImagePath '{vhd}'")
            for _ in range(50):
                if target.parent.exists():
                    break
                time.sleep(0.2)
        self.assertTrue(target.parent.exists(), "test VHD did not return on the same drive letter")
        recovered = json.loads(run("recover", "--transaction", record["id"], "--state", state,
                                   "--yes").stdout)
        self.assertIn(recovered["phase"], ("completed", "rolled_back"))
        self.assertEqual(contents(source if recovered["phase"] == "rolled_back" else target), before)

    # P08: special objects that must not be restored or migrated.
    def test_p08_special_objects_are_blocked(self):
        root = fixture("p08")
        state = root / "state"
        broken, gone = root / "broken", root / "gone"
        populate(gone)
        run("__junction-test", "--source", broken, "--target", gone)
        shutil.rmtree(gone)
        cycle = root / "cycle"
        cycle.mkdir()
        (cycle / "freeport-fixture.marker").write_bytes(b"FreePort test fixture\n")
        loop = cycle / "loop"
        run("__junction-test", "--source", loop, "--target", cycle)
        mount, volume = root / "mount", None
        if args.second_volume:
            mount.mkdir()
            volume = subprocess.run(["mountvol", args.second_volume, "/L"], capture_output=True,
                                    text=True, check=True).stdout.strip()
            subprocess.run(["mountvol", str(mount), volume], check=True, capture_output=True)
        try:
            for link in [broken, loop] + ([mount] if volume else []):
                with self.subTest(restore=link.name):
                    self.assertNotEqual(run("restore", "--source", link, "--state", state, "--yes",
                                            success=False).returncode, 0)
                    self.assertTrue(is_reparse(link), "link was removed")
            with self.subTest(migrate="nested junction"):
                nested = root / "nested"
                populate(nested)
                (nested / "freeport-fixture.marker").write_bytes(b"FreePort test fixture\n")
                run("__junction-test", "--source", nested / "inner", "--target", nested / "empty")
                before = contents(nested)
                self.assertNotEqual(run("migrate", "--source", nested, "--target", root / "t-nested",
                                        "--state", state, "--yes", success=False).returncode, 0)
                self.assertEqual(contents(nested), before)
                self.assertFalse((root / "t-nested").exists())
            with self.subTest(migrate="external hard link"):
                shared = root / "shared"
                populate(shared)
                os.link(shared / "日本語-한국어.txt", root / "outside-link.txt")
                self.assertNotEqual(run("migrate", "--source", shared, "--target", root / "t-hard",
                                        "--state", state, "--yes", success=False).returncode, 0)
                verify(shared)
                self.assertIsNone(junction_target(shared))
            with self.subTest(apply="parent/child overlap"):
                parent = root / "parent"
                populate(parent)
                plan = write_plan(root / "overlap.json", [(parent, root / "t-parent"),
                                                          (parent / "empty", root / "t-child")])
                self.assertNotEqual(run("apply", "--plan", plan, "--state", state, "--yes",
                                        success=False).returncode, 0)
                verify(parent)
                self.assertFalse((root / "t-parent").exists() or (root / "t-child").exists())
            for record in history(state):
                self.assertIn(record["phase"], ("completed", "restored", "rolled_back"),
                              "a blocked operation left an open transaction")
        finally:
            if volume:
                subprocess.run(["mountvol", str(mount), "/D"], capture_output=True)

    # P09: a batch with one failing item is never reported as a whole success.
    def test_p09_partial_batch_is_reported_accurately(self):
        root = fixture("p09")
        state = root / "state"
        sources = [root / f"s{i}" for i in range(3)]
        for s in sources:
            populate(s)
        locked = sources[1] / "locked.db"
        locked.write_bytes(b"in use")
        plan = write_plan(root / "plan.json", [(s, root / f"t{i}") for i, s in enumerate(sources)])
        holder = lock(locked)
        try:
            self.assertNotEqual(run("apply", "--plan", plan, "--state", state, "--yes",
                                    success=False).returncode, 0)
        finally:
            unlock(holder)
        batches = json.loads(run("history", "--state", state, "--batches").stdout)
        self.assertEqual(len(batches), 1)
        self.assertNotEqual(batches[0]["phase"], "completed")
        status = [i["status"] for i in batches[0]["items"]]
        self.assertEqual(status[0], "completed")
        self.assertNotEqual(status[1], "completed")
        self.assertNotEqual(status[2], "completed")
        self.assertEqual(junction_target(sources[0]), os.path.normcase(str(root / "t0")))
        verify(sources[1])
        verify(sources[2])
        self.assertIsNone(junction_target(sources[2]))


if __name__ == "__main__":
    handles_test = "test_force_closes_only_own_test_holder_selected_handles"
    if args.test == "handles":
        suite = unittest.TestSuite([NativeAcceptance(handles_test)])
    else:
        # Force-closing handles is deferred to the next milestone (docs/requirements.md 2.1).
        names = [n for n in unittest.defaultTestLoader.getTestCaseNames(NativeAcceptance) if n != handles_test]
        suite = unittest.TestSuite([NativeAcceptance(n) for n in names])
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    Path(args.out).write_text(json.dumps({"tests": result.testsRun, "failures": len(result.failures),
        "errors": len(result.errors), "passed": result.wasSuccessful(), "power_loss_tested": False}, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if result.wasSuccessful() else 1)
