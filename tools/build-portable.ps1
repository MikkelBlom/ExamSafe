<#
.SYNOPSIS
    Builds the portable ExamSafe executable into dist\ExamSafe.exe.

.DESCRIPTION
    One self-contained exe: no installer, no second file. Copy it anywhere and run it.
    It stores its state in %LOCALAPPDATA%\ExamSafe, not next to the exe.
#>
$ErrorActionPreference = 'Stop'
$root = Join-Path $PSScriptRoot '..'
Push-Location $root
try {
    cargo build --release -p examsafe-app
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    $dist = Join-Path $root 'dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $target = Join-Path $dist 'ExamSafe.exe'
    Copy-Item -Force (Join-Path $root 'target\release\examsafe.exe') $target
    $item = Get-Item $target
    Write-Host ("Portable build: {0} ({1:N1} MB)" -f $item.FullName, ($item.Length / 1MB)) -ForegroundColor Green
} finally {
    Pop-Location
}
