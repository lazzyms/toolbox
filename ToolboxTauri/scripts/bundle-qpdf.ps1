# Bundles a qpdf.exe alongside the built Windows Toolbox executable so Protect
# works out of the box. Run after `npm run tauri build`.
#
# Copies qpdf.exe next to toolbox.exe in the release dir. Tauri's NSIS/MSI
# bundles pick up extra files that live in the release directory when they are
# listed via the bundle `resources` config; placing qpdf.exe beside the binary
# keeps the NSIS installer able to include it.
param(
    [string]$ReleaseDir = "$PSScriptRoot\..\src-tauri\target\release"
)

$ErrorActionPreference = "Stop"

$exe = Join-Path $ReleaseDir "toolbox.exe"
if (-not (Test-Path $exe)) {
    Write-Error "bundle-qpdf: toolbox.exe not found in $ReleaseDir"
}

$qpdf = Get-Command qpdf -ErrorAction SilentlyContinue
if (-not $qpdf) {
    Write-Error "bundle-qpdf: qpdf not found; install it (choco install qpdf / scoop install qpdf) or add it to PATH"
}

Copy-Item $qpdf.Source (Join-Path $ReleaseDir "qpdf.exe") -Force
Write-Host "bundle-qpdf: embedded qpdf.exe -> $ReleaseDir\qpdf.exe"

# The Rust runtime also checks TOOLBOX_QPDF_PATH before PATH, so a QPDF sibling
# is discovered in dev/testing; in the installed app the NSIS/MSI path must list
# qpdf.exe as an extra bundle resource so it ships next to toolbox.exe.
