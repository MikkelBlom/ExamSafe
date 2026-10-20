<#
.SYNOPSIS
    End-to-end test of the real exe against real apps: make-safe closes them, verify confirms
    they stay closed, restore starts them again exactly as they were.

.DESCRIPTION
    Two test targets the script starts itself, so none of your real apps are touched:
    - Character Map (charmap.exe), a normal app with a window: must come back with its window.
    - A background ping (ping.exe -n 600 127.0.0.1 -w <marker>) started in a test folder: must
      come back with the same arguments and folder, in the background.
    Uses a test catalog containing only these two and a throwaway state folder, so your real
    exam-mode record is never touched. Refuses to run if either is already running.
#>
param([string] $Exe = (Join-Path $PSScriptRoot '..\target\release\examsafe.exe'))
$ErrorActionPreference = 'Stop'

$work = Join-Path ([IO.Path]::GetTempPath()) "examsafe-e2e-$PID"
$pingDir = Join-Path $work 'ping-folder'
New-Item -ItemType Directory -Force -Path $pingDir | Out-Null
@'
{ "version": 1, "entries": [
  { "id": "charmap", "name": "Character Map", "category": "Test", "processes": ["charmap"] },
  { "id": "ping", "name": "Test ping", "category": "Test", "processes": ["ping"] } ] }
'@ | Set-Content (Join-Path $work 'catalog.json')
$env:EXAMSAFE_CATALOG = Join-Path $work 'catalog.json'
$env:EXAMSAFE_STATE_DIR = Join-Path $work 'state'
# A marker value no real ping uses, so we can tell our test process apart.
$pingArgs = '-n 600 -w 4321 127.0.0.1'

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
function Get-TestPing { @(Get-CimInstance Win32_Process -Filter "Name = 'PING.EXE'" -ErrorAction SilentlyContinue) }
function Has-Window($Processes) { @($Processes | Where-Object { $_.MainWindowHandle -ne 0 }).Count -gt 0 }

try {
    if ((Get-Charmap).Count -gt 0) { throw 'Character Map is already open - close it and run again.' }
    if ((Get-TestPing).Count -gt 0) { throw 'A ping.exe is already running - let it finish and run again.' }
    Start-Process C:\Windows\System32\charmap.exe | Out-Null
    Start-Process C:\Windows\System32\PING.EXE -ArgumentList $pingArgs -WorkingDirectory $pingDir -WindowStyle Hidden | Out-Null
    Start-Sleep -Seconds 2
    Check ((Get-Charmap).Count -eq 1) 'Character Map is running before the test'
    Check ((Get-TestPing).Count -eq 1) 'background test ping is running before the test'

    $scan = Invoke-Cli @('scan')
    Check ($scan.Code -eq 0 -and $scan.Text -match 'Character Map' -and $scan.Text -match 'Test ping') 'scan finds both'

    $noYes = Invoke-Cli @('make-safe')
    Check ($noYes.Code -eq 3 -and (Get-Charmap).Count -eq 1) 'make-safe without --yes closes nothing'

    $safe = Invoke-Cli @('make-safe', '--yes')
    Check ($safe.Code -eq 0) 'make-safe --yes succeeds'
    Check ((Get-Charmap).Count -eq 0) 'Character Map is really closed'
    Check ((Get-TestPing).Count -eq 0) 'test ping is really closed'

    $status = Invoke-Cli @('status')
    Check ($status.Text -match 'Exam mode: on' -and $status.Text -match 'charmap.exe.*with its window') 'journal records Character Map with its window'
    Check ($status.Text -match [regex]::Escape("args: $pingArgs") -and $status.Text -match 'in the background') 'journal records the ping arguments and background state'
    Check ($status.Text -match [regex]::Escape("in: $pingDir")) 'journal records the ping folder'

    # Running already: restore must not start a second copy.
    Start-Process C:\Windows\System32\charmap.exe | Out-Null
    Start-Sleep -Seconds 1

    $restore = Invoke-Cli @('restore')
    Check ($restore.Code -eq 0) 'restore succeeds'
    Start-Sleep -Seconds 2
    Check ((Get-Charmap).Count -eq 1) 'Character Map that was already running is not started twice'
    $ping = Get-TestPing
    Check ($ping.Count -eq 1) 'test ping is really started again'
    Check ($ping.Count -eq 1 -and $ping[0].CommandLine -match [regex]::Escape($pingArgs)) 'test ping got the same arguments'
    $pingIds = @($ping | ForEach-Object ProcessId)
    Check ($pingIds.Count -eq 1 -and -not (Has-Window (Get-Process -Id $pingIds -ErrorAction SilentlyContinue))) 'test ping came back without a window'
    Check ($pingIds.Count -eq 1 -and (Get-CimInstance Win32_Process -Filter "ProcessId = $($pingIds[0])").ParentProcessId -notin @(Get-Process examsafe -ErrorAction SilentlyContinue).Id) 'test ping is not a child of ExamSafe'

    $after = Invoke-Cli @('status')
    Check ($after.Text -match 'Exam mode: off') 'exam mode is off after restore'

    # The user's "leave this open" choice (from Advanced) must be honoured.
    '{ "version": 1, "left_open": ["charmap", "ping"] }' | Set-Content (Join-Path $env:EXAMSAFE_STATE_DIR 'preferences.json')
    $leftOpen = Invoke-Cli @('make-safe', '--yes')
    Check ($leftOpen.Code -eq 0) 'make-safe succeeds with the apps left open'
    Check ((Get-Charmap).Count -eq 1) 'an app the user left open is NOT closed'
    $cleanup = Invoke-Cli @('status')
    Check ($cleanup.Text -match 'Exam mode: off') 'nothing to restore when nothing was closed'
} finally {
    Get-Charmap | Stop-Process -ErrorAction SilentlyContinue
    Get-TestPing | Where-Object { $_.CommandLine -match '-w 4321' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -ErrorAction SilentlyContinue }
    Remove-Item Env:EXAMSAFE_CATALOG, Env:EXAMSAFE_STATE_DIR -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($failures.Count) { Write-Host "E2E FAILED: $($failures.Count) check(s)" -ForegroundColor Red; exit 1 }
Write-Host 'E2E passed.' -ForegroundColor Green
