# Verify: turn a classified scan into a pass/fail checklist. Pure, so it is unit tested.
#
# FAIL   = something on the block list is running/armed right now.
# WARN   = not running now, but could come alive during the exam (reboot, schedule, open port).
# REVIEW = you have not decided about it yet (unknown app, or a 'review' category).

function Get-EsVerifyFindings {
    param(
        [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [object[]] $ClassifiedItems,
        [datetime] $Now = (Get-Date),
        [double] $ExamWindowHours = 5
    )
    $findings = New-Object System.Collections.Generic.List[object]
    $windowEnd = $Now.AddHours($ExamWindowHours)
    foreach ($item in $ClassifiedItems) {
        $what = if ($item.RuleLabel) { "$($item.DisplayName) [$($item.RuleLabel)]" } else { $item.DisplayName }
        if ($item.Action -eq 'stop') {
            switch ($item.Type) {
                'process' { Add-EsFinding $findings 'FAIL' $item "$what is running." }
                'service' {
                    if ($item.Active) { Add-EsFinding $findings 'FAIL' $item "Service $what is running." }
                    elseif ($item.StartMode -ne 'Disabled') { Add-EsFinding $findings 'WARN' $item "Service $what is stopped but set to '$($item.StartMode)' - it can start again." }
                }
                'task' {
                    if ($item.Active) { Add-EsFinding $findings 'FAIL' $item "Scheduled task $what is enabled." }
                }
                'startup' {
                    if ($item.Active) { Add-EsFinding $findings 'WARN' $item "$what starts automatically if you reboot." }
                }
                'port' { Add-EsFinding $findings 'FAIL' $item "Port $($item.Port) is open ($($item.OwnerName)) [$($item.RuleLabel)]." }
            }
            continue
        }

        # Allowed/reviewed tasks can still pop up a window or hog the CPU mid-exam.
        if ($item.Type -eq 'task' -and $item.Active -and $item.NextRunTime -and -not $item.IsSystem) {
            $next = [datetime]$item.NextRunTime
            if ($next -ge $Now -and $next -le $windowEnd) {
                Add-EsFinding $findings 'WARN' $item "Scheduled task $what will run at $($next.ToString('HH:mm')) - during your exam window."
            }
        }
        if ($item.IsSystem) { continue }
        if ($item.Action -eq 'unclassified') {
            if ($item.Type -eq 'port' -and $item.BindsAllInterfaces) {
                Add-EsFinding $findings 'WARN' $item "Unknown program $($item.OwnerName) accepts network connections on port $($item.Port)."
            } elseif ($item.Type -eq 'process' -or ($item.Type -eq 'service' -and $item.Active)) {
                Add-EsFinding $findings 'REVIEW' $item "$what is running and has no rule yet - is it exam-safe?"
            }
        } elseif ($item.Action -eq 'review' -and $item.Active -and $item.Type -ne 'port') {
            Add-EsFinding $findings 'REVIEW' $item "$what ($($item.CategoryLabel)) is on - check it is allowed in this exam."
        }
    }
    $findings.ToArray()
}

function Add-EsFinding {
    param($List, [string] $Severity, $Item, [string] $Message)
    $List.Add([pscustomobject]@{ Severity = $Severity; Type = $Item.Type; Name = $Item.Name; Category = $Item.CategoryLabel; Message = $Message; Key = $Item.Key })
}

function Get-EsVerifySummary {
    param([AllowEmptyCollection()] [object[]] $Findings)
    $fail = @($Findings | Where-Object Severity -eq 'FAIL').Count
    $warn = @($Findings | Where-Object Severity -eq 'WARN').Count
    $review = @($Findings | Where-Object Severity -eq 'REVIEW').Count
    $verdict = if ($fail -gt 0) { 'NOT SAFE' } elseif ($warn -gt 0) { 'SAFE WITH WARNINGS' } else { 'SAFE' }
    [pscustomobject]@{ Verdict = $verdict; Fail = $fail; Warn = $warn; Review = $review; Passed = ($fail -eq 0) }
}

function Write-EsVerifyReport {
    # Plain-text report saved next to the tool, so you can show what was checked and when.
    param(
        [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [object[]] $Findings,
        [Parameter(Mandatory = $true)] $Summary,
        [Parameter(Mandatory = $true)] [string] $ReportDir,
        [int] $ItemCount,
        [bool] $ExamModeOn,
        [string[]] $ScanErrors = @()
    )
    if (-not (Test-Path -LiteralPath $ReportDir)) { New-Item -ItemType Directory -Path $ReportDir | Out-Null }
    $now = Get-Date
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("ExamSafe verification report")
    $lines.Add("Computer : $env:COMPUTERNAME")
    $lines.Add("Time     : $($now.ToString('yyyy-MM-dd HH:mm:ss'))")
    $lines.Add("Exam mode: $(if ($ExamModeOn) { 'ON' } else { 'OFF' })")
    $lines.Add("Checked  : $ItemCount processes, services, tasks, startup entries and ports")
    $lines.Add("VERDICT  : $($Summary.Verdict)  (fail $($Summary.Fail), warn $($Summary.Warn), review $($Summary.Review))")
    foreach ($err in $ScanErrors) { $lines.Add("SCAN ERROR: $err") }
    $lines.Add('')
    foreach ($severity in @('FAIL', 'WARN', 'REVIEW')) {
        $group = @($Findings | Where-Object Severity -eq $severity)
        if ($group.Count -eq 0) { continue }
        $lines.Add("== $severity ($($group.Count)) ==")
        foreach ($f in $group) { $lines.Add("  [$($f.Type)] $($f.Message)") }
        $lines.Add('')
    }
    $lines.Add('Note: this checks this PC only. Browser extensions, open browser tabs, other devices and')
    $lines.Add('your exam rules are not covered - see README "What ExamSafe does not check".')
    $path = Join-Path $ReportDir "verify-$($now.ToString('yyyyMMdd-HHmmss')).txt"
    [System.IO.File]::WriteAllLines($path, $lines.ToArray())
    $path
}
