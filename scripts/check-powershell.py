"""Read-only parser regression for Windows PowerShell 5.1.

Never runs the checked scripts, changes execution policy, or contacts GitHub.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = sorted((ROOT / "scripts").glob("*.ps1"))
engines = [shutil.which("powershell.exe")]
if not all(engines):
    raise SystemExit("Windows PowerShell 5.1 (powershell.exe) is required")
# Explicit encoding avoids a locale-dependent read of non-ASCII string literals.
for path in SCRIPTS:
    data = path.read_bytes()
    data.decode("utf-8-sig", errors="strict")
    if not data.isascii() and not data.startswith(b"\xef\xbb\xbf"):
        raise SystemExit(f"UTF-8 BOM required for non-ASCII PowerShell source: {path.name}")
paths = ",".join("'" + str(p).replace("'", "''") + "'" for p in SCRIPTS)
command = r"""
$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$results=@()
foreach($path in @(__PATHS__)) {
    $tokens=$null; $errors=$null
    $null=[System.Management.Automation.Language.Parser]::ParseFile($path,[ref]$tokens,[ref]$errors)
    $results += [PSCustomObject]@{
        file=[IO.Path]::GetFileName($path)
        version=$PSVersionTable.PSVersion.ToString()
        passed=($errors.Count -eq 0)
        errors=@($errors | ForEach-Object { [PSCustomObject]@{line=$_.Extent.StartLineNumber; id=$_.ErrorId; message=$_.Message} })
    }
}
ConvertTo-Json -InputObject $results -Depth 5
if(@($results | Where-Object { !$_.passed }).Count){exit 1}
""".replace("__PATHS__", paths)
results = []
env = dict(os.environ, VSCMD_SKIP_SENDTELEMETRY="1", POWERSHELL_TELEMETRY_OPTOUT="1")
for engine in engines:
    process = subprocess.run([engine, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", command],
                             capture_output=True, env=env, timeout=30)
    output = process.stdout.decode("utf-8-sig", errors="strict")
    try:
        checks = json.loads(output)
    except (ValueError, UnicodeError):
        print(output, file=sys.stderr)
        print(process.stderr.decode("utf-8", errors="replace"), file=sys.stderr)
        raise SystemExit(f"Could not read parser results from {engine}")
    results.extend(checks)
    if process.returncode:
        print(json.dumps(checks, ensure_ascii=False, indent=2), file=sys.stderr)
        raise SystemExit("PowerShell parser regression failed")
print(json.dumps({"passed": all(r["passed"] for r in results), "checks": results}, ensure_ascii=False, indent=2))
