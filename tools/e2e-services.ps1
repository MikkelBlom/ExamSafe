<#
.SYNOPSIS
    End-to-end test of services and scheduled tasks: make-safe pauses them (journal first),
    restore puts back exactly the recorded start type (delayed start included) and state.

.DESCRIPTION
    Only touches test targets this script creates itself and removes afterwards:
    - a scheduled task "\ExamSafe E2E Task" (runs nothing useful, trigger far in the future);
    - a do-nothing Windows service "ExamSafeE2ESvc" (compiled here from a few lines of C#),
      set to Automatic (delayed start) and running.
    The task part runs as a normal user. The service part needs an elevated (admin) terminal,
    because creating a service does; without one it is skipped and says so (use
    -RequireServices to make that a failure).

    Uses a test catalog listing only these two and a throwaway state folder, so your real
    exam-mode record and your real services and tasks are never touched. The elevated helper is
    run without a UAC prompt (EXAMSAFE_HELPER=direct), with this terminal's rights.
#>
param(
    [string] $Exe = (Join-Path $PSScriptRoot '..\target\release\examsafe.exe'),
    [switch] $RequireServices
)
$ErrorActionPreference = 'Stop'

$taskName = 'ExamSafe E2E Task'
$serviceName = 'ExamSafeE2ESvc'
$work = Join-Path ([IO.Path]::GetTempPath()) "examsafe-e2e-svc-$PID"
New-Item -ItemType Directory -Force -Path $work | Out-Null
$elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)

@"
{ "version": 1, "entries": [
  { "id": "e2e", "name": "ExamSafe E2E", "category": "Test",
    "services": ["$serviceName"], "tasks": ["\\$taskName"] } ] }
"@ | Set-Content (Join-Path $work 'catalog.json')
$env:EXAMSAFE_CATALOG = Join-Path $work 'catalog.json'
$env:EXAMSAFE_STATE_DIR = Join-Path $work 'state'
$env:EXAMSAFE_HELPER = 'direct'

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
function Get-TestTask { Get-ScheduledTask -TaskPath '\' -TaskName $taskName -ErrorAction SilentlyContinue }
function Get-TestService { Get-CimInstance Win32_Service -Filter "Name = '$serviceName'" -ErrorAction SilentlyContinue }

function New-TestService {
    # A do-nothing service, compiled by Windows PowerShell 5.1 (PowerShell 7 cannot emit exes).
    $exe = Join-Path $work "$serviceName.exe"
    $source = @"
using System.ServiceProcess;
public class ExamSafeE2EService : ServiceBase {
    public ExamSafeE2EService() { ServiceName = "$serviceName"; }
    public static void Main() { ServiceBase.Run(new ExamSafeE2EService()); }
}
"@
    $sourceFile = Join-Path $work 'service.cs'
    Set-Content $sourceFile $source
    powershell.exe -NoProfile -Command "Add-Type -TypeDefinition (Get-Content -Raw '$sourceFile') -ReferencedAssemblies System.ServiceProcess -OutputAssembly '$exe' -OutputType ConsoleApplication"
    if (-not (Test-Path $exe)) { throw 'Could not compile the test service.' }
    sc.exe create $serviceName binPath= "`"$exe`"" start= delayed-auto DisplayName= 'ExamSafe E2E test service' | Out-Null
    sc.exe start $serviceName | Out-Null
    for ($i = 0; $i -lt 20 -and (Get-TestService).State -ne 'Running'; $i++) { Start-Sleep -Milliseconds 500 }
}

try {
    if (Get-TestTask) { throw "A task named '$taskName' already exists - remove it and run again." }
    if (Get-TestService) { throw "A service named '$serviceName' already exists - remove it and run again." }

    # --- Arrange: the test targets ------------------------------------------------------------
    $action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument '/c exit 0'
    $trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddYears(5)
    Register-ScheduledTask -TaskName $taskName -TaskPath '\' -Action $action -Trigger $trigger `
        -Description 'Created and removed by ExamSafe tools/e2e-services.ps1' | Out-Null
    Check ((Get-TestTask).State -ne 'Disabled') 'test task exists and is enabled'

    $withServices = $false
    if ($elevated) {
        New-TestService
        $svc = Get-TestService
        Check ($svc.State -eq 'Running' -and $svc.StartMode -eq 'Auto' -and $svc.DelayedAutoStart) 'test service runs, Automatic (delayed start)'
        $withServices = $true
    } elseif ($RequireServices) {
        throw 'The service part needs an elevated terminal (Run as administrator).'
    } else {
        Write-Host '  SKIP  service part (needs an elevated terminal) - task part only' -ForegroundColor Yellow
    }

    # --- Act + assert -------------------------------------------------------------------------
    $scan = Invoke-Cli @('scan')
    Check ($scan.Text -match [regex]::Escape("$taskName (scheduled task)") -and $scan.Text -match 'will disable') 'scan finds the task and plans to disable it'
    if ($withServices) {
        Check ($scan.Text -match 'ExamSafe E2E test service \(service\)' -and $scan.Text -match 'Automatic \(delayed\); will stop \+ set Disabled') 'scan finds the service with its delayed start type'
    }

    $safe = Invoke-Cli @('make-safe', '--yes')
    Check ($safe.Code -eq 0) 'make-safe --yes succeeds'
    Check ((Get-TestTask).State -eq 'Disabled') 'task is really disabled'
    if ($withServices) {
        $svc = Get-TestService
        Check ($svc.State -eq 'Stopped' -and $svc.StartMode -eq 'Disabled') 'service is really stopped and disabled'
    }

    $status = Invoke-Cli @('status')
    Check ($status.Text -match 'Exam mode: on' -and $status.Text -match [regex]::Escape("\$taskName (scheduled task) -> enabled")) 'journal records the task as it was (enabled)'
    if ($withServices) {
        Check ($status.Text -match 'ExamSafe E2E test service \(service\) -> Automatic \(delayed\), running') 'journal records the service as it was (delayed start, running)'
    }

    $restore = Invoke-Cli @('restore')
    Check ($restore.Code -eq 0) 'restore succeeds'
    Check ((Get-TestTask).State -ne 'Disabled') 'task is enabled again'
    if ($withServices) {
        $svc = Get-TestService
        Check ($svc.State -eq 'Running') 'service is running again'
        Check ($svc.StartMode -eq 'Auto' -and $svc.DelayedAutoStart) 'service is Automatic (delayed start) again, exactly as before'
    }
    $after = Invoke-Cli @('status')
    Check ($after.Text -match 'Exam mode: off') 'exam mode is off after restore'

    # Already off: nothing is changed or journaled.
    Disable-ScheduledTask -TaskName $taskName -TaskPath '\' | Out-Null
    if ($withServices) { sc.exe stop $serviceName | Out-Null; sc.exe config $serviceName start= disabled | Out-Null; Start-Sleep -Seconds 2 }
    $nothing = Invoke-Cli @('make-safe', '--yes')
    Check ($nothing.Code -eq 0 -and $nothing.Text -match 'nothing on the exam list needed turning off') 'items that are already off need nothing'
    $cleanup = Invoke-Cli @('status')
    Check ($cleanup.Text -match 'Exam mode: off') 'nothing journaled when nothing changed'
} finally {
    Unregister-ScheduledTask -TaskName $taskName -TaskPath '\' -Confirm:$false -ErrorAction SilentlyContinue
    if (Get-TestService) {
        sc.exe stop $serviceName 2>&1 | Out-Null
        Start-Sleep -Seconds 1
        sc.exe delete $serviceName 2>&1 | Out-Null
    }
    Remove-Item Env:EXAMSAFE_CATALOG, Env:EXAMSAFE_STATE_DIR, Env:EXAMSAFE_HELPER -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($failures.Count) { Write-Host "E2E FAILED: $($failures.Count) check(s)" -ForegroundColor Red; exit 1 }
if ($withServices) { Write-Host 'E2E passed (tasks and services).' -ForegroundColor Green }
else { Write-Host 'E2E passed (tasks only - run elevated to include services).' -ForegroundColor Yellow }
