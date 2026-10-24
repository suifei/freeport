param([switch]$RetireOldSourceBundle)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
Set-Location (Split-Path $PSScriptRoot -Parent)
function Invoke-Checked([string]$Program,[string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed (exit $LASTEXITCODE)." }
}
$repo='suifei/freeport'
$tag='v0.1.1-dev.1'
$owner='flynn <7343085+suifei@users.noreply.github.com>'
$name='FreePort-v0.1.1-dev.1-windows-x64'
$archive=Join-Path (Get-Location) "dist\$name.zip"
$checksum=Join-Path (Get-Location) "dist\$name.sha256"
$manifest=Join-Path (Get-Location) '.build\v011-source-fingerprint.json'
if (!(Test-Path -LiteralPath $manifest)) { throw 'Missing local build fingerprint. Rebuild this preview first.' }
$fingerprint=Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
foreach ($entry in $fingerprint.files) {
    if ((Get-FileHash -LiteralPath $entry.path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry.sha256) {
        throw "Source changed after build: $($entry.path). Rebuild before publishing."
    }
}
$expected=$fingerprint.archive_sha256
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected) { throw 'Binary archive hash mismatch.' }
if ((Get-Content -LiteralPath $checksum -Raw).Trim() -cne "$expected  $name.zip") { throw 'Checksum file mismatch.' }
$identity=& gh api user 2>$null
if ($LASTEXITCODE -ne 0) {
    Invoke-Checked gh @('auth','login','--hostname','github.com','--web','--git-protocol','https','--scopes','workflow')
    $identity=& gh api user
    if ($LASTEXITCODE -ne 0) { throw 'GitHub login failed.' }
}
$identity=$identity | ConvertFrom-Json
if ($identity.login -cne 'suifei' -or $identity.id -ne 7343085) { throw 'Expected the repository owner account suifei.' }
$origin=& git remote get-url origin
if ($LASTEXITCODE -ne 0 -or $origin -notin @('https://github.com/suifei/freeport.git','git@github.com:suifei/freeport.git')) { throw 'Unexpected origin.' }
if ((& git branch --show-current) -cne 'main') { throw 'Expected main branch.' }
foreach ($key in @('GIT_AUTHOR_NAME','GIT_COMMITTER_NAME')) {
    $value=[Environment]::GetEnvironmentVariable($key)
    if ($value -and $value -cne 'flynn') { throw "Unexpected identity override: $key" }
}
foreach ($key in @('GIT_AUTHOR_EMAIL','GIT_COMMITTER_EMAIL')) {
    $value=[Environment]::GetEnvironmentVariable($key)
    if ($value -and $value -cne '7343085+suifei@users.noreply.github.com') { throw "Unexpected identity override: $key" }
}
Invoke-Checked python @('.githooks/setup.py')
$paths=@('Cargo.toml','Cargo.lock','resources/freeport.manifest','src/cli.rs','src/engine.rs','src/elevation.rs','src/folder_tree.rs','src/gui.rs','src/lib.rs','src/main.rs','src/native_dialog.rs','src/scan.rs','src/selection.rs','docs/elevation-and-selection.md','docs/implementation-status.md','docs/requirements.md','docs/technical-design.md','locales/en.json','locales/zh-CN.json','locales/zh-TW.json','locales/ja.json','locales/ko.json','locales/de.json','scripts/smoke.py','scripts/package-preview.py','scripts/publish-preview.ps1','scripts/check-powershell.py','.github/workflows/ci.yml')
$changed=@(& git diff --name-only) + @(& git diff --cached --name-only) + @(& git ls-files --others --exclude-standard)
foreach ($path in $changed) { if ($path -and $path -notin $paths) { throw "Unrelated change: $path" } }
Invoke-Checked git (@('add','--')+$paths)
& git diff --cached --quiet
if ($LASTEXITCODE -eq 1) {
    $message=Join-Path (Get-Location) '.build\v011-commit-message.txt'
    [IO.File]::WriteAllText($message,"Add directory tree selection and scoped elevation`n",[Text.UTF8Encoding]::new($false))
    Invoke-Checked git @('-c','user.name=flynn','-c','user.email=7343085+suifei@users.noreply.github.com','commit','--author',$owner,'--file',$message)
} elseif ($LASTEXITCODE -ne 0) { throw 'Cannot inspect staged changes.' }
$head=& git rev-parse HEAD
$actual=& git show -s '--format=%an <%ae>|%cn <%ce>' HEAD
if ($actual -cne "$owner|$owner") { throw 'Commit identity mismatch.' }
Invoke-Checked git @('-c','remote.origin.pushurl=https://github.com/suifei/freeport.git','-c','credential.helper=','-c','credential.helper=!gh auth git-credential','push','origin','HEAD:refs/heads/main')
$remote=& gh api "repos/$repo/commits/main" --jq '.sha'
if ($LASTEXITCODE -ne 0 -or $remote -cne $head) { throw 'Remote source commit verification failed.' }
$notes=Join-Path (Get-Location) '.build\v011-release-notes.md'
$body=@'
FreePort 0.1.1 development preview / 开发预览

Native folder tree with inherited/partial checkboxes, parent/child task reduction,
save/load selection files, CLI --selection and optional --elevate workers.
原生目录树、级联和半选、父子目录归并、保存/载入选择及按次申请管理员权限。

Binary-only ZIP: CLI, GUI, dependency notices and usage notes. No project source or user data.
ZIP 仅含二进制、必要依赖声明和说明，不含项目源码或用户数据。

Validation: 37/46 unit tests passed; 9 native fixtures failed with access denied.
All 10 new core tests and 10 interface tests passed; PE checks passed with system DLLs only.
Actual UAC interaction, Windows 7, VM power loss, complete cross-volume and forced-handle acceptance remain unverified.
仅供开发验证；原生安全验收尚未完成，不用于迁移真实用户数据。
'@
[IO.File]::WriteAllText($notes,$body,[Text.UTF8Encoding]::new($false))
Invoke-Checked gh @('release','create',$tag,$archive,$checksum,'--repo',$repo,'--target',$head,'--title','FreePort 0.1.1 - Development preview','--prerelease','--notes-file',$notes)
$release=& gh api "repos/$repo/releases/tags/$tag" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or !$release.prerelease) { throw 'Release verification failed.' }
$asset=@($release.assets | Where-Object name -CEQ "$name.zip")
if ($asset.Count -ne 1 -or $asset[0].digest -cne "sha256:$expected") { throw 'Uploaded archive verification failed.' }
if ($RetireOldSourceBundle) {
    $old=& gh api "repos/$repo/releases/tags/v0.1.0-dev.1" | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect the old source bundle.' }
    $oldAsset=@($old.assets | Where-Object name -CEQ 'FreePort-dev-preview-v0.1.0.zip')
    if ($oldAsset.Count -eq 1) {
        if ($oldAsset[0].digest -cne 'sha256:c1175f9302a79330da81b56df73a4d0bbd88f9ca5309c2253a48953a6792f193') { throw 'Unexpected old archive; not deleting.' }
        Invoke-Checked gh @('release','delete-asset','v0.1.0-dev.1','FreePort-dev-preview-v0.1.0.zip','--repo',$repo,'--yes')
    }
}
Write-Output $release.html_url
