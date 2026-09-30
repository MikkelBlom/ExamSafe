<#
.SYNOPSIS
    Measures ExamSafe's startup time, memory and idle CPU, with the window open and tray-only.

.EXAMPLE
    cargo build --release; ./tools/measure.ps1
#>
param(
    [string] $Exe = (Join-Path $PSScriptRoot '..\target\release\examsafe.exe'),
    [int] $IdleSeconds = 20
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path $Exe)) { throw "Build first: cargo build --release ($Exe not found)" }

function Measure-Idle([System.Diagnostics.Process] $Process, [string] $Label) {
    $Process.Refresh()
    $cpuStart = $Process.TotalProcessorTime
    $clock = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $IdleSeconds
    $Process.Refresh()
    $cpuPercent = ($Process.TotalProcessorTime - $cpuStart).TotalMilliseconds / $clock.Elapsed.TotalMilliseconds / [Environment]::ProcessorCount * 100
    $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($Process.Id)").Count
    [pscustomobject]@{
        State           = $Label
        WorkingSetMB    = [math]::Round($Process.WorkingSet64 / 1MB, 1)
        PrivateMB       = [math]::Round($Process.PrivateMemorySize64 / 1MB, 1)
        PeakWorkingSetMB = [math]::Round($Process.PeakWorkingSet64 / 1MB, 1)
        IdleCpuPercent  = [math]::Round($cpuPercent, 3)
        Threads         = $Process.Threads.Count
        ChildProcesses  = $children
    }
}

$clock = [Diagnostics.Stopwatch]::StartNew()
$process = Start-Process -FilePath $Exe -PassThru
while ($process.MainWindowHandle -eq 0 -and $clock.Elapsed.TotalSeconds -lt 20) {
    Start-Sleep -Milliseconds 10
    $process.Refresh()
}
$startupMs = $clock.ElapsedMilliseconds
Start-Sleep -Seconds 2

try {
    $open = Measure-Idle $process 'Window open (idle)'
    # WM_CLOSE only hides the window; the app keeps running in the tray.
    $null = $process.CloseMainWindow()
    Start-Sleep -Seconds 2
    $tray = Measure-Idle $process 'Tray only'
    "Startup to window: $startupMs ms"
    @($open, $tray) | Format-Table -AutoSize
} finally {
    Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
}
