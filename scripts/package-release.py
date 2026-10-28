"""Binary-only stable release archive. Project source, reports and user data are never included.

Run after ./scripts/build.ps1 and scripts/smoke.py on the release binaries, from a clean
working tree. Entry timestamps come from the HEAD commit, so the same build gives the same ZIP.
"""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BIN, REPORT, DIST = ROOT / "target/x86_64-pc-windows-gnu/release", ROOT / ".build", ROOT / "dist"
VERSION = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(encoding="utf-8"), re.M)[1]
if not re.fullmatch(r"\d+\.\d+\.\d+", VERSION):
    raise SystemExit(f"Not a stable version: {VERSION}")
git = lambda *a: subprocess.check_output(["git", *a], cwd=ROOT, text=True).strip()
if git("status", "--porcelain", "--untracked-files=no"):
    raise SystemExit("Working tree has local changes; build the release from a clean commit")
pe = json.loads((REPORT / "pe-audit.json").read_text(encoding="utf-8"))
smoke = json.loads((REPORT / "smoke-result.json").read_text(encoding="utf-8"))
if not smoke["passed"] or not all(x["passed"] for x in pe):
    raise SystemExit("PE audit or interface checks did not pass")
name = f"FreePort-v{VERSION}-windows-x64"
readme = f"""FreePort {VERSION}

freeport.exe  Windows GUI / 图形界面
fp.exe        command line / 命令行

Moves selected folders to another drive with robocopy and leaves a directory
junction (mklink /J) at the original path; restore reverses it.
用 robocopy 把选中目录搬到其他盘，并在原路径建立目录连接；恢复为反向操作。

Requires Windows 10 or 11 x64. Close the programs FreePort names as using a
folder before migrating it. FreePort never ends processes itself.
需要 Windows 10/11 x64。迁移前请关闭 FreePort 提示正在使用该目录的程序。

License: MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE).
Dependencies keep their own terms (third-party-notices/).
Source: https://github.com/suifei/freeport
"""
entries = {}
binaries = []
for filename in ["fp.exe", "freeport.exe"]:
    data = (BIN / filename).read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    audit = [p for p in pe if p["file"] == filename]
    if len(audit) != 1 or audit[0].get("sha256") != digest or smoke.get("binaries", {}).get(filename) != digest:
        raise SystemExit("Binary changed after validation: " + filename)
    binaries.append({"name": filename, "bytes": len(data), "sha256": digest})
    entries[filename] = data
entries["READ-ME.txt"] = b"\xef\xbb\xbf" + readme.encode("utf-8")
for filename in ["LICENSE-MIT", "LICENSE-APACHE"]:
    entries[filename] = (ROOT / filename).read_bytes()
for path in sorted((ROOT / "third-party-notices").rglob("*")):
    if path.is_file() and "__pycache__" not in path.parts:
        entries["third-party-notices/" + path.relative_to(ROOT / "third-party-notices").as_posix()] = path.read_bytes()
stamp = time.gmtime(int(git("log", "-1", "--format=%ct")))[:6]
DIST.mkdir(exist_ok=True)
archive = DIST / (name + ".zip")
with zipfile.ZipFile(archive, "w") as out:
    for entry, data in entries.items():
        info = zipfile.ZipInfo(entry, stamp)
        info.compress_type, info.external_attr = zipfile.ZIP_DEFLATED, 0o644 << 16
        out.writestr(info, data, compresslevel=9)
with zipfile.ZipFile(archive) as check:
    names = check.namelist()
    if check.testzip() or set(names) != set(entries):
        raise SystemExit("Archive integrity or allowlist failed")
    if any(n.endswith((".rs", ".ps1", ".py", ".toml", ".lock", ".json")) and not n.startswith("third-party-notices/") for n in names):
        raise SystemExit("Project source or reports are forbidden in the binary archive")
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
checksum = DIST / (name + ".sha256")
checksum.write_text(f"{digest}  {archive.name}\n", encoding="ascii")
report = {"version": VERSION, "source_commit": git("rev-parse", "HEAD"), "binaries": binaries,
    "interface_tests": smoke, "pe_audit": pe,
    "archive": {"filename": archive.name, "bytes": archive.stat().st_size, "sha256": digest, "contains_project_source": False}}
(REPORT / f"release-{VERSION}-build-report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"archive": str(archive), "checksum": str(checksum), "sha256": digest, "binaries": binaries}))
