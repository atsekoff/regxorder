# Project Guidelines

## Project Snapshot

- regxorder is a Windows-first keyboard and mouse recorder and replayer.
- The project is library-first: the Rust SDK is the primary product surface, the CLI is a thin wrapper over it, and the Slint UI comes after the SDK and CLI are stable.
- Use `docs/implementation-plan.md` as the source of truth for scope, architecture, testing expectations, and phased milestones.

## Architecture

- Keep the core domain platform-agnostic. Do not leak Win32 handles, raw messages, or unsafe code into the core crate.
- Isolate unsafe Windows interop in the Windows backend crate behind thin safe wrappers.
- Treat recording, playback, and hotkey handling as pluggable backends.
- V1 defaults are Raw Input for recording, SendInput for playback, RegisterHotKey for global hotkeys, and Slint for the desktop UI.
- Do not expand V1 toward anti-cheat support, kernel drivers, or stealth-oriented input injection.

## Code And API

- Prefer explicit domain types and typed errors over loose primitives and stringly typed state.
- Prefer verbose, unambiguous names for types, fields, functions, variables, and commands so intent is obvious at a glance.
- Preserve deterministic semantics: strict event ordering, relative timestamps, speed scaling, explicit cleanup on abort, and clear capability boundaries.
- Keep business rules in shared services or the core SDK. The CLI and UI should stay thin.
- Favor small, composable modules that are easy to test with fake clocks and fake backends.

## Workflow

- Before any commit, run the appropriate formatter for changed code and stage the formatted result.
- For Rust code, the baseline formatting command is `cargo fmt --all`.

## Testing

- High test coverage is a project goal, especially in the core domain.
- Treat tests as executable documentation. Use descriptive scenario-based test names and cover success, failure, validation, and cleanup behavior.
- Prefer unit tests for scheduling, serialization, validation, and orchestration. Use targeted integration tests for CLI wiring and live Windows behavior.
- Avoid fragile sleeps or environment-dependent tests when a deterministic fake will verify the behavior better.

## UI

- The UI direction is Slint.
- Favor clarity, responsiveness, keyboard-friendly workflows, and efficient handling of large recordings over decorative visuals or heavy animation.
- Use terminology in the UI that matches the SDK and CLI.
