<#
.SYNOPSIS
  Archives a workspace directory, keeping a fixed number of generations.

.DESCRIPTION
  Copies everything under -Path into a timestamped archive, verifies the
  archive can be opened, writes a manifest beside it, and prunes archives
  older than -Keep generations. Supports -WhatIf through ShouldProcess.

.EXAMPLE
  .\Backup-Workspace.ps1 -Path Z:\repos\RepoSphereExplorer -Destination D:\backups -Keep 5
#>

[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateScript({ Test-Path -LiteralPath $_ -PathType Container })]
    [string] $Path,

    [Parameter(Mandatory)]
    [string] $Destination,

    [ValidateRange(1, 100)]
    [int] $Keep = 7,

    [string[]] $Exclude = @('target', 'node_modules', '.git'),

    [switch] $SkipVerify
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:StartedAt = Get-Date
$script:BytesCopied = 0

function Write-Step {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Message,
        [ValidateSet('Info', 'Warn', 'Done')] [string] $Level = 'Info'
    )

    $elapsed = (Get-Date) - $script:StartedAt
    $prefix = switch ($Level) {
        'Info' { '  ' }
        'Warn' { '! ' }
        'Done' { '* ' }
    }
    Write-Host ('{0}[{1:mm\:ss}] {2}' -f $prefix, $elapsed, $Message)
}

function Get-ArchiveName {
    [OutputType([string])]
    param([string] $Root)

    $leaf = Split-Path -Path $Root -Leaf
    $stamp = (Get-Date).ToString('yyyyMMdd-HHmmss')
    return "$leaf-$stamp.zip"
}

function Select-Source {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [string[]] $Excluded
    )

    Get-ChildItem -LiteralPath $Root -Recurse -File | Where-Object {
        $relative = $_.FullName.Substring($Root.Length).TrimStart('\', '/')
        -not ($Excluded | Where-Object { $relative -like "$_*" })
    }
}

function New-Manifest {
    param(
        [Parameter(Mandatory)] [System.IO.FileInfo[]] $Files,
        [Parameter(Mandatory)] [string] $ManifestPath
    )

    $rows = foreach ($file in $Files) {
        [pscustomobject]@{
            Path     = $file.FullName
            Length   = $file.Length
            Modified = $file.LastWriteTimeUtc.ToString('o')
        }
    }

    $rows | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $ManifestPath -Encoding utf8
    return $rows.Count
}

function Test-Archive {
    param([Parameter(Mandatory)] [string] $ArchivePath)

    try {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [System.IO.Compression.ZipFile]::OpenRead($ArchivePath)
        $count = $zip.Entries.Count
        $zip.Dispose()
        return $count
    } catch {
        Write-Step -Message "archive did not open: $($_.Exception.Message)" -Level Warn
        return -1
    }
}

function Remove-OldArchives {
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [Parameter(Mandatory)] [string] $Folder,
        [Parameter(Mandatory)] [int] $KeepCount
    )

    $archives = Get-ChildItem -LiteralPath $Folder -Filter '*.zip' |
        Sort-Object -Property LastWriteTime -Descending

    $doomed = $archives | Select-Object -Skip $KeepCount
    foreach ($archive in $doomed) {
        if ($PSCmdlet.ShouldProcess($archive.FullName, 'Remove archive')) {
            Remove-Item -LiteralPath $archive.FullName -Force
            Write-Step -Message "pruned $($archive.Name)"
        }
    }

    return @($doomed).Count
}

if (-not (Test-Path -LiteralPath $Destination)) {
    New-Item -ItemType Directory -Path $Destination | Out-Null
}

$resolved = (Resolve-Path -LiteralPath $Path).Path
$archiveName = Get-ArchiveName -Root $resolved
$archivePath = Join-Path -Path $Destination -ChildPath $archiveName

Write-Step -Message "collecting from $resolved"
$files = Select-Source -Root $resolved -Excluded $Exclude
$script:BytesCopied = ($files | Measure-Object -Property Length -Sum).Sum

if ($PSCmdlet.ShouldProcess($archivePath, 'Create archive')) {
    Compress-Archive -LiteralPath $files.FullName -DestinationPath $archivePath -CompressionLevel Optimal
    Write-Step -Message ('archived {0:N0} files, {1:N1} MB' -f $files.Count, ($script:BytesCopied / 1MB))
}

$manifestPath = [System.IO.Path]::ChangeExtension($archivePath, '.manifest.json')
$listed = New-Manifest -Files $files -ManifestPath $manifestPath

if (-not $SkipVerify) {
    $entries = Test-Archive -ArchivePath $archivePath
    if ($entries -ne $listed) {
        Write-Step -Message "manifest lists $listed files, archive holds $entries" -Level Warn
    }
}

$pruned = Remove-OldArchives -Folder $Destination -KeepCount $Keep
Write-Step -Message "kept $Keep generations, pruned $pruned" -Level Done
