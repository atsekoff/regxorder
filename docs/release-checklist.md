# Release Checklist

Use this checklist before creating and pushing a release tag.

## Branch Rules

- Land release-bound changes on `main` through a pull request.
- Keep `main` green before tagging.
- Treat `material_ui_transition` and other feature branches as non-release branches until they are merged.

## Local Verification

Run these commands from the repository root:

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
powershell -ExecutionPolicy Bypass -File .\packaging\package-cli-share.ps1
cargo build -p regxorder-ui --release
```

Verify:

- `dist/regxorder-cli-share.zip` exists
- `target/release/regxorder-cli.exe` exists
- `target/release/regxorder-ui.exe` exists
- The CLI share bundle contains `regxorder-cli.exe`, `record-session.ps1`, `play-session.ps1`, `README.md`, and `sessions/`

## Release Tagging

Create a semantic version tag from `main`:

```powershell
git checkout main
git pull --ff-only
git tag v0.1.0
git push origin main
git push origin v0.1.0
```

Replace `v0.1.0` with the actual release version.

## GitHub Verification

After the tag is pushed:

- Confirm the `Windows Release` workflow starts for the tag.
- Wait for the tagged workflow to finish successfully.
- Confirm the GitHub Release page exists for the tag.
- Confirm the release assets include:
  - `regxorder-cli-share.zip`
  - `regxorder-cli.exe`
  - `regxorder-ui.exe`

## Notes

- Release automation currently targets Windows artifacts.
- Playback into elevated targets may still require elevation at runtime.
- V1 scope remains Windows-only and excludes anti-cheat-protected targets.
