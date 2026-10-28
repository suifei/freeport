param([switch]$RetireOldSourceBundle)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
Set-Location (Split-Path $PSScriptRoot -Parent)
function Invoke-Checked([string]$Program,[string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed (exit $LASTEXITCODE)." }
}
# gh emits UTF-8. Use an ASCII wire representation so PowerShell 5.1 cannot
# decode native stdout as the current ANSI code page before JSON parsing.
function Read-GhApiJson([string]$ApiPath,[switch]$AllowMissing) {
    $savedPreference=$ErrorActionPreference
    try {
        $ErrorActionPreference='Continue'
        $response=@(& gh api $ApiPath --jq '@base64' 2>&1)
        $nativeExit=$LASTEXITCODE
    } finally { $ErrorActionPreference=$savedPreference }
    if ($nativeExit -ne 0) {
        $detail=($response | ForEach-Object { [string]$_ }) -join "`n"
        if ($AllowMissing -and $detail -match '\(HTTP 404\)') { return $null }
        throw "GitHub API failed for $ApiPath (exit $nativeExit): $detail"
    }
    try {
        $encoded=($response -join '').Trim()
        if ($encoded -notmatch '^[A-Za-z0-9+/]+={0,2}$') { throw 'Invalid base64 response.' }
        $utf8=[Text.UTF8Encoding]::new($false,$true)
        $json=$utf8.GetString([Convert]::FromBase64String($encoded))
        return ConvertFrom-Json -InputObject $json
    } catch { throw "Invalid UTF-8 GitHub JSON for ${ApiPath}: $($_.Exception.Message)" }
}
function Assert-Release($Release) {
    if ($Release.tag_name -cne $tag -or $Release.author.login -cne 'suifei' -or
        $Release.prerelease -cne $true -or $Release.draft -cne $false) {
        throw 'Unexpected existing release; nothing will be overwritten.'
    }
    foreach ($file in $files) {
        $found=@($Release.assets | Where-Object name -CEQ $file.Name)
        if ($found.Count -gt 1) { throw "Duplicate asset: $($file.Name)" }
        if ($found.Count -eq 1 -and ($found[0].digest -cne "sha256:$($file.Hash)" -or
            $found[0].size -ne $file.Size -or $found[0].state -cne 'uploaded')) {
            throw "Existing asset differs from the local build: $($file.Name). Not overwriting."
        }
    }
}
function Assert-TagSource($Ref) {
    if ($Ref.object.type -cne 'commit' -or $Ref.object.sha -cnotmatch '^[0-9a-f]{40}$') {
        throw 'Unexpected existing tag type or commit; not changing the tag.'
    }
    # Publishing-script fixes may add later commits. Keep the original release tag
    # when its compiled inputs still exactly match this build; never force-move it.
    $sourcePaths=@($fingerprint.files | ForEach-Object { $_.path })
    $compare=@('diff','--quiet',$Ref.object.sha,'HEAD','--')+$sourcePaths
    & git @compare
    if ($LASTEXITCODE -ne 0) { throw 'Existing tag compiled inputs differ from this build; not changing the release.' }
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
$files=@()
foreach ($path in @($archive,$checksum)) {
    $item=Get-Item -LiteralPath $path
    $files += [PSCustomObject]@{Path=$item.FullName;Name=$item.Name;Size=$item.Length;Hash=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()}
}
try { $identity=Read-GhApiJson 'user' } catch {
    if ($_.Exception.Message -notmatch 'HTTP 401|not logged|authentication required') { throw }
    Invoke-Checked gh @('auth','login','--hostname','github.com','--web','--git-protocol','https','--scopes','workflow')
    $identity=Read-GhApiJson 'user'
}
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
$existing=Read-GhApiJson "repos/$repo/releases/tags/$tag" -AllowMissing
$tagRef=Read-GhApiJson "repos/$repo/git/ref/tags/$tag" -AllowMissing
if ($null -ne $existing) {
    Assert-Release $existing
    if ($null -eq $tagRef) { throw 'Existing release has no tag; not recreating it automatically.' }
}
if ($null -ne $tagRef) { Assert-TagSource $tagRef }
Invoke-Checked python @('.githooks/setup.py')
$paths=@('Cargo.toml','Cargo.lock','resources/freeport.manifest','src/cli.rs','src/engine.rs','src/elevation.rs','src/folder_tree.rs','src/gui.rs','src/lib.rs','src/main.rs','src/native_dialog.rs','src/scan.rs','src/selection.rs','docs/implementation-status.md','docs/requirements.md','docs/technical-design.md','locales/en.json','locales/zh-CN.json','locales/zh-TW.json','locales/ja.json','locales/ko.json','locales/de.json','scripts/smoke.py','scripts/package-preview.py','scripts/publish-preview.ps1','scripts/check-powershell.py','scripts/test-publisher.py','.github/workflows/ci.yml')
$changed=@(& git diff --name-only) + @(& git diff --cached --name-only) + @(& git ls-files --others --exclude-standard)
foreach ($path in $changed) { if ($path -and $path -notin $paths) { throw "Unrelated change: $path" } }
Invoke-Checked git (@('add','--')+$paths)
& git diff --cached --quiet
if ($LASTEXITCODE -eq 1) {
    $message=Join-Path (Get-Location) '.build\v011-commit-message.txt'
    $commitTitle=if ($null -ne $existing) { 'Fix UTF-8 release checks and resumable publishing' } else { 'Add directory tree selection and scoped elevation' }
    [IO.File]::WriteAllText($message,$commitTitle+"`n",[Text.UTF8Encoding]::new($false))
    Invoke-Checked git @('-c','user.name=flynn','-c','user.email=7343085+suifei@users.noreply.github.com','commit','--author',$owner,'--file',$message)
} elseif ($LASTEXITCODE -ne 0) { throw 'Cannot inspect staged changes.' }
$head=& git rev-parse HEAD
$actual=& git show -s '--format=%an <%ae>|%cn <%ce>' HEAD
if ($actual -cne "$owner|$owner") { throw 'Commit identity mismatch.' }
Invoke-Checked git @('-c','remote.origin.pushurl=https://github.com/suifei/freeport.git','-c','credential.helper=','-c','credential.helper=!gh auth git-credential','push','origin','HEAD:refs/heads/main')
$remote=(Read-GhApiJson "repos/$repo/commits/main").sha
if ($remote -cne $head) { throw 'Remote source commit verification failed.' }
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
if ($null -eq $existing) {
    Invoke-Checked gh @('release','create',$tag,$archive,$checksum,'--repo',$repo,'--target',$head,'--title','FreePort 0.1.1 - Development preview','--prerelease','--notes-file',$notes)
}
$release=Read-GhApiJson "repos/$repo/releases/tags/$tag"
Assert-Release $release
$tagRef=Read-GhApiJson "repos/$repo/git/ref/tags/$tag"
Assert-TagSource $tagRef
foreach ($file in $files) {
    $asset=@($release.assets | Where-Object name -CEQ $file.Name)
    if ($asset.Count -eq 0) {
        Invoke-Checked gh @('release','upload',$tag,$file.Path,'--repo',$repo)
    }
}
$release=Read-GhApiJson "repos/$repo/releases/tags/$tag"
Assert-Release $release
foreach ($file in $files) {
    if (@($release.assets | Where-Object name -CEQ $file.Name).Count -ne 1) {
        throw "Upload is incomplete: $($file.Name). Rerun to resume."
    }
}
if ($RetireOldSourceBundle) {
    $old=Read-GhApiJson "repos/$repo/releases/tags/v0.1.0-dev.1" -AllowMissing
    if ($null -ne $old) {
        $oldAsset=@($old.assets | Where-Object name -CEQ 'FreePort-dev-preview-v0.1.0.zip')
        if ($oldAsset.Count -gt 1) { throw 'Duplicate old archive; not deleting.' }
        if ($oldAsset.Count -eq 1) {
            if ($oldAsset[0].digest -cne 'sha256:c1175f9302a79330da81b56df73a4d0bbd88f9ca5309c2253a48953a6792f193') { throw 'Unexpected old archive; not deleting.' }
            Invoke-Checked gh @('release','delete-asset','v0.1.0-dev.1','FreePort-dev-preview-v0.1.0.zip','--repo',$repo,'--yes')
        }
    }
}
Write-Output $release.html_url
