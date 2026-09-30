<#
.SYNOPSIS
    ExamSafe - one switch to make this PC exam-safe, and put everything back afterwards.

.DESCRIPTION
    Commands:
      scan     Show everything found and how it is classified (read-only).
      verify   Pass/fail check that nothing on the block list is running. Saves a report.
      on       Enter exam mode: snapshot, then close/disable everything marked 'stop'.
               Running it again while on re-applies (catches apps that restarted).
      off      Leave exam mode: restore exactly what was running/enabled before.
      status   Is exam mode on, and what is being held?
      gui      Open the ExamSafe window (default when started from ExamSafe.cmd).

.EXAMPLE
    .\ExamSafe.ps1 on -WhatIf     # show what exam mode would do, change nothing
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [Parameter(Position = 0)] [ValidateSet('scan', 'verify', 'on', 'off', 'status', 'gui')] [string] $Command = 'gui',
    [switch] $All,
    [double] $ExamWindowHours = 5
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
foreach ($lib in 'Rules', 'Inventory', 'ExamMode', 'Verify') { . (Join-Path $PSScriptRoot "lib\$lib.ps1") }

$paths = [pscustomobject]@{
    Root         = $root
    DefaultRules = Join-Path $root 'config\default-rules.json'
    Profile      = Join-Path $root 'config\my-profile.json'
    StateDir     = Join-Path $root 'state'
    ReportDir    = Join-Path $root 'reports'
}

function Test-EsIsAdmin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Invoke-EsScan {
    $ruleSet = Import-EsRuleSet -DefaultPath $paths.DefaultRules -ProfilePath $paths.Profile
    $inventory = Get-EsInventory
    $classified = Get-EsClassification -Items $inventory.Items -RuleSet $ruleSet
    [pscustomobject]@{ Items = (Select-EsRelevantItems -Items $classified); Errors = $inventory.Errors; RuleSet = $ruleSet }
}

function Write-EsScanErrors {
    param([string[]] $Errors)
    foreach ($err in $Errors) { Write-Warning $err }
}

switch ($Command) {
    'gui' {
        if (-not (Test-EsIsAdmin)) {
            # Toggling services, machine-wide startup entries and other users' tasks needs admin.
            $launchArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-STA', '-WindowStyle', 'Hidden', '-File', "`"$PSCommandPath`"", 'gui')
            try {
                Start-Process -FilePath (Get-Process -Id $PID).Path -ArgumentList $launchArgs -Verb RunAs
            } catch {
                Write-Warning 'Administrator rights were declined - opening in read-only mode (scan and verify still work).'
                . (Join-Path $PSScriptRoot 'Gui.ps1')
                Show-EsWindow -Paths $paths -IsAdmin $false
            }
            return
        }
        . (Join-Path $PSScriptRoot 'Gui.ps1')
        Show-EsWindow -Paths $paths -IsAdmin $true
    }
    'scan' {
        $scan = Invoke-EsScan
        Write-EsScanErrors $scan.Errors
        $order = @{ stop = 0; unclassified = 1; review = 2; allow = 3 }
        $scan.Items |
            Where-Object { $All -or $_.Action -ne 'allow' } |
            Sort-Object { $order[$_.Action] }, CategoryLabel, Type, Name |
            Format-Table @{ n = 'Action'; e = { $_.Action } }, @{ n = 'Category'; e = { $_.CategoryLabel } }, Type,
                @{ n = 'Name'; e = { $_.DisplayName } }, State -AutoSize -Wrap
        if (-not $All) { Write-Host "(allowed items hidden - add -All to show them)" -ForegroundColor DarkGray }
    }
    'verify' {
        $scan = Invoke-EsScan
        Write-EsScanErrors $scan.Errors
        $findings = Get-EsVerifyFindings -ClassifiedItems $scan.Items -ExamWindowHours $ExamWindowHours
        $summary = Get-EsVerifySummary -Findings $findings
        $examOn = [bool](Read-EsExamState -StateDir $paths.StateDir)
        $report = Write-EsVerifyReport -Findings $findings -Summary $summary -ReportDir $paths.ReportDir `
            -ItemCount $scan.Items.Count -ExamModeOn $examOn -ScanErrors $scan.Errors
        $colors = @{ FAIL = 'Red'; WARN = 'Yellow'; REVIEW = 'Cyan' }
        foreach ($f in $findings) { Write-Host ("{0,-7}{1}" -f $f.Severity, $f.Message) -ForegroundColor $colors[$f.Severity] }
        $verdictColor = if ($summary.Passed) { 'Green' } else { 'Red' }
        Write-Host "`nVERDICT: $($summary.Verdict)  (fail $($summary.Fail), warn $($summary.Warn), review $($summary.Review))" -ForegroundColor $verdictColor
        Write-Host "Report: $report" -ForegroundColor DarkGray
        if (-not $summary.Passed) { exit 1 }
    }
    'on' {
        if (-not $WhatIfPreference -and -not (Test-EsIsAdmin)) { throw 'Exam mode needs an administrator PowerShell (or use ExamSafe.cmd). Add -WhatIf to preview without admin.' }
        $scan = Invoke-EsScan
        Write-EsScanErrors $scan.Errors
        $results = @(Enable-EsExamMode -ClassifiedItems $scan.Items -StateDir $paths.StateDir)
        if ($WhatIfPreference) { return }
        $results | Format-Table Ok, Type, Name, Message -AutoSize -Wrap
        $failed = @($results | Where-Object { -not $_.Ok }).Count
        Write-Host "Exam mode ON. $($results.Count - $failed) changed, $failed failed. Run 'verify' to double-check." -ForegroundColor $(if ($failed) { 'Yellow' } else { 'Green' })
    }
    'off' {
        if (-not $WhatIfPreference -and -not (Test-EsIsAdmin)) { throw 'Restoring needs an administrator PowerShell (or use ExamSafe.cmd).' }
        if (-not (Read-EsExamState -StateDir $paths.StateDir)) { Write-Host 'Exam mode is not on - nothing to restore.'; return }
        $results = @(Disable-EsExamMode -StateDir $paths.StateDir)
        if ($WhatIfPreference) { return }
        $results | Format-Table Ok, Type, Name, Message -AutoSize -Wrap
        $failed = @($results | Where-Object { -not $_.Ok }).Count
        if ($failed) { Write-Host "$failed item(s) could not be restored - they are kept; run 'off' again to retry." -ForegroundColor Yellow }
        else { Write-Host 'Exam mode OFF. Everything restored.' -ForegroundColor Green }
    }
    'status' {
        $state = Read-EsExamState -StateDir $paths.StateDir
        if (-not $state) { Write-Host 'Exam mode: OFF'; return }
        Write-Host "Exam mode: ON since $(([datetime]$state.StartedAt).ToString('yyyy-MM-dd HH:mm')) - holding $(@($state.Entries).Count) item(s):"
        $state.Entries | Format-Table Type, Name, RuleLabel, @{ n = 'WasRunning'; e = { $_.PriorRunning } }, LastError -AutoSize -Wrap
    }
}
