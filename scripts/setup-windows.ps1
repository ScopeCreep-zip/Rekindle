# Rekindle — Windows developer setup
# Usage: powershell -ExecutionPolicy Bypass -File scripts\setup-windows.ps1
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info($msg)  { Write-Host "[+] $msg" -ForegroundColor Green }
function Warn($msg)  { Write-Host "[!] $msg" -ForegroundColor Yellow }
function Error($msg) { Write-Host "[x] $msg" -ForegroundColor Red; exit 1 }

# ── Visual Studio Build Tools ───────────────────────────────────────
$vsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasCpp = $false
if (Test-Path $vsWhere) {
    $result = & $vsWhere -latest -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    if ($result) { $hasCpp = $true }
}

if ($hasCpp) {
    Info "Visual Studio C++ Build Tools found"
} else {
    Error @"
Visual Studio Build Tools with C++ workload not found.

Install from: https://visualstudio.microsoft.com/visual-cpp-build-tools/
Select "Desktop development with C++" workload during installation.
Then re-run this script.
"@
}

# ── winget availability ─────────────────────────────────────────────
$hasWinget = [bool](Get-Command winget -ErrorAction SilentlyContinue)
$hasChoco  = [bool](Get-Command choco -ErrorAction SilentlyContinue)

if (-not $hasWinget -and -not $hasChoco) {
    Error "Neither winget nor chocolatey found. Install winget (built into Windows 11) or chocolatey first."
}

# ── Helper: install a package ───────────────────────────────────────
function Install-Pkg {
    param(
        [string]$Name,
        [string]$WingetId,
        [string]$ChocoName,
        [string]$TestCommand
    )

    if ($TestCommand -and (Get-Command $TestCommand -ErrorAction SilentlyContinue)) {
        Info "$Name already installed"
        return
    }

    if ($hasWinget -and $WingetId) {
        Warn "Installing $Name via winget..."
        winget install --id $WingetId --accept-source-agreements --accept-package-agreements --silent
    } elseif ($hasChoco -and $ChocoName) {
        Warn "Installing $Name via chocolatey..."
        choco install $ChocoName -y
    } else {
        Warn "Cannot install $Name automatically — install it manually"
    }
}

# ── CMake ────────────────────────────────────────────────────────────
Install-Pkg -Name "CMake" -WingetId "Kitware.CMake" -ChocoName "cmake" -TestCommand "cmake"

# ── Cap'n Proto ──────────────────────────────────────────────────────
Install-Pkg -Name "Cap'n Proto" -WingetId "" -ChocoName "capnproto" -TestCommand "capnp"
# winget doesn't have capnproto — fall back to choco
if (-not (Get-Command capnp -ErrorAction SilentlyContinue)) {
    if ($hasChoco) {
        Warn "Installing Cap'n Proto via chocolatey..."
        choco install capnproto -y
    } else {
        Warn "Cap'n Proto not found — install manually from https://capnproto.org/install.html"
    }
}

# ── Rust ─────────────────────────────────────────────────────────────
if (Get-Command rustup -ErrorAction SilentlyContinue) {
    $rustVer = rustc --version
    Info "Rust already installed ($rustVer)"
    rustup update stable 2>$null
} else {
    Warn "Installing Rust via rustup..."
    $rustupInit = "$env:TEMP\rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupInit
    & $rustupInit -y --default-toolchain stable --default-host x86_64-pc-windows-msvc
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    Info "Rust installed ($(rustc --version))"
}

# ── Node.js ──────────────────────────────────────────────────────────
if (Get-Command node -ErrorAction SilentlyContinue) {
    $nodeVer = node --version
    Info "Node.js already installed ($nodeVer)"
} else {
    Install-Pkg -Name "Node.js 22 LTS" -WingetId "OpenJS.NodeJS.LTS" -ChocoName "nodejs-lts" -TestCommand "node"
}

# ── pnpm via corepack ───────────────────────────────────────────────
if (Get-Command corepack -ErrorAction SilentlyContinue) {
    corepack enable 2>$null
    Info "corepack enabled"
} else {
    Warn "corepack not found — install Node.js 22+ first"
}

if (Get-Command pnpm -ErrorAction SilentlyContinue) {
    Info "pnpm available ($(pnpm --version))"
} else {
    Warn "pnpm not found after corepack enable — try: npm install -g pnpm"
}

# ── Install frontend dependencies ───────────────────────────────────
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir

if (Test-Path "$ProjectDir\node_modules") {
    Info "node_modules exists — run 'pnpm install' manually if needed"
} else {
    Warn "Running pnpm install..."
    Push-Location $ProjectDir
    pnpm install
    Pop-Location
    Info "Frontend dependencies installed"
}

# ── Summary ──────────────────────────────────────────────────────────
Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host "  Rekindle Windows setup complete!" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
Write-Host ""
Write-Host "  Note: opus-sys bundles libopus on Windows (no manual install needed)"
Write-Host "  Note: WebView2 is pre-installed on Windows 10+     "
Write-Host ""
Write-Host "  Next steps:"
Write-Host "    cd $((Get-Item $ProjectDir).Name)"
Write-Host "    pnpm tauri dev      # Start development"
Write-Host "    pnpm tauri build    # Build for distribution"
Write-Host ""
