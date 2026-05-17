# regxorder

regxorder is a Windows-first keyboard and mouse recorder and replayer written in Rust.

The project is library-first:

- `regxorder-core` defines the canonical recording model, validation rules, diagnostics, and playback-preparation logic.
- `regxorder-win32` provides the Windows recording, playback, elevation, and hotkey backends.
- `regxorder-cli` is the thin scriptable command-line surface for recording, inspection, validation, diagnostics, and replay.
- `regxorder-ui` is the Slint desktop application and is still in active iteration.

## Current Focus

The most stable surfaces today are the Rust crates and the CLI workflow. The UI is being developed on a separate branch and should be treated as work in progress.

## Workspace Crates

### `regxorder-core`

Purpose:

- Platform-agnostic event types and recording schema
- Import and export helpers for canonical JSON
- Validation, diagnostics, and playback-plan preparation

Use it when:

- You want to inspect, validate, serialize, or transform recordings in Rust
- You want stable domain types without pulling in Windows API code

Example:

```rust
use regxorder_core::{Recording, SpeedMultiplier, diagnose_playback};

let json = std::fs::read_to_string("sessions/capture-001.json")?;
let recording = Recording::from_json_str(&json)?;
let speed = SpeedMultiplier::new(1.0)?;
let report = diagnose_playback(&recording, speed)?;

println!("prepared events: {}", report.prepared_event_count);
# Ok::<(), Box<dyn std::error::Error>>(())
```

### `regxorder-win32`

Purpose:

- Raw Input and low-level hook recording backends
- SendInput playback backend
- Global hotkey handling and elevation helpers

Use it when:

- You need actual recording or playback on Windows
- You are embedding regxorder into another Rust application

Example:

```rust
use std::sync::atomic::AtomicBool;

use regxorder_core::{Recording, SpeedMultiplier};
use regxorder_win32::play_recording;

let json = std::fs::read_to_string("sessions/capture-001.json")?;
let recording = Recording::from_json_str(&json)?;
let stop_requested = AtomicBool::new(false);

let report = play_recording(&recording, SpeedMultiplier::new(1.0)?, &stop_requested)?;
println!("dispatched {} events", report.dispatched_events);
# Ok::<(), Box<dyn std::error::Error>>(())
```

### `regxorder-cli`

Purpose:

- Script-friendly wrapper over the SDK
- Record, play, inspect, validate, doctor, and diagnostics commands
- Best current surface for sharing replay flows with non-Rust users

Use it when:

- You want to record or replay sessions without writing Rust code
- You want a simple executable that can be bundled with helper scripts

Examples:

```powershell
cargo run -p regxorder-cli -- sample --output demo.json --title "Demo capture"
cargo run -p regxorder-cli -- validate --input demo.json
cargo run -p regxorder-cli -- play --input demo.json --speed 1.0 --elevate
```

The repository also includes a shareable CLI bundle under `packaging/cli-share/` and a packaging script at `packaging/package-cli-share.ps1`.

### `regxorder-ui`

Purpose:

- Desktop application built with Slint
- Recording library browsing, playback controls, and event editing surfaces

Use it when:

- You want to explore the in-progress desktop UX
- You are contributing to the app shell and editor experience

Example:

```powershell
cargo run -p regxorder-ui -- --elevate
```

## Quick Start

### Build the CLI

```powershell
cargo build -p regxorder-cli --release
```

### Build the shareable CLI bundle

```powershell
powershell -ExecutionPolicy Bypass -File .\packaging\package-cli-share.ps1
```

This creates `dist/regxorder-cli-share.zip`.

### Build the UI

```powershell
cargo build -p regxorder-ui --release
```

## GitHub Releases And Actions

GitHub Actions builds Windows release artifacts from the workflow in `.github/workflows/windows-release.yml`.

- Pushes and pull requests upload build artifacts.
- Tags that start with `v`, such as `v0.1.0`, also publish GitHub Release assets.

## Constraints

- Windows only in V1
- Intended for normal desktop targets on the same machine and display setup used for recording
- Playback into elevated targets may require elevation
- Anti-cheat-protected targets are out of scope