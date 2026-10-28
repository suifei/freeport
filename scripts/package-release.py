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
name = f"SaveMyCDrive-v{VERSION}-windows-x64"
readme = f"""SaveMyCDrive {VERSION}

SaveMyCDrive.exe  Windows GUI / 图形界面
smcd.exe        command line / 命令行

Moves selected folders to another drive with robocopy and leaves a directory
junction (mklink /J) at the original path; restore reverses it.
用 robocopy 把选中目录搬到其他盘，并在原路径建立目录连接；恢复为反向操作。

Requires Windows 10 or 11 x64. Close the programs SaveMyCDrive names as using a
folder before migrating it. SaveMyCDrive never ends processes itself.
需要 Windows 10/11 x64。迁移前请关闭 SaveMyCDrive 提示正在使用该目录的程序。

License: MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE).
Dependencies keep their own terms (third-party-notices/).
Source: https://github.com/suifei/save-my-c-drive
"""
entries = {}
binaries = []
for filename in ["smcd.exe", "SaveMyCDrive.exe"]:
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
# Only crates compiled into the EXEs: no build scripts, proc macros or other-platform crates.
tree = subprocess.check_output(["cargo", "+stable-x86_64-pc-windows-gnu", "tree", "--locked", "--target",
    "x86_64-pc-windows-gnu", "-e", "normal,no-proc-macro", "--prefix", "none", "-f", "{p}"], cwd=ROOT, text=True)
linked = {tuple(line.split()[:2]) for line in tree.splitlines() if line.strip()} - {("savemycdrive", "v" + VERSION)}
NOTICES = ROOT / "third-party-notices"
inventory = json.loads((NOTICES / "dependencies.json").read_text(encoding="utf-8"))
crates = [c for c in inventory if (c["name"], "v" + c["version"]) in linked]
if len(crates) != len(linked):
    raise SystemExit("Missing notices for linked crates: " + ", ".join(sorted(
        f"{n} {v}" for n, v in linked - {(c["name"], "v" + c["version"]) for c in crates})))
for c in crates:
    if not c["notices"] or not all((NOTICES / f'{c["name"]}-{c["version"]}' / n).is_file() for n in c["notices"]):
        raise SystemExit(f'Missing notice files for linked crate {c["name"]} {c["version"]}')
allowed = {f'{c["name"]}-{c["version"]}' for c in crates} | {"rust-licenses", "crt", "gcc-libs", "libwinpthread"}
for path in sorted(NOTICES.rglob("*")):
    relative = path.relative_to(NOTICES)
    if path.is_file() and (len(relative.parts) == 1 or relative.parts[0] in allowed) and relative.name != "dependencies.json":
        entries["third-party-notices/" + relative.as_posix()] = path.read_bytes()
entries["third-party-notices/dependencies.json"] = (json.dumps(crates, indent=2) + "\n").encode("utf-8")
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
checksum.write_text(f"{digest}  {archive.name}\n", encoding="ascii", newline="\n")
report = {"version": VERSION, "source_commit": git("rev-parse", "HEAD"), "binaries": binaries,
    "interface_tests": smoke, "pe_audit": pe,
    "archive": {"filename": archive.name, "bytes": archive.stat().st_size, "sha256": digest, "contains_project_source": False}}
(REPORT / f"release-{VERSION}-build-report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"archive": str(archive), "checksum": str(checksum), "sha256": digest, "binaries": binaries}))
