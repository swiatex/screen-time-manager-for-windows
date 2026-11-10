# Screen Time Manager - Install Script
# Run as Administrator for best results

param(
    [string]$ExePath = ""
)

$ErrorActionPreference = "Stop"
$TaskName = "ScreenTimeManager"
$WatchdogTaskName = "ScreenTimeManagerWatchdog"
$Description = "Screen Time Manager - Manages daily computer time limits"

Write-Host "Screen Time Manager - Installation" -ForegroundColor Cyan
Write-Host "===================================" -ForegroundColor Cyan
Write-Host ""

# Find the executable
if ($ExePath -eq "") {
    # Try to find it in common locations
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $possiblePaths = @(
        (Join-Path $scriptDir "screen-time-manager.exe"),
        (Join-Path $scriptDir "target\release\screen-time-manager.exe"),
        (Join-Path $scriptDir "target\debug\screen-time-manager.exe")
    )

    foreach ($path in $possiblePaths) {
        if (Test-Path $path) {
            $ExePath = $path
            break
        }
    }
}

if ($ExePath -eq "" -or -not (Test-Path $ExePath)) {
    Write-Host "ERROR: Could not find screen-time-manager.exe" -ForegroundColor Red
    Write-Host ""
    Write-Host "Usage: .\install.ps1 -ExePath 'C:\path\to\screen-time-manager.exe'" -ForegroundColor Yellow
    Write-Host ""
    exit 1
}

$ExePath = (Resolve-Path $ExePath).Path
$InstallDir = Split-Path -Parent $ExePath
Write-Host "Found executable: $ExePath" -ForegroundColor Green
Write-Host ""

# Make sure the watchdog script sits next to the executable.
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$sourceWatchdog = Join-Path $scriptDir "watchdog.ps1"
$watchdogPath = Join-Path $InstallDir "watchdog.ps1"

if (Test-Path $sourceWatchdog) {
    if ([IO.Path]::GetFullPath($sourceWatchdog) -ne [IO.Path]::GetFullPath($watchdogPath)) {
        Copy-Item $sourceWatchdog $watchdogPath -Force
    }
}

if (-not (Test-Path $watchdogPath)) {
    Write-Host "WARNING: watchdog.ps1 not found - crash auto-restart will be unavailable." -ForegroundColor Yellow
}

# Remove duplicate autostart entries (a Startup shortcut would launch a second
# copy at logon, which is exactly when the app must start reliably).
$startupDir = [Environment]::GetFolderPath('Startup')
if ($startupDir -and (Test-Path $startupDir)) {
    Get-ChildItem $startupDir -Filter *.lnk -ErrorAction SilentlyContinue | ForEach-Object {
        try {
            $shell = New-Object -ComObject WScript.Shell
            $target = $shell.CreateShortcut($_.FullName).TargetPath
            if ([IO.Path]::GetFileName($target) -ieq "screen-time-manager.exe") {
                Remove-Item $_.FullName -Force
                Write-Host "Removed duplicate Startup shortcut: $($_.Name)" -ForegroundColor Yellow
            }
        } catch {
            # Ignore shortcuts we cannot inspect.
        }
    }
}

# Create the main scheduled task
Write-Host "Creating scheduled task..." -ForegroundColor White

$action = New-ScheduledTaskAction -Execute $ExePath
$trigger = New-ScheduledTaskTrigger -AtLogOn
# Give Explorer a moment to create the notification area. The app also retries
# on its own, but this avoids the race in the first place.
$trigger.Delay = "PT20S"
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -ExecutionTimeLimit (New-TimeSpan -Days 0) `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1)

$task = New-ScheduledTask -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description $Description
Register-ScheduledTask -TaskName $TaskName -InputObject $task -Force | Out-Null

Write-Host "Main task created." -ForegroundColor Green

# Create the watchdog task (checks once a minute whether the app is alive)
if (Test-Path $watchdogPath) {
    Write-Host "Creating watchdog task..." -ForegroundColor White

    $watchdogAction = New-ScheduledTaskAction `
        -Execute "powershell.exe" `
        -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$watchdogPath`""

    # Repeat every minute, starting shortly from now, indefinitely.
    $watchdogTrigger = New-ScheduledTaskTrigger -Once -At ((Get-Date).AddMinutes(1)) `
        -RepetitionInterval (New-TimeSpan -Minutes 1) `
        -RepetitionDuration (New-TimeSpan -Days 3650)
    $watchdogLogonTrigger = New-ScheduledTaskTrigger -AtLogOn
    $watchdogLogonTrigger.Delay = "PT30S"

    $watchdogSettings = New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -StartWhenAvailable `
        -MultipleInstances IgnoreNew `
        -ExecutionTimeLimit (New-TimeSpan -Minutes 2)

    $watchdogTask = New-ScheduledTask -Action $watchdogAction -Trigger @($watchdogTrigger, $watchdogLogonTrigger) `
        -Principal $principal -Settings $watchdogSettings `
        -Description "Restarts Screen Time Manager if it is not running"

    Register-ScheduledTask -TaskName $WatchdogTaskName -InputObject $watchdogTask -Force | Out-Null
    Write-Host "Watchdog task created." -ForegroundColor Green
}

Write-Host ""
Write-Host "Installation complete!" -ForegroundColor Green
Write-Host ""
Write-Host "Screen Time Manager will now start automatically when you log in." -ForegroundColor White
Write-Host "If it ever crashes or is killed, the watchdog restarts it within a minute." -ForegroundColor White
Write-Host ""

# Ask if user wants to start it now
$response = Read-Host "Do you want to start Screen Time Manager now? (Y/n)"
if ($response -eq "" -or $response -match "^[Yy]") {
    Write-Host "Starting Screen Time Manager..." -ForegroundColor White
    Start-ScheduledTask -TaskName $TaskName
    Write-Host "Started!" -ForegroundColor Green
}

Write-Host ""
Write-Host "To uninstall, run: .\uninstall.ps1" -ForegroundColor Cyan
Write-Host ""
