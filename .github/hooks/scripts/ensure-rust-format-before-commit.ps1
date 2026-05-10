$inputPayload = [Console]::In.ReadToEnd()

if ([string]::IsNullOrWhiteSpace($inputPayload)) {
    return
}

$looksLikeCommit = $inputPayload -match 'git\s+commit'
$looksLikeTerminalTool = $inputPayload -match 'run_in_terminal'

if (-not $looksLikeCommit -or -not $looksLikeTerminalTool) {
    @{
        hookSpecificOutput = @{
            hookEventName = 'PreToolUse'
            permissionDecision = 'allow'
        }
    } | ConvertTo-Json -Compress
    return
}

$workspaceRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$cargoManifest = Join-Path $workspaceRoot 'Cargo.toml'

if (-not (Test-Path $cargoManifest)) {
    @{
        hookSpecificOutput = @{
            hookEventName = 'PreToolUse'
            permissionDecision = 'allow'
        }
    } | ConvertTo-Json -Compress
    return
}

Push-Location $workspaceRoot
try {
    cargo fmt --all --check | Out-Null
    $formatStatus = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($formatStatus -eq 0) {
    @{
        hookSpecificOutput = @{
            hookEventName = 'PreToolUse'
            permissionDecision = 'allow'
        }
    } | ConvertTo-Json -Compress
    return
}

@{
    hookSpecificOutput = @{
        hookEventName = 'PreToolUse'
        permissionDecision = 'deny'
        permissionDecisionReason = 'Rust files are not formatted. Run cargo fmt --all, restage the changes, and then commit.'
    }
} | ConvertTo-Json -Compress