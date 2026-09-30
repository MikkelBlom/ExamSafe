# Inventory: read-only snapshot of what is running or armed to run on this PC.
# Every item has the same shape (see New-EsItem) so rules, verify and the GUI can treat them alike.

function New-EsItem {
    param(
        [Parameter(Mandatory = $true)] [string] $Type,
        [Parameter(Mandatory = $true)] [string] $Name,
        [string] $DisplayName,
        [string] $Path,
        [string] $State,
        [bool] $Active,
        [bool] $IsSystem,
        [hashtable] $Extra
    )
    $item = [ordered]@{
        Key              = "$Type`:$Name"
        Type             = $Type
        Name             = $Name
        DisplayName      = if ($DisplayName) { $DisplayName } else { $Name }
        Path             = $Path
        State            = $State
        Active           = $Active
        IsSystem         = $IsSystem
        Pids             = @()
        StartMode        = $null
        DelayedAutoStart = $false
        TaskPath         = $null
        TaskName         = $null
        NextRunTime      = $null
        ApprovedKey      = $null
        ApprovedValue    = $null
        Port             = $null
        OwnerPid         = $null
        OwnerName        = $null
        BindsAllInterfaces = $false
        RuleId           = $null
        RuleLabel        = $null
        Category         = $null
        CategoryLabel    = $null
        Action           = $null
        Relaunch         = $false
    }
    if ($Extra) { foreach ($key in $Extra.Keys) { $item[$key] = $Extra[$key] } }
    [pscustomobject]$item
}

function Test-EsSystemPath {
    param([string] $Path)
    if (-not $Path) { return $true }
    $windowsDir = [Environment]::GetFolderPath('Windows')
    return $Path.TrimStart('"').StartsWith($windowsDir, [StringComparison]::OrdinalIgnoreCase)
}

function Get-EsExecutablePath {
    # Pulls the executable out of a command line such as '"C:\x y\a.exe" --flag' or 'C:\a.exe -s'.
    param([string] $CommandLine)
    if (-not $CommandLine) { return $null }
    $trimmed = $CommandLine.Trim()
    if ($trimmed.StartsWith('"')) {
        $end = $trimmed.IndexOf('"', 1)
        if ($end -gt 1) { return $trimmed.Substring(1, $end - 1) }
    }
    $exeIndex = $trimmed.IndexOf('.exe', [StringComparison]::OrdinalIgnoreCase)
    if ($exeIndex -gt 0) { return $trimmed.Substring(0, $exeIndex + 4) }
    return ($trimmed -split '\s+')[0]
}

function Get-EsProcessItems {
    $all = @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop)
    $groups = $all | Where-Object { $_.ProcessId -gt 4 } | Group-Object { [IO.Path]::GetFileNameWithoutExtension($_.Name) }
    foreach ($group in $groups) {
        $withPath = @($group.Group | Where-Object { $_.ExecutablePath })
        $path = if ($withPath.Count) { $withPath[0].ExecutablePath } else { $null }
        # No readable path usually means a protected process of another account - treat as system.
        $isSystem = Test-EsSystemPath -Path $path
        New-EsItem -Type 'process' -Name $group.Name -Path $path -State "Running ($($group.Count))" -Active $true -IsSystem $isSystem -Extra @{
            Pids = @($group.Group | ForEach-Object { [int]$_.ProcessId })
        }
    }
}

function Get-EsServiceItems {
    foreach ($svc in @(Get-CimInstance -ClassName Win32_Service -ErrorAction Stop)) {
        $path = Get-EsExecutablePath -CommandLine $svc.PathName
        $mode = [string]$svc.StartMode
        $delayed = [bool]$svc.DelayedAutoStart
        $modeText = if ($mode -eq 'Auto' -and $delayed) { 'Auto (delayed)' } else { $mode }
        New-EsItem -Type 'service' -Name $svc.Name -DisplayName $svc.DisplayName -Path $path `
            -State "$($svc.State), $modeText" -Active ($svc.State -eq 'Running') -IsSystem (Test-EsSystemPath -Path $path) -Extra @{
                StartMode        = $mode
                DelayedAutoStart = $delayed
                Pids             = @(if ($svc.ProcessId) { [int]$svc.ProcessId })
            }
    }
}

function Get-EsTaskItems {
    # Microsoft's own tasks are skipped entirely: there are hundreds and none are exam risks.
    $tasks = @(Get-ScheduledTask -ErrorAction Stop | Where-Object { $_.TaskPath -notlike '\Microsoft\*' })
    foreach ($task in $tasks) {
        $fullName = "$($task.TaskPath)$($task.TaskName)"
        $action = @($task.Actions | Where-Object { $_.Execute }) | Select-Object -First 1
        $path = if ($action) { Get-EsExecutablePath -CommandLine $action.Execute } else { $null }
        $nextRun = $null
        if ($task.State -ne 'Disabled') {
            try {
                $info = Get-ScheduledTaskInfo -TaskPath $task.TaskPath -TaskName $task.TaskName -ErrorAction Stop
                if ($info.NextRunTime -and $info.NextRunTime.Year -gt 2000) { $nextRun = $info.NextRunTime }
            } catch {
                Write-Verbose "Could not read run info for $fullName : $($_.Exception.Message)"
            }
        }
        $state = [string]$task.State
        if ($nextRun) { $state = "$state, next $($nextRun.ToString('ddd HH:mm'))" }
        New-EsItem -Type 'task' -Name $fullName -Path $path -State $state -Active ($task.State -ne 'Disabled') -IsSystem $false -Extra @{
            TaskPath    = $task.TaskPath
            TaskName    = $task.TaskName
            NextRunTime = $nextRun
        }
    }
}

function Get-EsStartupSources {
    # Each Run location has a matching "StartupApproved" key - that is where Task Manager's
    # Startup tab stores enabled/disabled, and it is what ExamSafe toggles.
    $approvedRoot = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved'
    @(
        @{ Hive = 'HKCU'; RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run'; ApprovedKey = "$approvedRoot\Run" },
        @{ Hive = 'HKLM'; RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run'; ApprovedKey = "$approvedRoot\Run" },
        @{ Hive = 'HKLM'; RunKey = 'Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run'; ApprovedKey = "$approvedRoot\Run32" },
        @{ Hive = 'HKCU'; Folder = [Environment]::GetFolderPath('Startup'); ApprovedKey = "$approvedRoot\StartupFolder" },
        @{ Hive = 'HKLM'; Folder = [Environment]::GetFolderPath('CommonStartup'); ApprovedKey = "$approvedRoot\StartupFolder" }
    )
}

function Get-EsRegistryBaseKey {
    param([string] $Hive)
    if ($Hive -eq 'HKLM') { return [Microsoft.Win32.Registry]::LocalMachine }
    return [Microsoft.Win32.Registry]::CurrentUser
}

function Get-EsApprovedBytes {
    param([string] $Hive, [string] $ApprovedKey, [string] $ValueName)
    $key = (Get-EsRegistryBaseKey -Hive $Hive).OpenSubKey($ApprovedKey)
    if (-not $key) { return $null }
    try { return $key.GetValue($ValueName) } finally { $key.Close() }
}

function Test-EsStartupEnabled {
    # Missing value = enabled. Otherwise the first byte is 0x02/0x06 for enabled and 0x03/0x07
    # for disabled, i.e. the low bit means "disabled".
    param([byte[]] $Bytes)
    if (-not $Bytes -or $Bytes.Length -eq 0) { return $true }
    return (($Bytes[0] -band 1) -eq 0)
}

function Get-EsStartupItems {
    foreach ($source in Get-EsStartupSources) {
        $entries = @()
        if ($source.RunKey) {
            $key = (Get-EsRegistryBaseKey -Hive $source.Hive).OpenSubKey($source.RunKey)
            if ($key) {
                try {
                    foreach ($valueName in $key.GetValueNames()) {
                        if ($valueName) { $entries += @{ Name = $valueName; Command = [string]$key.GetValue($valueName) } }
                    }
                } finally { $key.Close() }
            }
        } elseif ($source.Folder -and (Test-Path -LiteralPath $source.Folder)) {
            foreach ($file in Get-ChildItem -LiteralPath $source.Folder -File | Where-Object { $_.Name -ne 'desktop.ini' }) {
                $entries += @{ Name = $file.Name; Command = $file.FullName }
            }
        }
        foreach ($entry in $entries) {
            $bytes = Get-EsApprovedBytes -Hive $source.Hive -ApprovedKey $source.ApprovedKey -ValueName $entry.Name
            $enabled = Test-EsStartupEnabled -Bytes $bytes
            $path = Get-EsExecutablePath -CommandLine $entry.Command
            $item = New-EsItem -Type 'startup' -Name $entry.Name -Path $path -State $(if ($enabled) { 'Enabled' } else { 'Disabled' }) `
                -Active $enabled -IsSystem (Test-EsSystemPath -Path $path) -Extra @{
                    ApprovedKey   = "$($source.Hive)\$($source.ApprovedKey)"
                    ApprovedValue = $entry.Name
                }
            # The same name can exist in HKCU and HKLM - keep keys unique.
            $item.Key = "startup:$($source.Hive):$($source.ApprovedKey.Split('\')[-1]):$($entry.Name)"
            $item
        }
    }
}

function Get-EsPortItems {
    $listeners = @(Get-NetTCPConnection -State Listen -ErrorAction Stop)
    $names = @{}
    foreach ($proc in Get-Process -ErrorAction SilentlyContinue) { $names[[int]$proc.Id] = $proc.Name }
    foreach ($group in ($listeners | Group-Object LocalPort)) {
        $first = $group.Group[0]
        $ownerPid = [int]$first.OwningProcess
        $ownerName = if ($names.ContainsKey($ownerPid)) { $names[$ownerPid] } else { "pid $ownerPid" }
        $bindsAll = [bool]($group.Group | Where-Object { $_.LocalAddress -in @('0.0.0.0', '::') -or ($_.LocalAddress -notlike '127.*' -and $_.LocalAddress -ne '::1') })
        $scope = if ($bindsAll) { 'network' } else { 'local only' }
        New-EsItem -Type 'port' -Name ([string]$group.Name) -DisplayName "TCP $($group.Name) ($ownerName)" -State "Listening, $scope" `
            -Active $true -IsSystem ($ownerPid -le 4 -or $ownerName -in @('svchost', 'System', 'lsass', 'wininit', 'services', 'spoolsv')) -Extra @{
                Port      = [int]$group.Name
                OwnerPid  = $ownerPid
                OwnerName = $ownerName
                BindsAllInterfaces = $bindsAll
            }
    }
}

function Get-EsInventory {
    # Each source is isolated so one failing API (e.g. no permission) does not hide the rest.
    $items = New-Object System.Collections.Generic.List[object]
    $errors = New-Object System.Collections.Generic.List[string]
    $sources = [ordered]@{
        processes = { Get-EsProcessItems }
        services  = { Get-EsServiceItems }
        tasks     = { Get-EsTaskItems }
        startup   = { Get-EsStartupItems }
        ports     = { Get-EsPortItems }
    }
    foreach ($name in $sources.Keys) {
        try {
            foreach ($item in & $sources[$name]) { if ($item) { $items.Add($item) } }
        } catch {
            $errors.Add("Could not scan $name`: $($_.Exception.Message)")
        }
    }
    [pscustomobject]@{ Items = $items.ToArray(); Errors = $errors.ToArray(); ScannedAt = Get-Date }
}
