# Screen Time Manager - watchdog
# Runs from Task Scheduler once a minute. If the app is not running (crash,
# killed via Task Manager, etc.) it starts it again.
#
# An intentional quit from the tray menu (passcode protected) creates the
# "stopped" marker, which tells this script to stay out of the way until the
# app is launched again.

$ErrorActionPreference = "SilentlyContinue"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $root "screen-time-manager.exe"
$dataDir = Join-Path $env:LOCALAPPDATA ".screen-time-manager"
$stopMarker = Join-Path $dataDir "stopped"
$lastStartFile = Join-Path $dataDir "watchdog-last-start"
$watchdogLog = Join-Path $dataDir "watchdog.log"

function Write-WatchdogLog([string]$message) {
    if ((Test-Path $watchdogLog) -and ((Get-Item $watchdogLog).Length -gt 262144)) {
        Clear-Content -Path $watchdogLog
    }
    $ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    Add-Content -Path $watchdogLog -Value "[$ts] $message"
}

# Nothing to watch.
if (-not (Test-Path $exe)) { exit 0 }

# Parent quit the app on purpose - do not fight that decision.
if (Test-Path $stopMarker) { exit 0 }

$running = Get-Process -Name "screen-time-manager" -ErrorAction SilentlyContinue
if ($running) { exit 0 }

# Guard against a restart loop: at most one start attempt every 30 seconds.
if (Test-Path $lastStartFile) {
    $lastStart = (Get-Item $lastStartFile).LastWriteTime
    if (((Get-Date) - $lastStart).TotalSeconds -lt 30) { exit 0 }
}

New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
Set-Content -Path $lastStartFile -Value (Get-Date -Format o)

Write-WatchdogLog "App not running - starting $exe"
Start-Process -FilePath $exe -WindowStyle Hidden
