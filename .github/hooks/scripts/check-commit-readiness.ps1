$workspaceRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$cargoManifest = Join-Path $workspaceRoot 'Cargo.toml'

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
        Write-Output 'Unable to inspect staged files before commit.'
        exit 1
    }

    $unstagedPaths = @(
        git diff --name-only --diff-filter=ACMR | Where-Object {
            -not [string]::IsNullOrWhiteSpace($_)
        }
    )
    if ($LASTEXITCODE -ne 0) {
        Write-Output 'Unable to inspect unstaged files before commit.'
        exit 1
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
        Write-Output "Some staged files also have unstaged changes. Save, format, and restage every changed file before committing. Examples: $pathList"
        exit 1
    }

    git diff --cached --check | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Output 'Staged files still contain whitespace or newline problems. Apply the appropriate formatter, restage the files, and then commit.'
        exit 1
    }

    cargo fmt --all --check | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Output 'Rust files are not formatted. Run cargo fmt --all, restage every affected file, and then commit.'
        exit 1
    }

    cargo clippy --workspace --all-targets -- -D warnings | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Output 'Rust lint checks failed. Run cargo clippy --workspace --all-targets -- -D warnings, fix the reported issues, restage every affected file, and then commit.'
        exit 1
    }
}
finally {
    Pop-Location
}

exit 0
