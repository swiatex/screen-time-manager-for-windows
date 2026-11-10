# Screen Time Manager - Update Script
# Updates the executable and restarts the service

param(
    [string]$NewExePath = "",
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$TaskName = "ScreenTimeManager"
$WatchdogTaskName = "ScreenTimeManagerWatchdog"
$DataDir = Join-Path $env:LOCALAPPDATA ".screen-time-manager"
$StopMarker = Join-Path $DataDir "stopped"

Write-Host "Screen Time Manager - Update" -ForegroundColor Cyan
Write-Host "============================" -ForegroundColor Cyan
Write-Host ""

# Check if task exists to get current exe path
$existingTask = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
if (-not $existingTask) {
    Write-Host "ERROR: Screen Time Manager is not installed." -ForegroundColor Red
    Write-Host "Please run install.ps1 first." -ForegroundColor Yellow
    Write-Host ""
    exit 1
}

# Get current executable path from the scheduled task
$taskInfo = Get-ScheduledTask -TaskName $TaskName | Get-ScheduledTaskInfo
$currentExePath = (Get-ScheduledTask -TaskName $TaskName).Actions[0].Execute

Write-Host "Current installation: $currentExePath" -ForegroundColor White
Write-Host ""

# Find new executable
if ($NewExePath -eq "") {
    # Try to find it in the same directory as this script
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $possiblePaths = @(
        (Join-Path $scriptDir "screen-time-manager.exe")
    )

    foreach ($path in $possiblePaths) {
        if (Test-Path $path) {
            $NewExePath = $path
            break
        }
    }
}

if ($NewExePath -eq "" -or -not (Test-Path $NewExePath)) {
    Write-Host "ERROR: Could not find new screen-time-manager.exe" -ForegroundColor Red
    Write-Host ""
    Write-Host "Usage: .\update.ps1 -NewExePath 'C:\path\to\new\screen-time-manager.exe'" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Or place the new screen-time-manager.exe in the same folder as this script." -ForegroundColor Yellow
    Write-Host ""
    exit 1
}

$NewExePath = (Resolve-Path $NewExePath).Path
Write-Host "New executable: $NewExePath" -ForegroundColor Green
Write-Host ""

# Never silently downgrade: the fix in 1.0.41 must not be replaced by an older
# upstream release downloaded by this script.
if (-not $Force -and (Test-Path $currentExePath)) {
    $installedVersion = (Get-Item $currentExePath).VersionInfo.FileVersion
    $newVersion = (Get-Item $NewExePath).VersionInfo.FileVersion
    if ($installedVersion -and $newVersion) {
        try {
            if ([version]$newVersion -le [version]$installedVersion) {
                Write-Host "Refusing to install $newVersion over installed $installedVersion." -ForegroundColor Red
                Write-Host "This would downgrade the app (and reintroduce the logon crash)." -ForegroundColor Yellow
                Write-Host "Use -Force if you really want to downgrade." -ForegroundColor Yellow
                exit 1
            }
        } catch {
            # Unparsable version metadata - continue.
        }
    }
}

# Check if updating in place (same path)
$updateInPlace = $currentExePath -eq $NewExePath
if ($updateInPlace) {
    Write-Host "Note: Updating in place (same location)" -ForegroundColor Yellow
    Write-Host ""
}

# Stop the watchdog first, then tell it (via the marker) not to resurrect the
# app while we replace the executable.
Write-Host "Stopping Screen Time Manager..." -ForegroundColor White
Stop-ScheduledTask -TaskName $WatchdogTaskName -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
Set-Content -Path $StopMarker -Value (Get-Date -Format o)
Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue

# Kill the process if running
$process = Get-Process -Name "screen-time-manager" -ErrorAction SilentlyContinue
if ($process) {
    Write-Host "Terminating running process..." -ForegroundColor White
    Stop-Process -Name "screen-time-manager" -Force -ErrorAction SilentlyContinue
    # Wait for process to fully terminate
    Start-Sleep -Seconds 2
}

# Double-check process is gone
$retries = 5
while ($retries -gt 0) {
    $process = Get-Process -Name "screen-time-manager" -ErrorAction SilentlyContinue
    if (-not $process) {
        break
    }
    Write-Host "Waiting for process to terminate..." -ForegroundColor Yellow
    Start-Sleep -Seconds 1
    $retries--
}

if ($process) {
    Write-Host "ERROR: Could not terminate the running process." -ForegroundColor Red
    Write-Host "Please close Screen Time Manager manually and try again." -ForegroundColor Yellow
    exit 1
}

# Copy new executable if different location
if (-not $updateInPlace) {
    Write-Host "Copying new executable to: $currentExePath" -ForegroundColor White

    # Backup old executable
    $backupPath = "$currentExePath.backup"
    if (Test-Path $currentExePath) {
        Copy-Item -Path $currentExePath -Destination $backupPath -Force
        Write-Host "Backed up old version to: $backupPath" -ForegroundColor Gray
    }

    # Copy new executable
    try {
        Copy-Item -Path $NewExePath -Destination $currentExePath -Force
    } catch {
        Write-Host "ERROR: Failed to copy new executable." -ForegroundColor Red
        Write-Host $_.Exception.Message -ForegroundColor Red

        # Restore backup
        if (Test-Path $backupPath) {
            Write-Host "Restoring backup..." -ForegroundColor Yellow
            Copy-Item -Path $backupPath -Destination $currentExePath -Force
        }

        # The old version should keep running: clear the marker and let the
        # watchdog bring it back.
        Remove-Item $StopMarker -Force -ErrorAction SilentlyContinue
        Start-ScheduledTask -TaskName $WatchdogTaskName -ErrorAction SilentlyContinue
        exit 1
    }
}

Write-Host ""
Write-Host "Update complete!" -ForegroundColor Green
Write-Host ""

# Start the service again
$response = Read-Host "Do you want to start Screen Time Manager now? (Y/n)"
if ($response -eq "" -or $response -match "^[Yy]") {
    Write-Host "Starting Screen Time Manager..." -ForegroundColor White
    Remove-Item $StopMarker -Force -ErrorAction SilentlyContinue
    Start-ScheduledTask -TaskName $TaskName
    Write-Host "Started!" -ForegroundColor Green
}

Write-Host ""
