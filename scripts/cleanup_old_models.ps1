# Removes the legacy BF16 ASR model caches from %APPDATA%\reflow\models.
# Safe to re-run. Preserves config/ and database/.

$ErrorActionPreference = "Stop"
$appData = $env:APPDATA
if (-not $appData) { throw "APPDATA not set" }

$root = Join-Path $appData "reflow"
if (-not (Test-Path $root)) { Write-Host "No reflow appdata at $root - nothing to do."; exit 0 }
if (-not (Test-Path (Join-Path $root "config"))) { throw "reflow\config missing - refusing to delete from an unfamiliar layout." }
if (-not (Test-Path (Join-Path $root "database"))) { throw "reflow\database missing - refusing to delete from an unfamiliar layout." }

$targets = @("qwen3-asr-0.6b", "qwen3-asr-1.7b")
$modelsDir = Join-Path $root "models"
foreach ($t in $targets) {
    $p = Join-Path $modelsDir $t
    if (Test-Path $p) {
        $size = (Get-ChildItem -Recurse $p -File -ErrorAction SilentlyContinue |
                 Measure-Object -Property Length -Sum).Sum
        Write-Host ("Removing {0,-30} {1,8:N2} MB" -f $p, ($size/1MB))
        Remove-Item -Recurse -Force $p
    } else {
        Write-Host "Skip (not present): $p"
    }
}
Write-Host "Done. Remaining under reflow\:"
Get-ChildItem $root | ForEach-Object {
    $size = (Get-ChildItem -Recurse $_ -File -ErrorAction SilentlyContinue |
             Measure-Object -Property Length -Sum).Sum
    "{0,-20} {1,8:N2} MB" -f $_.Name, ($size/1MB)
}
