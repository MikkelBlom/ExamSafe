<#
.SYNOPSIS
    End-to-end test of the real exe against a real app: make-safe closes Character Map, verify
    confirms it stays closed, restore reopens it.

.DESCRIPTION
    Uses a test catalog containing only Character Map (charmap.exe, which nobody has open by
    accident) and a throwaway state folder, so none of your real apps or your real exam-mode
    record are touched.
#>
param([string] $Exe = (Join-Path $PSScriptRoot '..\target\release\examsafe.exe'))
$ErrorActionPreference = 'Stop'

$work = Join-Path ([IO.Path]::GetTempPath()) "examsafe-e2e-$PID"
New-Item -ItemType Directory -Force -Path $work | Out-Null
'{ "version": 1, "entries": [ { "id": "charmap", "name": "Character Map", "category": "Test", "processes": ["charmap"] } ] }' |
    Set-Content (Join-Path $work 'catalog.json')
$env:EXAMSAFE_CATALOG = Join-Path $work 'catalog.json'
$env:EXAMSAFE_STATE_DIR = Join-Path $work 'state'

$failures = @()
function Check([bool] $Condition, [string] $What) {
    if ($Condition) { Write-Host "  PASS  $What" -ForegroundColor Green }
    else { Write-Host "  FAIL  $What" -ForegroundColor Red; $script:failures += $What }
}
function Invoke-Cli([string[]] $Arguments) {
    $out = Join-Path $work 'out.txt'
    $process = Start-Process -FilePath $Exe -ArgumentList (@('--cli') + $Arguments) -Wait -PassThru `
        -NoNewWindow -RedirectStandardOutput $out
    $text = Get-Content $out -Raw
    Write-Host ("  > examsafe --cli {0}  (exit {1})" -f ($Arguments -join ' '), $process.ExitCode) -ForegroundColor DarkGray
    ($text -split "`r?`n" | Where-Object { $_ }) | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
    [pscustomobject]@{ Code = $process.ExitCode; Text = $text }
}
function Get-Charmap { @(Get-Process charmap -ErrorAction SilentlyContinue) }

try {
    if ((Get-Charmap).Count -gt 0) { throw 'Character Map is already open - close it and run again.' }
    Start-Process C:\Windows\System32\charmap.exe | Out-Null
    Start-Sleep -Seconds 2
    Check ((Get-Charmap).Count -eq 1) 'Character Map is running before the test'

    $scan = Invoke-Cli @('scan')
    Check ($scan.Code -eq 0 -and $scan.Text -match 'Character Map') 'scan finds it'

    $noYes = Invoke-Cli @('make-safe')
    Check ($noYes.Code -eq 3 -and (Get-Charmap).Count -eq 1) 'make-safe without --yes closes nothing'

    $safe = Invoke-Cli @('make-safe', '--yes')
    Check ($safe.Code -eq 0) 'make-safe --yes succeeds'
    Check ((Get-Charmap).Count -eq 0) 'Character Map is really closed'

    $status = Invoke-Cli @('status')
    Check ($status.Text -match 'Exam mode: on' -and $status.Text -match 'charmap.exe') 'journal records it with its path'

    $restore = Invoke-Cli @('restore')
    Check ($restore.Code -eq 0) 'restore succeeds'
    Start-Sleep -Seconds 2
    Check ((Get-Charmap).Count -eq 1) 'Character Map is really reopened'

    $after = Invoke-Cli @('status')
    Check ($after.Text -match 'Exam mode: off') 'exam mode is off after restore'
} finally {
    Get-Charmap | Stop-Process -ErrorAction SilentlyContinue
    Remove-Item Env:EXAMSAFE_CATALOG, Env:EXAMSAFE_STATE_DIR -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($failures.Count) { Write-Host "E2E FAILED: $($failures.Count) check(s)" -ForegroundColor Red; exit 1 }
Write-Host 'E2E passed.' -ForegroundColor Green
