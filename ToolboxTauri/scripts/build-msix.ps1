<#
.SYNOPSIS
    Builds an unsigned x64 MSIX package for Microsoft Store submission.

.DESCRIPTION
    Account-specific Partner Center values are required as parameters or
    environment variables. The generated manifest and package stay under
    .artifacts and are intentionally not committed.
#>
[CmdletBinding()]
param(
    [string]$PackageIdentityName = $env:TOOLBOX_MSIX_IDENTITY_NAME,
    [string]$Publisher = $env:TOOLBOX_MSIX_PUBLISHER,
    [string]$PublisherDisplayName = $env:TOOLBOX_MSIX_PUBLISHER_DISPLAY_NAME,
    [string]$StoreName,
    [string]$Description = "A privacy-first desktop toolbox for working with files.",
    [string]$Version,
    [string]$MainBinaryName,
    [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$tauriRoot = Join-Path $projectRoot "src-tauri"
$configPath = Join-Path $tauriRoot "tauri.conf.json"
$storeConfigPath = Join-Path $tauriRoot "tauri.windows-store.conf.json"
$manifestTemplatePath = Join-Path $projectRoot "packaging\windows-msix\AppxManifest.xml.template"
$artifactRoot = Join-Path $projectRoot ".artifacts\msix"
$stage = Join-Path $artifactRoot "stage"

function Require-Value([string]$Name, [string]$Value) {
    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "$Name is required. Pass -$Name or set the matching TOOLBOX_MSIX_* environment variable."
    }
    return $Value
}

function Escape-Xml([string]$Value) {
    return [System.Security.SecurityElement]::Escape($Value)
}

function Get-MsixVersion([string]$ConfiguredVersion) {
    $parts = $ConfiguredVersion.Split(".")
    $nonNumericParts = @($parts | Where-Object { $_ -notmatch '^[0-9]+$' })
    if ($parts.Count -ne 3 -or $nonNumericParts.Count -gt 0) {
        throw "Version '$ConfiguredVersion' must be a three-part numeric Tauri version."
    }
    foreach ($part in $parts) {
        $number = [int64]$part
        if ($number -gt 65535) { throw "Version component '$part' is greater than 65535." }
    }
    if ([int64]$parts[0] -eq 0) { throw "The MSIX major version cannot be 0." }
    return "$($parts[0]).$($parts[1]).$($parts[2]).0"
}

function Copy-IntoStage([string]$Source, [string]$Destination) {
    $sourcePath = Join-Path $projectRoot $Source
    $destinationPath = Join-Path $stage ($Destination -replace '/', '\')
    if (Test-Path -LiteralPath $sourcePath -PathType Container) {
        New-Item -ItemType Directory -Force -Path $destinationPath | Out-Null
        Copy-Item (Join-Path $sourcePath '*') $destinationPath -Recurse -Force
    } elseif (Test-Path -LiteralPath $sourcePath -PathType Leaf) {
        New-Item -ItemType Directory -Force -Path (Split-Path $destinationPath) | Out-Null
        Copy-Item -LiteralPath $sourcePath -Destination $destinationPath -Force
    } else {
        throw "Configured bundle input does not exist: $Source"
    }
}

function Copy-ConfiguredInputs($Configured, [string]$Kind) {
    if ($null -eq $Configured) { return }
    $entries = if ($Configured -is [array]) { $Configured } else { @($Configured) }
    foreach ($entry in $entries) {
        if ($entry -is [string]) {
            $source = $entry
            $wildcard = $source.IndexOfAny([char[]]@('*', '?')) -ge 0
            if ($wildcard) {
                $matches = Get-ChildItem -Path (Join-Path $projectRoot $source) -File -Recurse
                foreach ($match in $matches) {
                    $relative = [IO.Path]::GetRelativePath($projectRoot, $match.FullName)
                    Copy-IntoStage $relative $relative
                }
            } else {
                $name = Split-Path $source -Leaf
                if ($Kind -eq "externalBin") {
                    $candidates = @($source, "$source-$Target", "$source-$Target.exe", "$source.exe") |
                        Select-Object -Unique | Where-Object { Test-Path -LiteralPath (Join-Path $projectRoot $_) -PathType Leaf }
                    if (-not $candidates) { throw "Configured external binary does not exist for $Target`: $source" }
                    foreach ($candidate in $candidates) {
                        $relative = [IO.Path]::GetRelativePath($projectRoot, (Join-Path $projectRoot $candidate))
                        Copy-IntoStage $relative $relative
                    }
                } else {
                    Copy-IntoStage $source $source
                }
            }
        } elseif ($entry -is [hashtable] -or $entry -is [pscustomobject]) {
            foreach ($property in $entry.PSObject.Properties) {
                Copy-IntoStage ([string]$property.Name) ([string]$property.Value)
            }
        } else {
            throw "Unsupported $Kind bundle configuration entry."
        }
    }
}

$config = Get-Content -Raw -LiteralPath $configPath | ConvertFrom-Json
$PackageIdentityName = Require-Value "PackageIdentityName" $PackageIdentityName
$Publisher = Require-Value "Publisher" $Publisher
$PublisherDisplayName = Require-Value "PublisherDisplayName" $PublisherDisplayName
if ([string]::IsNullOrWhiteSpace($StoreName)) { $StoreName = "Toolbox: PDF & File Tools" }
if ([string]::IsNullOrWhiteSpace($Version)) { $Version = [string]$config.version }
if ([string]::IsNullOrWhiteSpace($MainBinaryName)) { $MainBinaryName = "toolbox" }
$msixVersion = Get-MsixVersion $Version

if (-not (Test-Path -LiteralPath $storeConfigPath) -or -not (Test-Path -LiteralPath $manifestTemplatePath)) {
    throw "Store packaging configuration or manifest template is missing."
}

if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $stage "Assets") | Out-Null

Push-Location $projectRoot
try {
    $env:VITE_ENABLE_UPDATES = "false"
    & npm run tauri -- build --no-bundle --target $Target --config src-tauri/tauri.windows-store.conf.json
    if ($LASTEXITCODE -ne 0) { throw "Tauri build failed with exit code $LASTEXITCODE." }
} finally {
    Pop-Location
}

$releaseDirectories = @(
    (Join-Path -Path $tauriRoot -ChildPath ("target\{0}\release" -f $Target)),
    (Join-Path -Path $tauriRoot -ChildPath "target\release")
)
$executable = $null
foreach ($releaseDirectory in $releaseDirectories) {
    $candidate = Join-Path -Path $releaseDirectory -ChildPath ("{0}.exe" -f $MainBinaryName)
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        $executable = Get-Item -LiteralPath $candidate
        break
    }
}
if ($null -eq $executable) { throw "Could not find the release executable for $MainBinaryName ($Target)." }
Copy-Item -LiteralPath $executable.FullName -Destination (Join-Path $stage "$MainBinaryName.exe") -Force

$resources = $config.bundle.PSObject.Properties["resources"]
$externalBin = $config.bundle.PSObject.Properties["externalBin"]
if ($resources) { Copy-ConfiguredInputs $resources.Value "resources" }
if ($externalBin) { Copy-ConfiguredInputs $externalBin.Value "externalBin" }

foreach ($asset in @("StoreLogo.png", "Square44x44Logo.png", "Square150x150Logo.png")) {
    $source = Join-Path $tauriRoot "icons\$asset"
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing Store asset: $source" }
    Copy-Item -LiteralPath $source -Destination (Join-Path $stage "Assets\$asset") -Force
}

$manifest = Get-Content -Raw -LiteralPath $manifestTemplatePath
$replacements = @{
    "[PARTNER_CENTER_PACKAGE_IDENTITY_NAME]" = (Escape-Xml $PackageIdentityName)
    "[PARTNER_CENTER_PUBLISHER]" = (Escape-Xml $Publisher)
    "[PARTNER_CENTER_PUBLISHER_DISPLAY_NAME]" = (Escape-Xml $PublisherDisplayName)
    "[RESERVED_STORE_NAME]" = (Escape-Xml $StoreName)
    "[SHORT_APP_DESCRIPTION]" = (Escape-Xml $Description)
    "[MSIX_VERSION]" = $msixVersion
    "[APP_EXECUTABLE_NAME]" = (Escape-Xml $MainBinaryName)
}
foreach ($placeholder in $replacements.Keys) { $manifest = $manifest.Replace($placeholder, $replacements[$placeholder]) }
if ($manifest -match '\[[A-Z0-9_]+\]') { throw "Unresolved manifest placeholder: $($Matches[0])" }
$manifestPath = Join-Path $stage "AppxManifest.xml"
[IO.File]::WriteAllText($manifestPath, $manifest, [Text.UTF8Encoding]::new($false))

$makeAppx = Get-Command MakeAppx.exe -ErrorAction SilentlyContinue
if (-not $makeAppx) {
    $windowsKitsBin = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    if (Test-Path -LiteralPath $windowsKitsBin -PathType Container) {
        $makeAppx = Get-ChildItem -LiteralPath $windowsKitsBin -Recurse -Filter MakeAppx.exe -File |
            Where-Object { $_.FullName -match "\\x64\\MakeAppx\.exe$" } |
            Sort-Object FullName -Descending |
            Select-Object -First 1
    }
}
if (-not $makeAppx) { throw "MakeAppx.exe was not found. Run from a Windows SDK Developer Command Prompt." }
$makeAppxPath = if ($makeAppx.PSObject.Properties.Name -contains "Source") {
    [string]$makeAppx.Source
} else {
    [string]$makeAppx.FullName
}
$packagePath = Join-Path $artifactRoot "Toolbox_${msixVersion}_x64.msix"
if (Test-Path -LiteralPath $packagePath) { Remove-Item -LiteralPath $packagePath -Force }
& $makeAppxPath pack /d $stage /p $packagePath /h SHA256 /o /v
if ($LASTEXITCODE -ne 0) { throw "MakeAppx.exe failed with exit code $LASTEXITCODE." }

$hash = (Get-FileHash -LiteralPath $packagePath -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Host "MSIX package: $packagePath"
Write-Host "SHA-256: $hash"
