param(
    [string]$Root=$env:USERPROFILE,
    [string]$Output=(Join-Path $PSScriptRoot '..\.build\research-users\metadata.json')
)
# Read-only, in-process .NET metadata enumeration. No application launches,
# external commands, network requests, credential/configuration reads or writes
# to the surveyed volume. All reparse points are boundaries, including files.
$ErrorActionPreference='Stop'
$rootPath=[IO.Path]::GetFullPath($Root)
if ($rootPath -notmatch '^[A-Za-z]:\\') { throw 'A local drive path is required.' }
if (([IO.File]::GetAttributes($rootPath) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Root is a reparse point.' }
$watch=[Diagnostics.Stopwatch]::StartNew()
$records=[Collections.Generic.Dictionary[string,object]]::new([StringComparer]::OrdinalIgnoreCase)
$links=[Collections.Generic.List[object]]::new()
$errors=[Collections.Generic.List[object]]::new()
$rootFiles=[Collections.Generic.List[object]]::new()
$stack=[Collections.Generic.Stack[object]]::new()
$stack.Push(@($rootPath,$false))
$fileCount=0L
$last=0L
while ($stack.Count) {
    $task=$stack.Pop(); $path=[string]$task[0]
    if ($task[1]) {
        $record=$records[$path]
        foreach($child in $record.children) {
            $sub=$records[$child]
            $record.logical += $sub.logical
            $record.files += $sub.files
            $record.links += $sub.links
            $record.errors += $sub.errors
        }
        $record.Remove('children')
        continue
    }
    $record=@{logical=0L;files=0L;links=0L;errors=0L;children=[Collections.Generic.List[string]]::new()}
    $records[$path]=$record
    try {
        foreach($entry in [IO.Directory]::EnumerateFileSystemEntries($path)) {
            try {
                $attributes=[IO.File]::GetAttributes($entry)
                if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    $record.links++
                    $links.Add(@{path=$entry;attributes=[int]$attributes})
                } elseif (($attributes -band [IO.FileAttributes]::Directory) -ne 0) {
                    $record.children.Add($entry)
                } else {
                    $length=([IO.FileInfo]::new($entry)).Length
                    $record.logical += $length; $record.files++; $fileCount++
                    if($path -eq $rootPath) { $rootFiles.Add(@{path=$entry;logical=$length}) }
                }
            } catch {
                $record.errors++
                $errors.Add(@{path=$entry;operation='metadata';hresult=$_.Exception.HResult})
            }
        }
    } catch {
        $record.errors++
        $errors.Add(@{path=$path;operation='enumerate';hresult=$_.Exception.HResult})
    }
    $stack.Push(@($path,$true))
    foreach($child in $record.children) { $stack.Push(@($child,$false)) }
    if($watch.ElapsedMilliseconds-$last -gt 5000) {
        Write-Output ("Directories={0}; files={1}; errors={2}; elapsed={3}s" -f $records.Count,$fileCount,$errors.Count,[int]$watch.Elapsed.TotalSeconds)
        $last=$watch.ElapsedMilliseconds
    }
}
$report=@{
    version=1;root=$rootPath;utc=[DateTime]::UtcNow.ToString('o');seconds=$watch.Elapsed.TotalSeconds
    backend='in-process-dotnet-no-reparse-follow';directories=$records;reparse_points=$links;errors=$errors;root_files=$rootFiles
    semantics='Logical path totals only; hard links counted per directory entry. Reparse targets, ADS, allocation and filesystem metadata excluded. Errors are explicit; not reclaimable space.'
}
$absoluteOutput=[IO.Path]::GetFullPath($Output)
if($absoluteOutput.StartsWith($rootPath,[StringComparison]::OrdinalIgnoreCase)) { throw 'Write the report outside the surveyed root.' }
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($absoluteOutput)) | Out-Null
$report | ConvertTo-Json -Depth 8 -Compress | Set-Content -LiteralPath $absoluteOutput -Encoding UTF8
Write-Output ("Saved {0}; directories={1}; files={2}; errors={3}; seconds={4}" -f $absoluteOutput,$records.Count,$fileCount,$errors.Count,[int]$watch.Elapsed.TotalSeconds)
