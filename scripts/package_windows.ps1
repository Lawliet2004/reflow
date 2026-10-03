# Build verified Tauri NSIS/MSI installers from this checkout. The caller's cwd is preserved.

param (
    [switch]$SkipFrontendBuild = $false,
    [switch]$SignBinaries = $false,
    [switch]$DryRun = $false
)

$ErrorActionPreference = "Stop"

if ($SignBinaries) {
    throw "This helper does not implement code signing. Configure and review real Tauri Windows signing before building; -SignBinaries cannot claim a signed release."
}
$PackagingArguments = @((Join-Path $PSScriptRoot "package.cjs"), "--platform", "win32")
if ($SkipFrontendBuild) { $PackagingArguments += "--skip-frontend-build" }
if ($DryRun) { $PackagingArguments += "--dry-run" }
& node @PackagingArguments
if ($LASTEXITCODE -ne 0) { throw "Tauri packaging failed with exit code $LASTEXITCODE" }
