param([Parameter(Mandatory=$true)][string]$Cli)
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent $PSScriptRoot
$fixtureRoot=[IO.Path]::GetFullPath((Join-Path $projectRoot '.build/test-disks'))
if (-not $fixtureRoot.StartsWith(([IO.Path]::GetFullPath($projectRoot)+[IO.Path]::DirectorySeparatorChar),[StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture path escapes workspace.' }
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
if (-not ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Native acceptance requires an elevated disposable test machine.' }
New-Item -ItemType Directory -Force $fixtureRoot | Out-Null
$vhdPath=Join-Path $fixtureRoot ('freeport-test-'+[guid]::NewGuid().ToString('N')+'.vhd')
if (Test-Path -LiteralPath $vhdPath) { throw 'Test disk must be new.' }
$diskScript=Join-Path $fixtureRoot 'create-vhd.txt'
@("create vdisk file=`"$vhdPath`" maximum=512 type=expandable",'exit') | Set-Content -LiteralPath $diskScript -Encoding ASCII
# DiskPart only creates a new file here. No physical disk is selected or formatted.
& diskpart /s $diskScript
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $vhdPath)) { throw 'Failed to create the new test VHD.' }
$image=$null
Push-Location $projectRoot
try {
    $image=Mount-DiskImage -ImagePath $vhdPath -PassThru
    $disk=$image | Get-Disk
    if ($disk.PartitionStyle -ne 'RAW' -or $disk.Size -ne 512MB -or @('File Backed Virtual','FileBackedVirtual','Virtual') -notcontains [string]$disk.BusType) { throw 'Mounted disk identity does not match a fresh test VHD.' }
    # Formatting is restricted to the disk returned by this exact newly created VHD image.
    $partition=Initialize-Disk -Number $disk.Number -PartitionStyle MBR -PassThru | New-Partition -UseMaximumSize -AssignDriveLetter
    $volume=$partition | Format-Volume -FileSystem NTFS -NewFileSystemLabel 'FREEPORT_TEST' -Confirm:$false
    if ($volume.FileSystem -ne 'NTFS' -or $volume.FileSystemLabel -ne 'FREEPORT_TEST') { throw 'Test volume validation failed.' }
    & python scripts/acceptance.py --cli ([IO.Path]::GetFullPath($Cli)) --second-volume ($volume.DriveLetter+':\')
    if ($LASTEXITCODE -ne 0) { throw 'Strict native acceptance failed; fixtures retained.' }
} finally {
    if ($null -ne $image) { Dismount-DiskImage -ImagePath $vhdPath }
    Pop-Location
}
