[CmdletBinding()]
param(
    [string]$OutputDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$OutputDirectory = if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    Join-Path $repoRoot "dist"
}
else {
    $OutputDirectory
}

$bundleSourceRoot = Join-Path $PSScriptRoot "cli-share"
$bundleName = "regxorder-cli-share"
$bundleRoot = Join-Path $OutputDirectory $bundleName
$zipPath = Join-Path $OutputDirectory "$bundleName.zip"
$releaseBinaryPath = Join-Path $repoRoot "target\release\regxorder-cli.exe"

Push-Location $repoRoot
try {
    Write-Host "Building regxorder-cli release binary..."
    cargo build -p regxorder-cli --release
}
finally {
    Pop-Location
}

if (-not (Test-Path $releaseBinaryPath)) {
    throw "Expected release binary at '$releaseBinaryPath', but it was not created."
}

if (Test-Path $bundleRoot) {
    Remove-Item -Path $bundleRoot -Recurse -Force
}

if (Test-Path $zipPath) {
    Remove-Item -Path $zipPath -Force
}

New-Item -ItemType Directory -Path $bundleRoot -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $bundleRoot "sessions") -Force | Out-Null

Copy-Item -Path $releaseBinaryPath -Destination (Join-Path $bundleRoot "regxorder-cli.exe")
Copy-Item -Path (Join-Path $bundleSourceRoot "record-session.ps1") -Destination (Join-Path $bundleRoot "record-session.ps1")
Copy-Item -Path (Join-Path $bundleSourceRoot "play-session.ps1") -Destination (Join-Path $bundleRoot "play-session.ps1")
Copy-Item -Path (Join-Path $bundleSourceRoot "README.md") -Destination (Join-Path $bundleRoot "README.md")

Compress-Archive -Path $bundleRoot -DestinationPath $zipPath

Write-Host "Created bundle folder: $bundleRoot"
Write-Host "Created shareable zip: $zipPath"
