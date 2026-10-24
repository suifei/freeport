"""Install local owner-identity hooks without embedding machine paths in Git."""
import ctypes
import pathlib
import subprocess
import sys

root = pathlib.Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
hooks = root / ".build" / "git-hooks"
hooks.mkdir(parents=True, exist_ok=True)
interpreter = sys.executable
if sys.platform == "win32" and " " in interpreter:
    buffer = ctypes.create_unicode_buffer(32768)
    if not ctypes.windll.kernel32.GetShortPathNameW(interpreter, buffer, len(buffer)):
        raise SystemExit("Cannot resolve a space-free Python interpreter path for Git hooks.")
    interpreter = buffer.value
if " " in interpreter:
    raise SystemExit("Git hooks require an interpreter path without spaces on this machine.")
interpreter = interpreter.replace("\\", "/")
for name in ("commit-msg", "pre-push"):
    body = (
        "#!" + interpreter + "\n"
        "import runpy, sys\n"
        "sys.path.insert(0, " + repr(str(root / ".githooks")) + ")\n"
        "runpy.run_path(" + repr(str(root / ".githooks" / name)) + ", run_name='__main__')\n"
    )
    path = hooks / name
    path.write_text(body, encoding="utf-8", newline="\n")
    path.chmod(0o755)
subprocess.run(["git", "config", "--local", "core.hooksPath", ".build/git-hooks"], check=True)
print("Repository-owner identity hooks installed locally.")
