# Screen Time Manager - deploy a locally built binary
# Run this script; it self-elevates (UAC), replaces the installed executable,
# refreshes the watchdog and (re)registers both scheduled tasks.

param(
    [string]$ExePath = "",
    [string]$InstallDir = "C:\Program Files\ScreenTimeManager"
)

$ErrorActionPreference = "Stop"

# --- self-elevate -----------------------------------------------------------
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "Requesting administrator privileges..." -ForegroundColor Yellow
    $argList = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", "`"$PSCommandPath`""
    )
    if ($ExePath) { $argList += @("-ExePath", "`"$ExePath`"") }
    if ($InstallDir) { $argList += @("-InstallDir", "`"$InstallDir`"") }
    Start-Process powershell.exe -Verb RunAs -ArgumentList $argList
    exit
}

if (-not $ExePath) {
    $distExe = Join-Path $PSScriptRoot "dist\screen-time-manager.exe"
    if (Test-Path $distExe) {
        $ExePath = $distExe
    } else {
        $ExePath = Join-Path $PSScriptRoot "target\release\screen-time-manager.exe"
        if (-not (Test-Path $ExePath)) {
            $ExePath = Join-Path $env:LOCALAPPDATA "stm-build-target\release\screen-time-manager.exe"
        }
    }
}
$ExePath = (Resolve-Path $ExePath).Path

$TaskName = "ScreenTimeManager"
$WatchdogTaskName = "ScreenTimeManagerWatchdog"
$WatchdogSource = Join-Path $PSScriptRoot "watchdog.ps1"
$TargetExe = Join-Path $InstallDir "screen-time-manager.exe"
$TargetWatchdog = Join-Path $InstallDir "watchdog.ps1"

Write-Host "Screen Time Manager - deploy" -ForegroundColor Cyan
Write-Host "Source exe: $ExePath" -ForegroundColor White
Write-Host "Install dir: $InstallDir" -ForegroundColor White
Write-Host ""

# --- stop running instance --------------------------------------------------
Write-Host "Stopping running instance..." -ForegroundColor White
Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
Stop-ScheduledTask -TaskName $WatchdogTaskName -ErrorAction SilentlyContinue
Get-Process -Name "screen-time-manager" -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 700

# --- copy files -------------------------------------------------------------
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

Copy-Item $ExePath $TargetExe -Force
Write-Host "Copied executable." -ForegroundColor Green

if (Test-Path $WatchdogSource) {
    Copy-Item $WatchdogSource $TargetWatchdog -Force
    Write-Host "Copied watchdog script." -ForegroundColor Green
} else {
    Write-Host "WARNING: watchdog.ps1 not found next to deploy.ps1" -ForegroundColor Yellow
}

# Keep the helper scripts in the install directory in sync as well.
foreach ($helper in @("watchdog.ps1", "install.ps1", "uninstall.ps1", "update.ps1")) {
    $helperSource = Join-Path $PSScriptRoot $helper
    if ((Test-Path $helperSource) -and ([IO.Path]::GetFullPath($helperSource) -ne [IO.Path]::GetFullPath((Join-Path $InstallDir $helper)))) {
        Copy-Item $helperSource (Join-Path $InstallDir $helper) -Force
    }
}
Write-Host "Copied helper scripts." -ForegroundColor Green

# --- remove duplicate Startup shortcut --------------------------------------
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
        } catch { }
    }
}

# --- scheduled tasks --------------------------------------------------------
Write-Host "Registering scheduled task..." -ForegroundColor White
$action = New-ScheduledTaskAction -Execute $TargetExe
$trigger = New-ScheduledTaskTrigger -AtLogOn
$trigger.Delay = "PT20S"
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -ExecutionTimeLimit (New-TimeSpan -Days 0) `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1)

$task = New-ScheduledTask -Action $action -Trigger $trigger -Principal $principal -Settings $settings `
    -Description "Screen Time Manager - Manages daily computer time limits"
Register-ScheduledTask -TaskName $TaskName -InputObject $task -Force | Out-Null

if (Test-Path $TargetWatchdog) {
    Write-Host "Registering watchdog task..." -ForegroundColor White
    $watchdogAction = New-ScheduledTaskAction `
        -Execute "powershell.exe" `
        -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$TargetWatchdog`""
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
}

# --- start ------------------------------------------------------------------
$stopMarker = Join-Path $env:LOCALAPPDATA ".screen-time-manager\stopped"
Remove-Item $stopMarker -Force -ErrorAction SilentlyContinue

Write-Host "Starting Screen Time Manager..." -ForegroundColor White
Start-ScheduledTask -TaskName $TaskName
Start-Sleep -Seconds 2

$proc = Get-Process -Name "screen-time-manager" -ErrorAction SilentlyContinue
if ($proc) {
    Write-Host "Running (PID $($proc.Id))." -ForegroundColor Green
} else {
    Write-Host "WARNING: process is not running yet; check the log below." -ForegroundColor Yellow
}

$logPath = Join-Path $env:LOCALAPPDATA ".screen-time-manager\app.log"
if (Test-Path $logPath) {
    Write-Host ""
    Write-Host "Last log lines:" -ForegroundColor Cyan
    Get-Content $logPath -Tail 15
}

Write-Host ""
Write-Host "Deploy complete." -ForegroundColor Green
