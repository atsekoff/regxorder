$checkScript = Resolve-Path (Join-Path $PSScriptRoot '..\.github\hooks\scripts\check-commit-readiness.ps1')

& $checkScript
exit $LASTEXITCODE