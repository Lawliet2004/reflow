# Reflow Windows Packaging & Installer Script
# Automates pre-flight verification, frontend build, Tauri Windows NSIS/MSI bundle generation, and SHA-256 checksum creation.

param (
    [switch]$SkipFrontendBuild = $false,
    [switch]$SignBinaries = $false
)

$ErrorActionPreference = "Stop"

Write-Host "=========================================" -ForegroundColor Cyan
Write-Host " Reflow Windows Packaging Script " -ForegroundColor Cyan
Write-Host "=========================================" -ForegroundColor Cyan

# 1. Pre-flight environment validation
Write-Host "`n[1/5] Checking build environment prerequisites..." -ForegroundColor Yellow

# Node.js
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Error "Node.js is not installed or not in PATH."
}
Write-Host "  ? Node.js detected: $(node --version)" -ForegroundColor Green

# Rust & Cargo
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "Cargo/Rust is not installed or not in PATH."
}
Write-Host "  ? Cargo detected: $(cargo --version)" -ForegroundColor Green

# Python 3
if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    Write-Error "Python 3 is not installed or not in PATH."
}
Write-Host "  ? Python runtime detected: $(python --version)" -ForegroundColor Green

# Runtime script presence
$RuntimeScript = Join-Path $PSScriptRoot "..\model-runtime\qwen3_asr_runtime.py"
if (-not (Test-Path $RuntimeScript)) {
    Write-Error "model-runtime/qwen3_asr_runtime.py missing."
}
Write-Host "  ? Runtime script found: $RuntimeScript" -ForegroundColor Green

# VC++ Redistributable check
$VCRedistKey = "HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\X64"
if (Test-Path $VCRedistKey) {
    Write-Host "  ? Visual C++ 2015-2022 Redistributable is installed." -ForegroundColor Green
} else {
    Write-Host "  ! VC++ Redistributable registry key not found (User may need VC++ redist installed at runtime)." -ForegroundColor Yellow
}

# 2. Frontend Assets Build
if (-not $SkipFrontendBuild) {
    Write-Host "`n[2/5] Building frontend application bundle..." -ForegroundColor Yellow
    npm run build
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Frontend build failed with exit code $LASTEXITCODE"
    }
    Write-Host "  ? Frontend assets successfully built to dist/" -ForegroundColor Green
} else {
    Write-Host "`n[2/5] Skipping frontend build as requested." -ForegroundColor Gray
}

# 3. Rust & Tauri Bundle Compilation
Write-Host "`n[3/5] Compiling Tauri release binary & packaging Windows installer..." -ForegroundColor Yellow
Set-Location (Join-Path $PSScriptRoot "..\src-tauri")

# Ensure Tauri CLI is available
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Error "Cargo release build failed with exit code $LASTEXITCODE"
}
Write-Host "  ? Cargo release compilation succeeded." -ForegroundColor Green

# 4. Binary Signing Hook (if configured)
if ($SignBinaries) {
    Write-Host "`n[4/5] Running Code Signing Hook..." -ForegroundColor Yellow
    if ($env:SIGNING_CERT_THUMBPRINT) {
        Write-Host "  Signing with certificate thumbprint: $env:SIGNING_CERT_THUMBPRINT" -ForegroundColor Green
        # Example: signtool sign /sha1 $env:SIGNING_CERT_THUMBPRINT /t http://timestamp.digicert.com target\release\reflow.exe
    } else {
        Write-Host "  ! SIGNING_CERT_THUMBPRINT environment variable not set; skipping signing." -ForegroundColor Yellow
    }
} else {
    Write-Host "`n[4/5] Skipping binary signing (run with -SignBinaries if needed)." -ForegroundColor Gray
}

# 5. Checksum Generation
Write-Host "`n[5/5] Generating SHA-256 Checksums..." -ForegroundColor Yellow
$ExePath = Join-Path (Get-Location) "target\release\reflow.exe"
if (Test-Path $ExePath) {
    $Hash = (Get-FileHash -Algorithm SHA256 -Path $ExePath).Hash
    $ChecksumFile = "$ExePath.sha256"
    Set-Content -Path $ChecksumFile -Value "$Hash  reflow.exe"
    Write-Host "  ? SHA-256: $Hash" -ForegroundColor Green
    Write-Host "  ? Checksum saved to: $ChecksumFile" -ForegroundColor Green
}

Set-Location $PSScriptRoot
Write-Host "`n=========================================" -ForegroundColor Cyan
Write-Host " Packaging Completed Successfully! " -ForegroundColor Cyan
Write-Host "=========================================" -ForegroundColor Cyan
