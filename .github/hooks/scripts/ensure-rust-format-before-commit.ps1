$inputPayload = [Console]::In.ReadToEnd()

if ([string]::IsNullOrWhiteSpace($inputPayload)) {
    return
}

$looksLikeCommit = $inputPayload -match 'git\s+commit'
$looksLikeTerminalTool = $inputPayload -match 'run_in_terminal'

if (-not $looksLikeCommit -or -not $looksLikeTerminalTool) {
    @{
        hookSpecificOutput = @{
            hookEventName      = 'PreToolUse'
            permissionDecision = 'allow'
        }
    } | ConvertTo-Json -Compress
    return
}

function Write-HookDecision {
    param(
        [Parameter(Mandatory = $true)]
        [string]$PermissionDecision,
        [string]$PermissionDecisionReason
    )

    $hookSpecificOutput = @{
        hookEventName      = 'PreToolUse'
        permissionDecision = $PermissionDecision
    }

    if (-not [string]::IsNullOrWhiteSpace($PermissionDecisionReason)) {
        $hookSpecificOutput.permissionDecisionReason = $PermissionDecisionReason
    }

    @{
        hookSpecificOutput = $hookSpecificOutput
    } | ConvertTo-Json -Compress
}

try {
    $checkOutput = & (Join-Path $PSScriptRoot 'check-commit-readiness.ps1') 2>&1 | Out-String
    $checkStatus = $LASTEXITCODE
}
catch {
    $checkOutput = $_.Exception.Message
    $checkStatus = 1
}

if ($checkStatus -eq 0) {
    Write-HookDecision -PermissionDecision 'allow'
    return
}

$denialReason = $checkOutput.Trim()
if ([string]::IsNullOrWhiteSpace($denialReason)) {
    $denialReason = 'Commit readiness checks failed.'
}

Write-HookDecision -PermissionDecision 'deny' -PermissionDecisionReason $denialReason
