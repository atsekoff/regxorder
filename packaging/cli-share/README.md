# regxorder CLI Share Bundle

This bundle contains a release build of `regxorder-cli.exe` plus two helper scripts:

- `record-session.ps1` records a new session into the local `sessions` folder.
- `play-session.ps1` replays a recorded session from the local `sessions` folder.

Keep all extracted files together in the same folder. The scripts expect `regxorder-cli.exe` to sit next to them.

## What Is Included

- `regxorder-cli.exe`
- `record-session.ps1`
- `play-session.ps1`
- `sessions\`

## Requirements

- Windows 10 or Windows 11
- PowerShell
- A normal desktop target application

Notes:

- Playback into elevated applications may require `-Elevate` so Windows can prompt for administrator rights.
- Replay is intended for the same machine and display setup that the session was recorded on.
- Anti-cheat-protected targets are out of scope.

## Record A Session

Open PowerShell in the extracted folder and run:

```powershell
powershell -ExecutionPolicy Bypass -File .\record-session.ps1 -Name demo
```

Default recording flow:

1. Run the script.
2. Press `Ctrl+Shift+F9` to start recording.
3. Press `Ctrl+Shift+F10` to stop recording.

The recording is saved as `sessions\demo.json`.

Optional examples:

```powershell
powershell -ExecutionPolicy Bypass -File .\record-session.ps1 -Name demo -Title "Demo capture"
powershell -ExecutionPolicy Bypass -File .\record-session.ps1 -Name demo -DurationSeconds 15
powershell -ExecutionPolicy Bypass -File .\record-session.ps1 -Name demo -Strategy low-level-hooks
```

## Play A Session

Open PowerShell in the extracted folder and run:

```powershell
powershell -ExecutionPolicy Bypass -File .\play-session.ps1 -Name demo
```

Default playback flow:

1. Run the script.
2. Focus the target application.
3. Press `Ctrl+Shift+F9` to start playback.
4. Press `Ctrl+Shift+F10` to stop playback early if needed.

Optional examples:

```powershell
powershell -ExecutionPolicy Bypass -File .\play-session.ps1 -Name demo -Speed 1.5
powershell -ExecutionPolicy Bypass -File .\play-session.ps1 -Name demo -Elevate
```

## Troubleshooting

- If PowerShell blocks script execution, use the `-ExecutionPolicy Bypass` examples above.
- If playback does not affect the target app, try `-Elevate` when the target app is running as administrator.
- If the recording file is missing, confirm the JSON file is inside the local `sessions` folder.
