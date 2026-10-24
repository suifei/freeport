"""Package an explicitly unapproved development preview, never a stable release."""
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target/x86_64-win7-windows-gnu/release"
REPORT = ROOT / ".build"
DIST = ROOT / "dist"
DIST.mkdir(exist_ok=True)
allowed = [".gitattributes", ".gitignore", ".githooks", ".github", "AGENTS.md", "Cargo.toml", "Cargo.lock",
           "README.md", "README.zh-CN.md", "build.rs", "src", "docs", "rules", "locales", "resources",
           "scripts", "third-party-notices"]
pe = json.loads((REPORT / "pe-audit.json").read_text(encoding="utf-8"))
smoke = json.loads((REPORT / "release-smoke-result.json").read_text(encoding="utf-8"))
validation = json.loads((REPORT / "validation-summary.json").read_text(encoding="utf-8"))
if not smoke["passed"] or not all(entry["passed"] for entry in pe):
    raise SystemExit("Preview interface/import checks failed; no package created")
validation.update({"version": "0.1.0-dev", "target": "x86_64-win7-windows-gnu", "host": platform.version(),
                   "interface_tests": smoke, "pe_audit": pe, "binaries": []})
for name in ["fp.exe", "freeport.exe"]:
    path = BIN / name
    data = path.read_bytes()
    validation["binaries"].append({"name": name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
validation["build_note"] = "Local offline development build with matching rust-src and manifest-only trimming of unused non-Windows dependencies. CI and actual Windows 7/power-loss gates have not run."
if validation["stable_release_approved"]:
    raise SystemExit("This script is only for unapproved development previews")
(REPORT / "build-report.json").write_text(json.dumps(validation, indent=2) + "\n", encoding="utf-8")
first = """FreePort 0.1.0 development preview / 开发预览

NOT APPROVED FOR REAL USER DATA. 未通过真实用户数据操作的安全验收。
Read source/docs/implementation-status.md and build-report.json first.

36 unit tests: 27 passed, 9 failed with access denied in this restricted environment.
8 six-language interface checks passed. Isolated handle closure remains unverified.
Actual Windows 7, VM power loss and cross-volume native acceptance are incomplete.
No real user data migration was performed. No Git commit, push or GitHub release was created.

bin/fp.exe       CLI development build
bin/freeport.exe Native Win32 GUI development build
source/          Source, specifications, tests, CI and third-party notices

The project owner has not selected a FreePort source license. Third-party notices
only preserve dependencies' existing terms. Both EXEs contain all six locales and
import only system DLLs checked by the included PE report.
"""
archive = DIST / "FreePort-dev-preview-v0.1.0.zip"
with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as out:
    out.writestr("READ-FIRST.txt", first)
    out.write(REPORT / "build-report.json", "build-report.json")
    for name in ["fp.exe", "freeport.exe"]:
        out.write(BIN / name, "bin/" + name)
    for name in allowed:
        path = ROOT / name
        files = sorted(path.rglob("*")) if path.is_dir() else [path]
        for file in files:
            if file.is_file() and "__pycache__" not in file.parts and file.suffix != ".pyc":
                out.write(file, "source/" + file.relative_to(ROOT).as_posix())
with zipfile.ZipFile(archive) as check:
    if check.testzip() is not None:
        raise SystemExit("Archive integrity failed")
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
(DIST / "SHA256SUMS.txt").write_text(f"{digest}  {archive.name}\n", encoding="ascii")
print(json.dumps({"archive": str(archive), "bytes": archive.stat().st_size, "sha256": digest}))
