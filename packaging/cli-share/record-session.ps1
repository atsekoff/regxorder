[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$Name = (Get-Date -Format "yyyyMMdd-HHmmss"),

    [string]$Title,

    [double]$DurationSeconds,

    [ValidateSet("raw-input", "low-level-hooks")]
    [string]$Strategy = "raw-input",

    [string]$StartHotkey = "ctrl+shift+f9",

    [string]$StopHotkey = "ctrl+shift+f10"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$bundleRoot = $PSScriptRoot
$cliPath = Join-Path $bundleRoot "regxorder-cli.exe"
$sessionsDirectory = Join-Path $bundleRoot "sessions"

if (-not (Test-Path $cliPath)) {
    throw "Could not find regxorder-cli.exe next to this script. Keep the extracted bundle contents together."
}

New-Item -ItemType Directory -Path $sessionsDirectory -Force | Out-Null

$fileName = if ($Name.EndsWith(".json", [System.StringComparison]::OrdinalIgnoreCase)) {
    $Name
}
else {
    "$Name.json"
}

$outputPath = Join-Path $sessionsDirectory $fileName
$arguments = @(
    "record",
    "--output", $outputPath,
    "--strategy", $Strategy,
    "--start-hotkey", $StartHotkey,
    "--stop-hotkey", $StopHotkey
)

if ($Title) {
    $arguments += @("--title", $Title)
}

if ($PSBoundParameters.ContainsKey("DurationSeconds")) {
    $arguments += @(
        "--duration-seconds",
        $DurationSeconds.ToString([System.Globalization.CultureInfo]::InvariantCulture)
    )
}

Write-Host "Recording will be saved to: $outputPath"
Write-Host "Press $StartHotkey to start recording."
Write-Host "Press $StopHotkey to stop recording, or press Ctrl+C in this window."

& $cliPath @arguments

if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
