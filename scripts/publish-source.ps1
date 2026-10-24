param([switch]$Push)
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    if (-not (Test-Path -LiteralPath .git)) { & git init -b main; if($LASTEXITCODE -ne 0){throw 'Cannot initialize repository.'} }
    foreach($setting in @(@('user.name','flynn'),@('user.email','7343085+suifei@users.noreply.github.com'),@('user.useConfigOnly','true'))){
        & git config --local $setting[0] $setting[1]
        if($LASTEXITCODE -ne 0){throw 'Repository configuration is not writable.'}
    }
    & python .githooks/setup.py
    if($LASTEXITCODE -ne 0){throw 'Identity hook installation failed.'}
    & python -c "import sys;sys.path.insert(0,'.githooks');from identity_policy import check_current_identity;check_current_identity()"
    if($LASTEXITCODE -ne 0){throw 'Author or committer is not the repository owner.'}
    $allowed=@('.gitattributes','.gitignore','.githooks','.github','AGENTS.md','Cargo.toml','Cargo.lock','README.md','README.zh-CN.md','build.rs','src','docs','rules','locales','resources','scripts','third-party-notices')
    & git add -- @allowed
    if($LASTEXITCODE -ne 0){throw 'Cannot stage source files.'}
    $staged=& git diff --cached --name-only
    foreach($file in $staged){if(-not ($allowed -contains ($file -split '/')[0])){throw "Unexpected staged file: $file"}}
    & git diff --cached --quiet
    if($LASTEXITCODE -eq 1){& git commit -m 'Implement directory migration core, native interface and offline languages';if($LASTEXITCODE -ne 0){throw 'Commit failed.'}}
    elseif($LASTEXITCODE -ne 0){throw 'Cannot inspect staged changes.'}
    $oid=& git rev-parse HEAD
    if($LASTEXITCODE -ne 0){throw 'No commit exists.'}
    "refs/heads/main $oid refs/heads/main 0000000000000000000000000000000000000000" | & python .githooks/pre-push
    if($LASTEXITCODE -ne 0){throw 'Pending history failed owner-identity checks.'}
    if($Push){
        $login=& gh api user --jq '.login'
        if($LASTEXITCODE -ne 0 -or $login -ne 'suifei'){throw 'GitHub authentication must be suifei.'}
        $accountId=& gh api user --jq '.id'
        if($LASTEXITCODE -ne 0 -or $accountId -ne '7343085'){throw 'GitHub account ID mismatch.'}
        $origin=& git remote get-url origin 2>$null
        if($LASTEXITCODE -eq 0){if($origin -notmatch '^(https://github\.com/|git@github\.com:)suifei/freeport(\.git)?$'){throw 'Unexpected origin; no push performed.'}}
        else{& gh repo create suifei/freeport --public --source . --remote origin --description 'Windows directory migration with recoverable NTFS junctions (development preview)';if($LASTEXITCODE -ne 0){throw 'Cannot create GitHub repository.'}}
        & git push -u origin HEAD:main
        if($LASTEXITCODE -ne 0){throw 'Push failed.'}
    }
    # This publishes source only. It does not create a stable binary release or approve failed gates.
} finally {Pop-Location}
