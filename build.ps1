# Build Screen Time Manager
# Usage:  powershell -ExecutionPolicy Bypass -File .\build.ps1

$ErrorActionPreference = "Stop"

# Make a per-user Rust installation visible even if PATH was not refreshed.
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) {
    $env:Path = "$cargoBin;$env:Path"
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "cargo not found. Install Rust from https://rustup.rs/ first." -ForegroundColor Red
    exit 1
}

# The GNU toolchain needs a real MinGW gcc for crates with C code (SQLite).
$hostLine = (& rustc -vV | Select-String '^host:').ToString()
if ($hostLine -match 'windows-gnu') {
    $mingwCandidates = @(
        (Join-Path $env:USERPROFILE "tools\msvcrt\mingw64\bin"),
        (Join-Path $env:USERPROFILE "tools\mingw64\bin"),
        "C:\msys64\mingw64\bin"
    )
    $mingwBin = $mingwCandidates |
        Where-Object { Test-Path (Join-Path $_ "gcc.exe") } |
        Select-Object -First 1

    if (-not $mingwBin) {
        Write-Host "GNU toolchain detected, but MinGW gcc was not found." -ForegroundColor Red
        Write-Host "Install WinLibs/MinGW-w64 or use the MSVC Rust toolchain." -ForegroundColor Yellow
        exit 1
    }

    $env:Path = "$mingwBin;$env:Path"
    $env:CC = "gcc"
    $env:CXX = "g++"
    $env:AR = "ar"
}

# GNU ld on Windows receives file names in the ANSI code page. If the project
# lives in a path with non-ASCII characters (e.g. a Polish "l"), linking fails
# with "cannot find ...rlib". Build in an ASCII target directory instead.
$targetDir = Join-Path $PSScriptRoot "target"
if ($PSScriptRoot -match '[^\x00-\x7F]') {
    $targetDir = Join-Path $env:LOCALAPPDATA "stm-build-target"
}
$env:CARGO_TARGET_DIR = $targetDir

Set-Location $PSScriptRoot
cargo build --release

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed." -ForegroundColor Red
    exit $LASTEXITCODE
}

# Keep a copy in an easy-to-find place for deploy.ps1.
$distDir = Join-Path $PSScriptRoot "dist"
New-Item -ItemType Directory -Force -Path $distDir | Out-Null
$builtExe = Join-Path $targetDir "release\screen-time-manager.exe"
Copy-Item $builtExe (Join-Path $distDir "screen-time-manager.exe") -Force

Write-Host ""
Write-Host "Build OK: $builtExe" -ForegroundColor Green
Write-Host "Copied to: $distDir\screen-time-manager.exe" -ForegroundColor Green
Write-Host "Install/update with: powershell -ExecutionPolicy Bypass -File .\deploy.ps1" -ForegroundColor Cyan
