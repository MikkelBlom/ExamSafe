# Exam mode: snapshot what is on, turn it off, and later put back exactly what was on before.
#
# Crash safety: the snapshot is written to disk BEFORE anything is changed, and again after every
# change. If the PC dies mid-exam, "restore" still knows what to bring back after a reboot.
# Services and startup entries are disabled (not only stopped) so a reboot does not revive them.

function Get-EsStatePath {
    param([Parameter(Mandatory = $true)] [string] $StateDir)
    Join-Path $StateDir 'exam-session.json'
}

function Read-EsExamState {
    param([Parameter(Mandatory = $true)] [string] $StateDir)
    $path = Get-EsStatePath -StateDir $StateDir
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    $state = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
    $state.Entries = @($state.Entries | Where-Object { $_ })
    $state
}

function Save-EsExamState {
    param([Parameter(Mandatory = $true)] [string] $StateDir, [Parameter(Mandatory = $true)] $State)
    Write-EsFileAtomic -Path (Get-EsStatePath -StateDir $StateDir) -Content ($State | ConvertTo-Json -Depth 8)
}

function Test-EsNeedsAction {
    # Is this 'stop' item currently on in a way exam mode should change?
    param($Item)
    switch ($Item.Type) {
        'process' { return $true }
        'service' { return ($Item.Active -or $Item.StartMode -ne 'Disabled') }
        'task'    { return $Item.Active }
        'startup' { return $Item.Active }
        default   { return $false }  # ports are only verified; their owning process is what gets stopped
    }
}

function Get-EsExamTargets {
    param([AllowEmptyCollection()] [object[]] $ClassifiedItems)
    @($ClassifiedItems | Where-Object { $_.Action -eq 'stop' -and (Test-EsNeedsAction -Item $_) })
}

function New-EsSnapshotEntry {
    # Records the state the item had BEFORE exam mode, which is what restore puts back.
    param([Parameter(Mandatory = $true)] $Item)
    [pscustomobject][ordered]@{
        Key              = $Item.Key
        Type             = $Item.Type
        Name             = $Item.Name
        DisplayName      = $Item.DisplayName
        RuleId           = $Item.RuleId
        RuleLabel        = $Item.RuleLabel
        Category         = $Item.Category
        Path             = $Item.Path
        Relaunch         = [bool]$Item.Relaunch
        PriorRunning     = [bool]$Item.Active
        PriorStartMode   = $Item.StartMode
        PriorDelayed     = [bool]$Item.DelayedAutoStart
        TaskPath         = $Item.TaskPath
        TaskName         = $Item.TaskName
        ApprovedKey      = $Item.ApprovedKey
        ApprovedValue    = $Item.ApprovedValue
        PriorApproved    = $null
        LastError        = $null
    }
}

function Merge-EsSnapshot {
    # Existing entries keep their ORIGINAL prior state (re-applying exam mode must not overwrite
    # "Slack was running" with "Slack was stopped"). Returns all entries plus the ones to act on now.
    param(
        [AllowEmptyCollection()] [object[]] $Existing,
        [AllowEmptyCollection()] [object[]] $Targets
    )
    $byKey = [ordered]@{}
    foreach ($entry in @($Existing | Where-Object { $_ })) { $byKey[$entry.Key] = $entry }
    $toApply = New-Object System.Collections.Generic.List[object]
    foreach ($item in @($Targets | Where-Object { $_ })) {
        if (-not $byKey.Contains($item.Key)) { $byKey[$item.Key] = New-EsSnapshotEntry -Item $item }
        $toApply.Add([pscustomobject]@{ Entry = $byKey[$item.Key]; Item = $item })
    }
    [pscustomobject]@{ Entries = @($byKey.Values); ToApply = $toApply.ToArray() }
}

function Get-EsApplyOrder {
    # Stop things from coming back before killing them: tasks and startup first, processes last.
    param([string] $Type)
    switch ($Type) { 'task' { 0 } 'startup' { 1 } 'service' { 2 } 'process' { 3 } default { 9 } }
}

function Get-EsProtectedPids {
    # ExamSafe must never kill itself or the shell/window that launched it.
    $protected = @($PID)
    $all = @{}
    foreach ($proc in Get-CimInstance -ClassName Win32_Process) { $all[[int]$proc.ProcessId] = [int]$proc.ParentProcessId }
    $current = $PID
    for ($i = 0; $i -lt 10 -and $all.ContainsKey($current); $i++) {
        $current = $all[$current]
        if ($current -le 4) { break }
        $protected += $current
    }
    $protected
}

function Enable-EsExamMode {
    [CmdletBinding(SupportsShouldProcess = $true)]
    param(
        [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [object[]] $ClassifiedItems,
        [Parameter(Mandatory = $true)] [string] $StateDir
    )
    $existing = Read-EsExamState -StateDir $StateDir
    $targets = Get-EsExamTargets -ClassifiedItems $ClassifiedItems
    $merge = Merge-EsSnapshot -Existing $(if ($existing) { $existing.Entries } else { @() }) -Targets $targets
    $state = if ($existing) { $existing } else {
        [pscustomobject][ordered]@{ Version = 1; StartedAt = (Get-Date).ToString('o'); Computer = $env:COMPUTERNAME; Entries = @() }
    }
    $state.Entries = $merge.Entries

    # Capture startup "approved" bytes now, before we overwrite them.
    foreach ($pair in $merge.ToApply) {
        if ($pair.Entry.Type -eq 'startup' -and -not $pair.Entry.PriorApproved -and $pair.Entry.PriorRunning) {
            $hive, $sub = Split-EsRegistryPath -Path $pair.Entry.ApprovedKey
            $bytes = Get-EsApprovedBytes -Hive $hive -ApprovedKey $sub -ValueName $pair.Entry.ApprovedValue
            if ($bytes) { $pair.Entry.PriorApproved = [Convert]::ToBase64String($bytes) }
        }
    }

    $results = New-Object System.Collections.Generic.List[object]
    if (-not $WhatIfPreference) { Save-EsExamState -StateDir $StateDir -State $state }
    $protectedPids = Get-EsProtectedPids
    foreach ($pair in ($merge.ToApply | Sort-Object { Get-EsApplyOrder -Type $_.Entry.Type })) {
        $entry = $pair.Entry
        $verb = if ($entry.Type -eq 'process') { 'Close' } else { 'Disable' }
        if (-not $PSCmdlet.ShouldProcess("$($entry.Type) '$($entry.Name)' [$($entry.RuleLabel)]", $verb)) { continue }
        try {
            Invoke-EsDisable -Entry $entry -Item $pair.Item -ProtectedPids $protectedPids
            $entry.LastError = $null
            $results.Add([pscustomobject]@{ Key = $entry.Key; Type = $entry.Type; Name = $entry.Name; Ok = $true; Message = "$verb`d" })
        } catch {
            $entry.LastError = $_.Exception.Message
            $results.Add([pscustomobject]@{ Key = $entry.Key; Type = $entry.Type; Name = $entry.Name; Ok = $false; Message = $_.Exception.Message })
        }
        Save-EsExamState -StateDir $StateDir -State $state
    }
    $results.ToArray()
}

function Invoke-EsDisable {
    param($Entry, $Item, [int[]] $ProtectedPids)
    switch ($Entry.Type) {
        'task' {
            Disable-ScheduledTask -TaskPath $Entry.TaskPath -TaskName $Entry.TaskName -ErrorAction Stop | Out-Null
            Stop-ScheduledTask -TaskPath $Entry.TaskPath -TaskName $Entry.TaskName -ErrorAction SilentlyContinue
        }
        'startup' {
            # Same format Task Manager writes: 0x03 + 3 zero bytes + FILETIME of when it was disabled.
            $bytes = [byte[]](@(3, 0, 0, 0) + [BitConverter]::GetBytes((Get-Date).ToFileTime()))
            Set-EsApprovedBytes -ApprovedKey $Entry.ApprovedKey -ValueName $Entry.ApprovedValue -Bytes $bytes
        }
        'service' {
            Set-EsServiceStartMode -Name $Entry.Name -StartMode 'Disabled' -Delayed $false
            $svc = Get-Service -Name $Entry.Name -ErrorAction Stop
            if ($svc.Status -ne 'Stopped') { Stop-Service -Name $Entry.Name -Force -ErrorAction Stop }
        }
        'process' {
            $pids = @($Item.Pids | Where-Object { $ProtectedPids -notcontains [int]$_ })
            foreach ($procId in $pids) {
                # Already-exited children (a parent took them down) are fine; anything else is a real failure.
                $proc = Get-Process -Id $procId -ErrorAction SilentlyContinue
                if ($proc) { Stop-Process -Id $procId -Force -ErrorAction Stop }
            }
        }
    }
}

function Split-EsRegistryPath {
    param([string] $Path)
    $parts = $Path.Split('\', 2)
    return $parts[0], $parts[1]
}

function Set-EsApprovedBytes {
    param([string] $ApprovedKey, [string] $ValueName, [byte[]] $Bytes)
    $hive, $sub = Split-EsRegistryPath -Path $ApprovedKey
    # CreateSubKey opens the key if it exists - it never wipes existing values.
    $key = (Get-EsRegistryBaseKey -Hive $hive).CreateSubKey($sub)
    try {
        if ($null -eq $Bytes) { $key.DeleteValue($ValueName, $false) }
        else { $key.SetValue($ValueName, $Bytes, [Microsoft.Win32.RegistryValueKind]::Binary) }
    } finally { $key.Close() }
}

function Set-EsServiceStartMode {
    param([string] $Name, [string] $StartMode, [bool] $Delayed)
    $scMode = switch ($StartMode) {
        'Auto'     { if ($Delayed) { 'delayed-auto' } else { 'auto' } }
        'Manual'   { 'demand' }
        'Disabled' { 'disabled' }
        default    { $null }
    }
    if (-not $scMode) { throw "Unsupported start mode '$StartMode' for service '$Name'." }
    $output = & sc.exe config "$Name" start= $scMode 2>&1
    if ($LASTEXITCODE -ne 0) { throw "sc.exe config $Name failed ($LASTEXITCODE): $($output -join ' ')" }
}

function Disable-EsExamMode {
    # Restores everything in the snapshot. Entries that fail stay in the state file so you can retry.
    [CmdletBinding(SupportsShouldProcess = $true)]
    param([Parameter(Mandatory = $true)] [string] $StateDir)
    $state = Read-EsExamState -StateDir $StateDir
    if (-not $state) { return @() }

    $results = New-Object System.Collections.Generic.List[object]
    $remaining = New-Object System.Collections.Generic.List[object]
    $restoreOrder = @{ service = 0; task = 1; startup = 2; process = 3 }
    $relaunched = @{}
    foreach ($entry in ($state.Entries | Sort-Object { $restoreOrder[$_.Type] })) {
        if (-not $PSCmdlet.ShouldProcess("$($entry.Type) '$($entry.Name)'", 'Restore')) { $remaining.Add($entry); continue }
        try {
            $message = Invoke-EsRestore -Entry $entry -Relaunched $relaunched
            $results.Add([pscustomobject]@{ Key = $entry.Key; Type = $entry.Type; Name = $entry.Name; Ok = $true; Message = $message })
        } catch {
            $entry.LastError = $_.Exception.Message
            $remaining.Add($entry)
            $results.Add([pscustomobject]@{ Key = $entry.Key; Type = $entry.Type; Name = $entry.Name; Ok = $false; Message = $_.Exception.Message })
        }
    }
    if ($WhatIfPreference) { return $results.ToArray() }

    $historyDir = Join-Path $StateDir 'history'
    if (-not (Test-Path -LiteralPath $historyDir)) { New-Item -ItemType Directory -Path $historyDir | Out-Null }
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    Copy-Item -LiteralPath (Get-EsStatePath -StateDir $StateDir) -Destination (Join-Path $historyDir "exam-session-$stamp.json")
    if ($remaining.Count -eq 0) {
        Remove-Item -LiteralPath (Get-EsStatePath -StateDir $StateDir)
    } else {
        $state.Entries = $remaining.ToArray()
        Save-EsExamState -StateDir $StateDir -State $state
    }
    $results.ToArray()
}

function Invoke-EsRestore {
    param($Entry, [hashtable] $Relaunched)
    switch ($Entry.Type) {
        'service' {
            Set-EsServiceStartMode -Name $Entry.Name -StartMode $Entry.PriorStartMode -Delayed ([bool]$Entry.PriorDelayed)
            if ($Entry.PriorRunning) { Start-Service -Name $Entry.Name -ErrorAction Stop; return 'Re-enabled and started' }
            return "Start mode set back to $($Entry.PriorStartMode)"
        }
        'task' {
            if ($Entry.PriorRunning) { Enable-ScheduledTask -TaskPath $Entry.TaskPath -TaskName $Entry.TaskName -ErrorAction Stop | Out-Null }
            return 'Re-enabled'
        }
        'startup' {
            $bytes = if ($Entry.PriorApproved) { [Convert]::FromBase64String($Entry.PriorApproved) } else { $null }
            Set-EsApprovedBytes -ApprovedKey $Entry.ApprovedKey -ValueName $Entry.ApprovedValue -Bytes $bytes
            return 'Startup entry re-enabled'
        }
        'process' {
            if (-not $Entry.Relaunch -or -not $Entry.PriorRunning) { return 'Not relaunched (start it yourself if needed)' }
            if (-not $Entry.Path) { throw 'Path unknown - start it manually.' }
            if ($Relaunched.ContainsKey($Entry.Path)) { return 'Already relaunched' }
            if (Get-Process -Name $Entry.Name -ErrorAction SilentlyContinue) { return 'Already running' }
            Start-EsUnelevated -Path $Entry.Path
            $Relaunched[$Entry.Path] = $true
            return 'Relaunched'
        }
    }
    return 'Nothing to restore'
}

function Start-EsUnelevated {
    # ExamSafe runs as admin; starting apps through explorer.exe gives them normal user rights,
    # exactly as if you had clicked them yourself.
    param([Parameter(Mandatory = $true)] [string] $Path)
    $aumid = Get-EsAppUserModelId -ExePath $Path
    if ($aumid) {
        Start-Process -FilePath 'explorer.exe' -ArgumentList "shell:AppsFolder\$aumid"
    } elseif (Test-Path -LiteralPath $Path) {
        Start-Process -FilePath 'explorer.exe' -ArgumentList "`"$Path`""
    } else {
        throw "Executable no longer exists: $Path"
    }
}

function Get-EsAppUserModelId {
    # Microsoft Store apps (Claude, Slack Store version, ...) cannot be started by their exe path.
    param([string] $ExePath)
    if (-not $ExePath -or $ExePath -notlike '*\WindowsApps\*') { return $null }
    try {
        $pkg = Get-AppxPackage -ErrorAction Stop | Where-Object {
            $_.InstallLocation -and $ExePath.StartsWith($_.InstallLocation + '\', [StringComparison]::OrdinalIgnoreCase)
        } | Select-Object -First 1
        if (-not $pkg) { return $null }
        $relative = $ExePath.Substring($pkg.InstallLocation.Length + 1)
        [xml]$manifest = Get-Content -Raw -LiteralPath (Join-Path $pkg.InstallLocation 'AppxManifest.xml')
        $apps = @($manifest.Package.Applications.Application)
        $app = $apps | Where-Object { $_.Executable -eq $relative } | Select-Object -First 1
        if (-not $app) { $app = $apps[0] }
        return "$($pkg.PackageFamilyName)!$($app.Id)"
    } catch {
        Write-Verbose "Could not resolve Store app id for $ExePath : $($_.Exception.Message)"
        return $null
    }
}
