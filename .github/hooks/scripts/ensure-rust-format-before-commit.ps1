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

$workspaceRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$cargoManifest = Join-Path $workspaceRoot 'Cargo.toml'

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

if (-not (Test-Path $cargoManifest)) {
    Write-HookDecision -PermissionDecision 'allow'
    return
}

Push-Location $workspaceRoot
try {
    $stagedPaths = @(git diff --cached --name-only --diff-filter=ACMR)
    if ($LASTEXITCODE -ne 0) {
        Write-HookDecision -PermissionDecision 'deny' -PermissionDecisionReason 'Unable to inspect staged files before commit.'
        return
    }

    $unstagedPaths = @(git diff --name-only --diff-filter=ACMR)
    if ($LASTEXITCODE -ne 0) {
        Write-HookDecision -PermissionDecision 'deny' -PermissionDecisionReason 'Unable to inspect unstaged files before commit.'
        return
    }

    $stagedPathLookup = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($stagedPath in $stagedPaths) {
        [void]$stagedPathLookup.Add($stagedPath)
    }

    $pathsNeedingRestage = @(
        $unstagedPaths | Where-Object {
            $stagedPathLookup.Contains($_)
        }
    )
    if ($pathsNeedingRestage.Count -gt 0) {
        $pathList = ($pathsNeedingRestage | Sort-Object | Select-Object -First 5) -join ', '
        Write-HookDecision -PermissionDecision 'deny' -PermissionDecisionReason "Some staged files also have unstaged changes. Save, format, and restage every changed file before committing. Examples: $pathList"
        return
    }

    git diff --cached --check | Out-Null
    $stagedDiffCheckStatus = $LASTEXITCODE
    if ($stagedDiffCheckStatus -ne 0) {
        Write-HookDecision -PermissionDecision 'deny' -PermissionDecisionReason 'Staged files still contain whitespace or newline problems. Apply the appropriate formatter, restage the files, and then commit.'
        return
    }

    cargo fmt --all --check | Out-Null
    $rustFormatStatus = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($rustFormatStatus -eq 0) {
    Write-HookDecision -PermissionDecision 'allow'
    return
}

Write-HookDecision -PermissionDecision 'deny' -PermissionDecisionReason 'Rust files are not formatted. Run cargo fmt --all, restage every affected file, and then commit.'
