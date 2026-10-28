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
PREFIX = "v" + "".join(VERSION.split("-", 1)[0].split("."))
pe = json.loads((REPORT / f"{PREFIX}-pe-audit.json").read_text(encoding="utf-8"))
smoke = json.loads((REPORT / f"{PREFIX}-smoke-result.json").read_text(encoding="utf-8"))
validation = json.loads((REPORT / f"{PREFIX}-validation.json").read_text(encoding="utf-8"))
if validation.get("version") != VERSION:
    raise SystemExit("Validation receipt belongs to a different version")
if not smoke["passed"] or not all(x["passed"] for x in pe) or validation["stable_release_approved"]:
    raise SystemExit("Cannot create this development preview")
name = f"FreePort-v{VERSION}-windows-x64"
folder = DIST / name
folder.mkdir(parents=True, exist_ok=True)
binaries = []
for filename in ["fp.exe", "freeport.exe"]:
    data = (BIN / filename).read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    audit = [p for p in pe if p["file"] == filename]
    if len(audit) != 1 or audit[0].get("sha256") != digest or smoke.get("binaries", {}).get(filename) != digest:
        raise SystemExit("Binary changed after validation: " + filename)
    binaries.append({"name": filename, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    shutil.copy2(BIN / filename, folder / filename)
text = f"""FreePort {VERSION} / 开发预览

freeport.exe: native Windows GUI / 原生图形界面
fp.exe: command-line interface / 命令行

Directory tree selection, partial/inherited checkboxes, save/load selection files,
parent/child task reduction and optional one-shot administrator workers are included.
目录树支持级联、半选、保存/载入选择及父子任务归并；需要时可申请管理员权限。
Permission never enables force-close handles or application exit.
Migration discovery now focuses on user profiles, including AppData. The bundled
v2 application knowledge pack requires ownership evidence. Native tree columns
show recursive logical totals, application, status and link targets. Sibling
folders are sorted by measured totals. This does not measure reclaimable space.
管理员授权不会自动启用强关句柄或退出程序。

DEVELOPMENT PREVIEW. Native data-safety acceptance is incomplete.
仅供开发验证。原生数据安全验收尚未完成，不用于迁移真实用户数据。
Unit tests: {validation['unit_tests']['passed']} passed / {validation['unit_tests']['failed']} access-denied failures.
Interface checks: {smoke['tests']} passed.
User-reported acceptance (2026-10-24): UAC interaction, Windows 7 and power loss passed.
Complete cross-volume and forced-handle-closure acceptance remain unverified here.
用户已反馈 UAC 实际交互、Win7 和断电验收通过；完整跨盘及强制句柄关闭仍未在此环境完成验收。

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
(REPORT / f"{PREFIX}-build-report.json").write_text(json.dumps(validation, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"archive": str(archive), "folder": str(folder), "checksum": str(checksum), "sha256": digest, "binaries": binaries}))
