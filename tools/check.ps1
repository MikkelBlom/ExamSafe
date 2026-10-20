<#
.SYNOPSIS
    The local quality gate: format, lint (warnings are errors), tests and architecture rules.
    Run before every commit. There is no hosted CI; this script is the gate.

.PARAMETER Fix
    Apply rustfmt instead of only checking formatting.
#>
param([switch] $Fix)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$failed = @()
function Step([string] $Name, [scriptblock] $Command) {
    Write-Host "== $Name" -ForegroundColor Cyan
    & $Command
    if ($LASTEXITCODE -ne 0) { $script:failed += $Name; Write-Host "   FAILED: $Name" -ForegroundColor Red }
}

if ($Fix) { Step 'Format' { cargo fmt --all } }
else { Step 'Format' { cargo fmt --all --check } }
Step 'Clippy (-D warnings)' { cargo clippy --workspace --all-targets --locked -- -D warnings }
Step 'Tests' { cargo test --workspace --locked }
Step 'Architecture' { pwsh -NoProfile -File (Join-Path $PSScriptRoot 'check-architecture.ps1') }

if ($failed.Count) {
    Write-Host "CHECK FAILED: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host 'All checks passed.' -ForegroundColor Green
