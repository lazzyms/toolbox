# Stages the exact qpdf release pinned in qpdf-pin.json for one Windows target into
# the bundle resources directory, so the installer ships a native qpdf.exe and its DLLs.
# The archive must match its pinned SHA-256, and the staged qpdf.exe must be native to
# the target and at or above the pinned minimum. Run before `npm run tauri build`.
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc")]
    [string]$Target,
    [string]$ResourcesDir = (Join-Path $PSScriptRoot "..\src-tauri\resources"),
    [string]$Archive
)

$ErrorActionPreference = "Stop"

$pin = Get-Content (Join-Path $PSScriptRoot "..\qpdf-pin.json") -Raw | ConvertFrom-Json
$entry = $pin.windows.archives.$Target
if (-not $entry) { throw "bundle-qpdf: qpdf-pin.json has no archive for $Target" }

$work = Join-Path ([System.IO.Path]::GetTempPath()) "toolbox-qpdf-$($pin.windows.version)-$Target"
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force -Path $work | Out-Null

if (-not $Archive) {
    $Archive = Join-Path $work "qpdf-$($pin.windows.version).zip"
    Invoke-WebRequest -Uri $entry.url -OutFile $Archive
}
$actual = (Get-FileHash $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $entry.sha256) {
    throw "bundle-qpdf: qpdf archive SHA-256 $actual does not match the pinned $($entry.sha256) for $Target"
}

$extracted = Join-Path $work "extracted"
Expand-Archive -Path $Archive -DestinationPath $extracted -Force
$qpdf = Get-ChildItem $extracted -Recurse -Filter qpdf.exe | Select-Object -First 1
if (-not $qpdf) { throw "bundle-qpdf: the pinned qpdf archive has no qpdf.exe" }

function Assert-Qpdf([string]$Path) {
    & node (Join-Path $PSScriptRoot "check-release-readiness.mjs") --qpdf $Path --target $Target
    if ($LASTEXITCODE -ne 0) { throw "bundle-qpdf: refusing qpdf at $Path" }
}

Assert-Qpdf $qpdf.FullName
New-Item -ItemType Directory -Force -Path $ResourcesDir | Out-Null
Copy-Item $qpdf.FullName (Join-Path $ResourcesDir "qpdf.exe") -Force
Get-ChildItem $qpdf.DirectoryName -Filter *.dll | Copy-Item -Destination $ResourcesDir -Force
Assert-Qpdf (Join-Path $ResourcesDir "qpdf.exe")
Write-Host "bundle-qpdf: staged qpdf $($pin.windows.version) for $Target -> $ResourcesDir"
