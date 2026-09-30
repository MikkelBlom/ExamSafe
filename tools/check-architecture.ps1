<#
.SYNOPSIS
    Fails if a crate depends on something its layer is not allowed to use (see docs/ARCHITECTURE.md).
#>
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$rules = @(
    @{ Crate = 'examsafe-core';     Forbidden = '^(slint|i-slint-.*|tray-icon|winit|windows|windows-.*|examsafe-platform|examsafe-app|examsafe-helper)$'; Why = 'core must stay pure: no UI, no OS APIs, no adapters' },
    @{ Crate = 'examsafe-platform'; Forbidden = '^(slint|i-slint-.*|tray-icon|winit|examsafe-app|examsafe-helper)$'; Why = 'platform adapters must not know about the UI' },
    @{ Crate = 'examsafe-helper';   Forbidden = '^(slint|i-slint-.*|tray-icon|winit|examsafe-app)$'; Why = 'the elevated helper must stay tiny and UI-free' }
)

$failures = @()
foreach ($rule in $rules) {
    $deps = cargo tree -p $rule.Crate -e normal --prefix none --format '{p}' 2>&1 |
        ForEach-Object { ($_ -split ' ')[0] } | Sort-Object -Unique
    if ($LASTEXITCODE -ne 0) { throw "cargo tree failed for $($rule.Crate)" }
    foreach ($dep in $deps) {
        if ($dep -match $rule.Forbidden) { $failures += "$($rule.Crate) depends on $dep - $($rule.Why)" }
    }
}

# unsafe is denied workspace-wide; the only allowed exemption is the Windows adapter module.
$allowedUnsafe = @('crates/examsafe-platform/src/windows_impl.rs')
Get-ChildItem crates -Recurse -Filter *.rs | ForEach-Object {
    $relative = (Resolve-Path -Relative $_.FullName).TrimStart('.', '\', '/') -replace '\\', '/'
    if ((Select-String -Path $_.FullName -Pattern 'allow\(unsafe_code\)' -Quiet) -and $allowedUnsafe -notcontains $relative) {
        $failures += "$relative opts out of the unsafe_code lint - unsafe belongs in examsafe-platform/*_impl.rs"
    }
}

if ($failures.Count) {
    $failures | ForEach-Object { Write-Host "ARCHITECTURE VIOLATION: $_" -ForegroundColor Red }
    exit 1
}
Write-Host 'Architecture check passed.' -ForegroundColor Green
