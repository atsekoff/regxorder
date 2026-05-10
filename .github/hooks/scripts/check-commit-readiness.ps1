$workspaceRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$cargoManifest = Join-Path $workspaceRoot 'Cargo.toml'

function Fail-CommitReadinessCheck {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    Write-Output $Message
    exit 1
}

if (-not (Test-Path $cargoManifest)) {
    exit 0
}

Push-Location $workspaceRoot
try {
    $stagedPaths = @(
        git diff --cached --name-only --diff-filter=ACMR | Where-Object {
            -not [string]::IsNullOrWhiteSpace($_)
        }
    )
    if ($LASTEXITCODE -ne 0) {
        Fail-CommitReadinessCheck -Message 'Unable to inspect staged files before commit.'
    }

    $unstagedPaths = @(
        git diff --name-only --diff-filter=ACMR | Where-Object {
            -not [string]::IsNullOrWhiteSpace($_)
        }
    )
    if ($LASTEXITCODE -ne 0) {
        Fail-CommitReadinessCheck -Message 'Unable to inspect unstaged files before commit.'
    }

    $stagedPathLookup = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
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
        Fail-CommitReadinessCheck -Message "Some staged files also have unstaged changes. Save, format, and restage every changed file before committing. Examples: $pathList"
    }

    git diff --cached --check | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Fail-CommitReadinessCheck -Message 'Staged files still contain whitespace or newline problems. Apply the appropriate formatter, restage the files, and then commit.'
    }

    cargo fmt --all --check | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Fail-CommitReadinessCheck -Message 'Rust files are not formatted. Run cargo fmt --all, restage every affected file, and then commit.'
    }
}
finally {
    Pop-Location
}

exit 0