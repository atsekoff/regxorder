[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Name,

    [double]$Speed = 1.0,

    [string]$StartHotkey = "ctrl+shift+f9",

    [string]$StopHotkey = "ctrl+shift+f10",

    [switch]$Elevate
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$bundleRoot = $PSScriptRoot
$cliPath = Join-Path $bundleRoot "regxorder-cli.exe"
$sessionsDirectory = Join-Path $bundleRoot "sessions"

if (-not (Test-Path $cliPath)) {
    throw "Could not find regxorder-cli.exe next to this script. Keep the extracted bundle contents together."
}

$fileName = if ($Name.EndsWith(".json", [System.StringComparison]::OrdinalIgnoreCase)) {
    $Name
}
else {
    "$Name.json"
}

$inputPath = Join-Path $sessionsDirectory $fileName

if (-not (Test-Path $inputPath)) {
    throw "Could not find recording '$inputPath'. Put the JSON recording inside the sessions folder or pass a matching file name."
}

$arguments = @(
    "play",
    "--input", $inputPath,
    "--speed", $Speed.ToString([System.Globalization.CultureInfo]::InvariantCulture),
    "--start-hotkey", $StartHotkey,
    "--stop-hotkey", $StopHotkey
)

if ($Elevate) {
    $arguments += "--elevate"
}

Write-Host "Ready to play: $inputPath"
Write-Host "Focus the target window, then press $StartHotkey to begin playback."
Write-Host "Press $StopHotkey to stop playback early."

& $cliPath @arguments

if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
