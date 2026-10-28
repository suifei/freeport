"""End-to-end GUI flow test. Drives the real SaveMyCDrive.exe window with Win32 messages only.

Use a debug SaveMyCDrive.exe (fault injection). Every run copies the EXE into a new directory under
.build so its savemycdrive-state is isolated, and only touches fixtures created here. Never use real data.
"""
import argparse
import ctypes
from ctypes import wintypes as w
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
p.add_argument("--gui", required=True)
p.add_argument("--second-volume", help="Existing local test NTFS directory on another volume")
p.add_argument("--out", default=".build/gui-e2e.json")
args = p.parse_args()
GUI = Path(args.gui).resolve()
BASE = Path(tempfile.mkdtemp(prefix="gui-", dir=Path(".build").resolve()))
ENV = dict(os.environ, LOCALAPPDATA=str(BASE / "local"), USERPROFILE=str(BASE / "home"))

user32 = ctypes.WinDLL("user32", use_last_error=True)
LRESULT = ctypes.c_ssize_t
user32.SendMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
user32.SendMessageW.restype = LRESULT
user32.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
user32.GetDlgItem.argtypes = [w.HWND, ctypes.c_int]
user32.GetDlgItem.restype = w.HWND
user32.IsWindowEnabled.argtypes = [w.HWND]
user32.IsWindowVisible.argtypes = [w.HWND]
user32.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
user32.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
ENUM = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
user32.EnumWindows.argtypes = [ENUM, w.LPARAM]

WM_SETTEXT, WM_GETTEXT, WM_GETTEXTLENGTH, WM_COMMAND, WM_CLOSE = 0x0C, 0x0D, 0x0E, 0x111, 0x10
WM_KEYDOWN, VK_SPACE, VK_HOME = 0x100, 0x20, 0x24
CB_SETCURSEL, CBN_SELCHANGE = 0x14E, 1
BM_GETCHECK = 0xF0
LVM_GETITEMCOUNT, LVM_GETITEMSTATE, LVIS_STATEIMAGEMASK = 0x1004, 0x102C, 0xF000
IDOK, IDCANCEL, IDYES, IDNO = 1, 2, 6, 7
# Control IDs from src/gui.rs.
LIST, TARGET, STATUS, LOG = 3, 5, 6, 7
SCAN, ADD, APPLY, ALL, NONE, MODE, BROWSE, REQUEST, FORCE, RECOVER, ELEVATE = (
    101, 102, 103, 105, 106, 107, 108, 109, 110, 112, 114)


def text(h):
    n = user32.SendMessageW(h, WM_GETTEXTLENGTH, 0, 0)
    buf = ctypes.create_unicode_buffer(n + 1)
    user32.SendMessageW(h, WM_GETTEXT, n + 1, ctypes.addressof(buf))
    return buf.value


def set_text(h, value):
    buf = ctypes.create_unicode_buffer(value)
    user32.SendMessageW(h, WM_SETTEXT, 0, ctypes.addressof(buf))


def windows(pid):
    found = []

    def visit(h, _):
        owner = w.DWORD()
        user32.GetWindowThreadProcessId(h, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(h):
            name = ctypes.create_unicode_buffer(64)
            user32.GetClassNameW(h, name, 64)
            found.append((h, name.value))
        return True

    user32.EnumWindows(ENUM(visit), 0)
    return found


def populate(source):
    source.mkdir(parents=True)
    (source / "empty").mkdir()
    (source / "日本語-한국어.txt").write_bytes(b"verified data" * 1000)
    Path(str(source / "日本語-한국어.txt") + ":metadata").write_bytes(b"alternate stream")


def verify(source):
    assert (source / "日本語-한국어.txt").read_bytes() == b"verified data" * 1000
    assert Path(str(source / "日本語-한국어.txt") + ":metadata").read_bytes() == b"alternate stream"
    assert (source / "empty").is_dir()


def junction_target(path):
    if not os.path.lexists(path) or not os.lstat(path).st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT:
        return None
    return os.path.normcase(os.readlink(path).removeprefix("\\\\?\\"))


def history(app):
    out = subprocess.run([str(GUI.parent / "smcd.exe"), "--lang", "en", "history", "--state",
                          str(app / "savemycdrive-state")],
                         capture_output=True, check=True, env=ENV,
                         creationflags=subprocess.CREATE_NO_WINDOW).stdout
    return json.loads(out)


def planned(preview, source):
    """The target the confirmation preview shows for source ("source
→ target")."""
    lines = preview.splitlines()
    return Path(lines[lines.index(str(source)) + 1].removeprefix("→ ").strip())


class Gui:
    def __init__(self, app, fail_at=None):
        """app: a private copy of SaveMyCDrive.exe, so its savemycdrive-state is isolated per test."""
        env = ENV.copy()
        if fail_at:
            env["SMCD_TEST_FAIL_AT"] = fail_at
        self.process = subprocess.Popen([str(app / "SaveMyCDrive.exe"), "--lang", "zh-CN"], env=env)
        self.main = self.wait(lambda: next((h for h, c in windows(self.process.pid)
                                            if c == "SaveMyCDriveNativeWindow"), None), "main window")

    def wait(self, condition, what, timeout=60):
        end = time.time() + timeout
        while time.time() < end:
            value = condition()
            if value:
                return value
            if self.process.poll() is not None:
                raise AssertionError(f"GUI exited ({self.process.returncode}) while waiting for {what}")
            time.sleep(0.1)
        raise AssertionError(f"Timed out waiting for {what}; log:\n{self.log()}")

    def item(self, id):
        return user32.GetDlgItem(self.main, id)

    def log(self):
        return text(self.item(LOG))

    def click(self, id):
        user32.PostMessageW(self.main, WM_COMMAND, id, self.item(id) or 0)

    def dialogs(self):
        return [h for h, c in windows(self.process.pid) if c == "#32770"]

    def dialog(self, timeout=60):
        """Wait for a modal dialog: ('picker', hwnd) or ('message', hwnd, text)."""
        h = self.wait(lambda: (self.dialogs() or [None])[0], "dialog", timeout)
        if user32.GetDlgItem(h, 11):
            return "picker", h, ""
        return "message", h, text(user32.GetDlgItem(h, 10))

    def answer(self, h, id):
        user32.PostMessageW(h, WM_COMMAND, id, user32.GetDlgItem(h, id) or 0)
        self.wait(lambda: h not in self.dialogs(), "dialog to close")

    def pick(self, path):
        kind, h, _ = self.dialog()
        assert kind == "picker", "expected the folder picker"
        set_text(user32.GetDlgItem(h, 10), str(path))
        self.answer(h, IDOK)

    def idle(self):
        self.wait(lambda: user32.IsWindowEnabled(self.item(SCAN)) and not self.dialogs(), "idle")

    def scan(self, path, restore=False):
        combo = self.item(MODE)
        user32.SendMessageW(combo, CB_SETCURSEL, 1 if restore else 0, 0)
        user32.PostMessageW(self.main, WM_COMMAND, MODE | (CBN_SELCHANGE << 16), combo)
        self.click(ADD)
        self.pick(path)
        # The scan runs on a worker thread; wait for it to finish and populate the list.
        self.wait(lambda: user32.IsWindowEnabled(self.item(SCAN))
                  and user32.SendMessageW(self.item(LIST), LVM_GETITEMCOUNT, 0, 0) > 0, "scan results")

    def apply(self):
        self.click(APPLY)
        return self.dialog(timeout=120)

    def finish(self):
        """Wait for a launched operation; returns error dialog text, or None on success."""
        def done():
            if self.dialogs():
                return "dialog"
            return user32.IsWindowEnabled(self.item(SCAN)) and "idle"
        if self.wait(done, "operation", timeout=180) == "dialog":
            kind, h, message = self.dialog()
            self.answer(h, IDOK)
            return message
        return None

    def close(self):
        if self.process.poll() is None:
            user32.PostMessageW(self.main, WM_CLOSE, 0, 0)
            self.process.wait(timeout=20)


class GuiFlow(unittest.TestCase):
    def setUp(self):
        self.gui = None
        self.app = BASE / self._testMethodName / "app"
        self.app.mkdir(parents=True)
        shutil.copy2(GUI, self.app / "SaveMyCDrive.exe")

    def tearDown(self):
        if self.gui and self.gui.process.poll() is None:
            for h in self.gui.dialogs():
                user32.PostMessageW(h, WM_COMMAND, IDCANCEL, 0)
            self.gui.close()

    def target_root(self, name):
        if args.second_volume:
            return Path(tempfile.mkdtemp(prefix="savemycdrive-gui-", dir=Path(args.second_volume))) / name
        return BASE / self._testMethodName / "targets" / name

    def test_defaults_have_no_deferred_switches_and_nothing_checked(self):
        self.gui = Gui(self.app)
        self.assertFalse(self.gui.item(REQUEST), "request-app-exit switch must not exist")
        self.assertFalse(self.gui.item(FORCE), "force-close switch must not exist")
        self.assertEqual(user32.SendMessageW(self.gui.item(ELEVATE), BM_GETCHECK, 0, 0), 0)
        self.gui.click(APPLY)
        kind, h, message = self.gui.dialog()
        self.assertEqual(kind, "message")
        self.gui.answer(h, IDOK)
        self.gui.idle()

    def test_migrate_cancel_error_and_restore_multiple(self):
        root = BASE / self._testMethodName / "flow"
        a, b = root / "appA", root / "appB"
        populate(a)
        populate(b)
        self.gui = gui = Gui(self.app)
        dest_a, dest_b = self.target_root("a"), self.target_root("b")

        # Nothing selected after a scan: apply only informs.
        gui.scan(a)
        kind, h, _ = gui.apply()
        self.assertEqual(kind, "message")
        gui.answer(h, IDOK)
        gui.idle()

        # Select, then cancel at the confirmation preview: nothing changes.
        gui.click(ALL)
        set_text(gui.item(TARGET), str(dest_a))
        kind, h, preview = gui.apply()
        target_a = planned(preview, a)
        self.assertEqual(target_a.parent, dest_a)
        gui.answer(h, IDCANCEL)
        gui.idle()
        self.assertIsNone(junction_target(a))
        self.assertFalse(target_a.exists())
        verify(a)

        # Confirm: migrate appA.
        gui.click(ALL)
        kind, h, preview = gui.apply()
        self.assertEqual(planned(preview, a), target_a)
        gui.answer(h, IDOK)
        self.assertIsNone(gui.finish(), "migration reported an error")
        self.assertEqual(junction_target(a), os.path.normcase(str(target_a)))
        verify(target_a)
        verify(a)

        # Keyboard toggle on the list checks the candidate row; migrate appB.
        gui.scan(b)
        listing = gui.item(LIST)
        user32.PostMessageW(listing, WM_KEYDOWN, VK_HOME, 0)
        user32.PostMessageW(listing, WM_KEYDOWN, VK_SPACE, 0)
        gui.wait(lambda: user32.SendMessageW(listing, LVM_GETITEMSTATE, 0, LVIS_STATEIMAGEMASK) >> 12 == 2,
                 "keyboard check")
        set_text(gui.item(TARGET), str(dest_b))
        kind, h, preview = gui.apply()
        target_b = planned(preview, b)
        gui.answer(h, IDOK)
        self.assertIsNone(gui.finish(), "migration reported an error")
        self.assertEqual(junction_target(b), os.path.normcase(str(target_b)))

        # Existing target: error is shown and nothing is touched.
        c = root / "appC"
        populate(c)
        dest_c = self.target_root("c")
        gui.scan(c)
        gui.click(ALL)
        set_text(gui.item(TARGET), str(dest_c))
        kind, h, preview = gui.apply()
        taken = planned(preview, c)
        gui.answer(h, IDCANCEL)
        gui.idle()
        taken.mkdir(parents=True)
        (taken / "keep.txt").write_bytes(b"existing")
        gui.click(ALL)
        kind, h, message = gui.apply()
        self.assertEqual(kind, "message")
        self.assertNotIn("→", message, "an existing target must not reach the confirmation preview")
        gui.answer(h, IDOK)
        gui.idle()
        verify(c)
        self.assertIsNone(junction_target(c))
        self.assertEqual(sorted(p.name for p in taken.iterdir()), ["keep.txt"])

        # Restore mode: both links are listed, select all, restore both.
        (target_a / "after-migration.txt").write_bytes(b"current")
        gui.scan(root, restore=True)
        gui.click(ALL)
        kind, h, preview = gui.apply()
        self.assertIn(str(a), preview)
        self.assertIn(str(b), preview)
        gui.answer(h, IDOK)
        self.assertIsNone(gui.finish(), "restore reported an error")
        for source, target in ((a, target_a), (b, target_b)):
            self.assertIsNone(junction_target(source))
            verify(source)
            verify(target)
        self.assertEqual((a / "after-migration.txt").read_bytes(), b"current")
        self.assertTrue(all(r["phase"] in ("completed", "restored", "rolled_back") for r in history(self.app)))
        self.assertEqual(sum(r["phase"] == "restored" for r in history(self.app)), 2)

    def test_interrupted_migration_is_recovered_from_the_gui(self):
        source = BASE / self._testMethodName / "data" / "app"
        populate(source)
        self.gui = gui = Gui(self.app, fail_at="source_held")
        gui.scan(source)
        gui.click(ALL)
        set_text(gui.item(TARGET), str(self.target_root("dest")))
        kind, h, _ = gui.apply()
        gui.answer(h, IDOK)
        gui.process.wait(timeout=120)
        self.assertNotEqual(gui.process.returncode, 0, "fault injection needs a debug SaveMyCDrive.exe")
        [record] = history(self.app)
        self.assertEqual(record["phase"], "source_held")

        self.gui = gui = Gui(self.app)
        gui.click(RECOVER)
        kind, h, message = gui.dialog()
        self.assertIn(record["id"], message)
        gui.answer(h, IDOK)
        self.assertIsNone(gui.finish(), "recovery reported an error")
        phase = history(self.app)[0]["phase"]
        self.assertIn(phase, ("completed", "rolled_back"))
        verify(source)


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(GuiFlow))
    Path(args.out).write_text(json.dumps({"tests": result.testsRun, "failures": len(result.failures),
                                          "errors": len(result.errors), "passed": result.wasSuccessful()},
                                         indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if result.wasSuccessful() else 1)
