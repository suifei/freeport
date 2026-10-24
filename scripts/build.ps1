param(
    [ValidateSet('release','dev')][string]$Profile='release',
    [string]$Toolchain='nightly'
)
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    if (-not (Get-Command windres -ErrorAction SilentlyContinue)) { throw 'MinGW-w64 gcc and windres must be on PATH (MSVCRT, not UCRT, for Windows 7).' }
    $env:RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-static'
    $cargoArguments=@("+$Toolchain",'build','--locked','--target','x86_64-win7-windows-gnu','-Zbuild-std=std,panic_abort','-Zbuild-std-features=optimize_for_size')
    if ($Profile -eq 'release') { $cargoArguments+='--release' }
    & cargo @cargoArguments
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    $folder=if($Profile -eq 'release'){'release'}else{'debug'}
    $binRoot=Join-Path $projectRoot "target/x86_64-win7-windows-gnu/$folder"
    if ($Profile -eq 'release') {
        New-Item -ItemType Directory -Force (Join-Path $projectRoot '.build') | Out-Null
        & python scripts/audit_pe.py (Join-Path $binRoot 'fp.exe') (Join-Path $binRoot 'freeport.exe') --out .build/pe-audit.json
        if ($LASTEXITCODE -ne 0) { throw 'PE import or EXE size gate failed.' }
    }
    & rustc "+$Toolchain" --version --verbose
    if ($LASTEXITCODE -ne 0) { throw 'Cannot record compiler version.' }
} finally { Pop-Location }
