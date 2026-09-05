param([string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location -LiteralPath $repoRoot
try {
    $changes = @(git status --porcelain --untracked-files=normal)
    if ($LASTEXITCODE -ne 0 -or $changes.Count -gt 0) { throw 'Commit the reviewed source first. Beta packaging requires a clean checkout.' }
    $commit = (git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify source commit.' }
    $commitDate = [DateTimeOffset]::Parse((git show -s --format=%cI HEAD).Trim())
    $metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
    $version = ($metadata.packages | Where-Object name -eq 'dayloop').version
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repoRoot 'target/beta' }
    $outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
    [IO.Directory]::CreateDirectory($outputRoot) | Out-Null
    $zipPath = Join-Path $outputRoot "dayloop-$version-windows-x64-$($commit.Substring(0,8)).zip"
    if (Test-Path -LiteralPath $zipPath) { throw 'Output ZIP already exists; no overwrite.' }
    $exe = Join-Path $repoRoot 'target/release/dayloop.exe'
    if (-not (Test-Path -LiteralPath $exe)) { throw 'This script packages a native Windows build only.' }
    $peBytes = [IO.File]::ReadAllBytes($exe)
    $peOffset = [BitConverter]::ToInt32($peBytes, 0x3c)
    if ([BitConverter]::ToUInt16($peBytes, $peOffset + 4) -ne 0x8664) { throw 'Expected a Windows x64 executable.' }
    $files = [ordered]@{ 'dayloop.exe'=$exe; 'README.md'=(Join-Path $repoRoot 'README.md'); 'LICENSE'=(Join-Path $repoRoot 'LICENSE'); 'docs/mcp-clients.md'=(Join-Path $repoRoot 'docs/mcp-clients.md'); 'docs/chat-workflows.md'=(Join-Path $repoRoot 'docs/chat-workflows.md'); 'docs/client-acceptance.md'=(Join-Path $repoRoot 'docs/client-acceptance.md') }
    $checksums = foreach ($entry in $files.GetEnumerator()) { "$( (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant())  $($entry.Key)" }
    $manifest = [ordered]@{name='dayloop'; version=$version; source_commit=$commit; source_clean=$true; target='windows-x64'; rustc=(rustc --version); signed=$false; publication='local-beta'; exe_sha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() } | ConvertTo-Json
    Add-Type -AssemblyName System.IO.Compression
    $stream = [IO.File]::Open($zipPath, [IO.FileMode]::CreateNew)
    $archive = [IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Create,$false)
    try {
        foreach ($entry in $files.GetEnumerator()) {
            $zipEntry=$archive.CreateEntry($entry.Key,[IO.Compression.CompressionLevel]::Optimal)
            $zipEntry.LastWriteTime=$commitDate
            $writer=$zipEntry.Open()
            $reader=[IO.File]::OpenRead($entry.Value)
            try { $reader.CopyTo($writer) } finally { $reader.Dispose(); $writer.Dispose() }
        }
        foreach ($entry in @{ 'manifest.json'=$manifest; 'SHA256SUMS.txt'=($checksums -join "`n") }.GetEnumerator() | Sort-Object Key) {
            $zipEntry=$archive.CreateEntry($entry.Key,[IO.Compression.CompressionLevel]::Optimal)
            $zipEntry.LastWriteTime=$commitDate
            $writer=[IO.StreamWriter]::new($zipEntry.Open(),[Text.UTF8Encoding]::new($false))
            try { $writer.Write($entry.Value) } finally { $writer.Dispose() }
        }
    } finally { $archive.Dispose(); $stream.Dispose() }
    $zipHash=(Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$zipPath.sha256", "$zipHash  $([IO.Path]::GetFileName($zipPath))`n", [Text.UTF8Encoding]::new($false))
    Write-Output $zipPath
    Write-Output "SHA256 $zipHash"
} finally { Pop-Location }
