"""Binary-only development distribution. Project source and user data are never included."""
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BIN, REPORT, DIST = ROOT / "target/x86_64-win7-windows-gnu/release", ROOT / ".build", ROOT / "dist"
VERSION = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
pe = json.loads((REPORT / "v011-pe-audit.json").read_text(encoding="utf-8"))
smoke = json.loads((REPORT / "v011-smoke-result.json").read_text(encoding="utf-8"))
validation = json.loads((REPORT / "v011-validation.json").read_text(encoding="utf-8"))
if not smoke["passed"] or not all(x["passed"] for x in pe) or validation["stable_release_approved"]:
    raise SystemExit("Cannot create this development preview")
name = f"FreePort-v{VERSION}-windows-x64"
folder = DIST / name
folder.mkdir(parents=True, exist_ok=True)
binaries = []
for filename in ["fp.exe", "freeport.exe"]:
    data = (BIN / filename).read_bytes()
    binaries.append({"name": filename, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    shutil.copy2(BIN / filename, folder / filename)
text = f"""FreePort {VERSION} / 开发预览

freeport.exe: native Windows GUI / 原生图形界面
fp.exe: command-line interface / 命令行

Directory tree selection, partial/inherited checkboxes, save/load selection files,
parent/child task reduction and optional one-shot administrator workers are included.
目录树支持级联、半选、保存/载入选择及父子任务归并；需要时可申请管理员权限。
Permission never enables force-close handles or application exit.
管理员授权不会自动启用强关句柄或退出程序。

DEVELOPMENT PREVIEW. Native data-safety acceptance is incomplete.
仅供开发验证。原生数据安全验收尚未完成，不用于迁移真实用户数据。
Unit tests: {validation['unit_tests']['passed']} passed / {validation['unit_tests']['failed']} access-denied failures.
Interface checks: {smoke['tests']} passed. Actual UAC consent and Windows 7 runtime remain unverified.
VM power loss, complete cross-volume and forced-handle-closure acceptance are incomplete.
UAC 实际授权、Windows 7 实机、断电及完整跨卷/强关验收仍未完成。

This ZIP contains EXEs, required dependency notices and this file only.
本包只含 EXE、必要的依赖版权声明和说明，不含项目源码或用户数据。
Source and validation: https://github.com/suifei/freeport
No project license has been selected by the owner. Dependencies retain their own terms.
"""
(folder / "READ-ME.txt").write_text(text, encoding="utf-8-sig")
entries = {p.name: p for p in [folder / "fp.exe", folder / "freeport.exe", folder / "READ-ME.txt"]}
for path in sorted((ROOT / "third-party-notices").rglob("*")):
    if path.is_file() and "__pycache__" not in path.parts:
        entries["third-party-notices/" + path.relative_to(ROOT / "third-party-notices").as_posix()] = path
archive = DIST / (name + ".zip")
with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as out:
    for entry, path in entries.items():
        out.write(path, entry)
with zipfile.ZipFile(archive) as check:
    if check.testzip() or set(check.namelist()) != set(entries):
        raise SystemExit("Archive integrity or allowlist failed")
    if any(p.startswith("source/") or p.endswith((".rs", ".ps1", ".py", ".toml", ".lock")) for p in check.namelist()):
        raise SystemExit("Project source is forbidden in the binary archive")
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
checksum = DIST / (name + ".sha256")
checksum.write_text(f"{digest}  {archive.name}\n", encoding="ascii")
validation.update({"version": VERSION, "binaries": binaries, "interface_tests": smoke, "pe_audit": pe,
    "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
    "source_has_local_changes": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
    "archive": {"filename": archive.name, "bytes": archive.stat().st_size, "sha256": digest, "contains_project_source": False}})
(REPORT / "v011-build-report.json").write_text(json.dumps(validation, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"archive": str(archive), "folder": str(folder), "checksum": str(checksum), "sha256": digest, "binaries": binaries}))
