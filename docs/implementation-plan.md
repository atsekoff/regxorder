# regxorder Implementation Plan

## Purpose

This document captures the current product scope, architectural decisions, testing expectations, and phased implementation plan for regxorder. It is intended to be a durable reference during design, implementation, code review, and release work. When a decision changes, update this file and the workspace instructions in the same change.

## Product Summary

regxorder is a Windows-first tool for recording keyboard and mouse input with precise relative timing and replaying it with deterministic event ordering and controllable speed. The primary integration surface is a Rust library. A CLI ships before the desktop UI, and the desktop UI uses Slint once the core SDK and CLI are stable.

## V1 Goals

1. Record keyboard and mouse events with precise relative timing.
2. Replay recordings with deterministic ordering and high timing fidelity on the same machine and display setup.
3. Support replay speed control without changing event order semantics.
4. Support global hotkeys while the app is unfocused, minimized, or in the tray.
5. Import and export recordings through a versioned text format.
6. Allow recordings to be edited both inside the app and outside the app as text.
7. Expose a stable Rust SDK so other programs can embed the core functionality directly.
8. Keep recording and playback backends modular so alternative strategies can be added later.
9. Keep the UI functional, intuitive, and performant rather than decorative.
10. Maintain high test coverage where it provides real confidence and use tests as executable documentation.

## Explicit Non-Goals For V1

1. Support for anti-cheat-protected targets.
2. Kernel driver development, driver signing, or stealth injection techniques.
3. Cross-platform support before Windows is proven.
4. Promising identical results across changed target application state, different monitor layouts, or different DPI environments.
5. Designing around a network API before the embedded SDK and CLI are stable.

## Working Definitions

### Deterministic Replay

For V1, deterministic replay means:

1. Recorded events preserve strict ordering.
2. Playback schedules events from recorded relative timestamps and the requested speed multiplier.
3. Playback reports or logs timing drift where timing matters.
4. Playback releases tracked pressed keys and buttons on completion, abort, cancellation, or failure.

It does not mean:

1. Guaranteed identical target application behavior if the application state has changed.
2. Guaranteed identical behavior across different monitor layouts or DPI setups.
3. Guaranteed compatibility with anti-cheat systems or unsupported input stacks.

### High Test Coverage

High test coverage means the project should aggressively cover core behavior, validation, and failure handling without gaming metrics. The core domain should be covered very heavily. Small Win32 interop shims should be verified indirectly through unit tests around safe wrappers and through targeted integration tests, not through brittle attempts to unit test every raw FFI edge.

### Tests As Documentation

Tests should explain behavior to future maintainers. A reader should be able to understand expected results, invalid inputs, cleanup guarantees, and edge cases by reading the test names and setup. Prefer descriptive test names, compact fixtures, and scenario-driven assertions.

## Decision Summary

| Area | Decision | Notes |
| --- | --- | --- |
| Platform | Windows only in V1 | Target Windows 10 and 11 first |
| Core language | Rust | Library-first architecture |
| UI | Slint | Functional and performant over visually rich |
| Default recording backend | Raw Input | Preferred Windows path for fidelity and lower overhead |
| Fallback recording backend | Low-level hooks | Compatibility fallback only |
| Default playback backend | SendInput | Practical Windows baseline |
| Global hotkeys | RegisterHotKey | Keep hotkeys simple and predictable in V1 |
| External integration | Embedded Rust SDK first | CLI is a thin wrapper over the SDK |
| Recording format | Versioned canonical JSON | Pretty-printed for editing, strict validation on import |
| Coordinates | Store exact and normalized | Include monitor and DPI metadata |
| Scope exclusions | Anti-cheat and kernel drivers | No stealth or unsupported low-level work in V1 |

## Guiding Principles

1. Library first. The SDK defines the product surface, and the CLI and UI are thin layers over it.
2. Isolate unsafe code. Keep Win32 interop and unsafe blocks in the Windows backend crate.
3. Prefer explicit contracts. Determinism, cleanup, timing, and capability boundaries should be represented in types and APIs, not implied.
4. Build for replacement. Recording and playback strategies should be swappable without rewriting the core domain.
5. Favor performance by design. Avoid unnecessary allocations, hidden polling, and chatty UI bindings.
6. Make failure modes obvious. Elevation problems, unsupported targets, malformed recordings, and backend capability gaps should surface actionable errors.
7. Test the important behavior. Use fake clocks, fake backends, and tight unit tests before reaching for live system integration.

## Proposed Workspace Layout

```text
regxorder/
|-- Cargo.toml
|-- .editorconfig
|-- .github/
|   |-- copilot-instructions.md
|   `-- instructions/
|       |-- docs.instructions.md
|       |-- rust-project.instructions.md
|       |-- slint.instructions.md
|       |-- testing.instructions.md
|       `-- windows-input.instructions.md
|-- .vscode/
|   |-- extensions.json
|   `-- settings.json
|-- docs/
|   `-- implementation-plan.md
`-- crates/
    |-- regxorder-core/
    |-- regxorder-win32/
    |-- regxorder-cli/
    `-- regxorder-ui/
```

## Crate Responsibilities

### regxorder-core

The platform-agnostic domain crate.

Expected responsibilities:

1. Event model and schema versioning.
2. Recording and playback plans.
3. Capability and backend traits.
4. Import, export, validation, and schema migration helpers.
5. Speed control, cleanup policies, and error types.
6. Test-only fake backends and fake clocks where practical.

### regxorder-win32

The Windows backend crate.

Expected responsibilities:

1. Raw Input recorder backend.
2. Low-level hook fallback backends.
3. SendInput player backend.
4. RegisterHotKey integration.
5. Dedicated message-loop thread.
6. High-resolution timing primitives and drift measurement.
7. Thin safe wrappers over Win32 APIs.

### regxorder-cli

The scriptable command-line surface over the SDK.

Expected responsibilities:

1. Record, play, abort, inspect, validate, import, and export commands.
2. Diagnostics for environment, permissions, and backend capabilities, including doctor-style preflight checks and deeper diagnostics output.
3. Minimal presentation logic only.

### regxorder-ui

The Slint desktop application.

Expected responsibilities:

1. Recording library and session management UI.
2. Playback controls and speed adjustment.
3. Import and export workflows.
4. Basic event editing.
5. Global hotkey configuration.
6. Tray and minimized workflow.

## Architecture Overview

### Core Event Model

Each recorded event should include:

1. Relative timestamp from session start.
2. Monotonic sequence number for strict ordering.
3. Event payload for keyboard or mouse action.
4. Exact coordinate data when applicable.
5. Normalized coordinate data when applicable.
6. Context metadata needed for validation or replay policy.

Recommended model details:

1. Use strong domain types rather than raw primitives for coordinates, button state, hotkeys, and speed multipliers.
2. Represent schema version explicitly.
3. Keep platform-specific raw payloads out of the public interchange format unless there is a strong migration story.

### Recording Architecture

1. Run recording on a dedicated message-loop thread.
2. Use Raw Input as the default keyboard and mouse capture backend.
3. Use low-level keyboard and mouse hooks only as optional fallback backends.
4. Convert raw backend events into the canonical event model as early as possible.
5. Record monitor and DPI metadata so coordinates can be validated later.
6. Keep callback paths minimal and hand work off quickly to channels or queues.
7. Keep both recorder strategies available as named backends. See `docs/recording-backend-strategies.md` for the current strengths, weaknesses, and selection guidance.

### Playback Architecture

1. Use SendInput as the default player backend.
2. Use a high-resolution monotonic timer and scheduler based on Windows timing primitives such as QueryPerformanceCounter.
3. Apply speed scaling to relative timestamps, not to event order.
4. Track pressed keys and buttons and release them on completion, abort, cancellation, or unexpected failure.
5. Capture expected versus actual dispatch timing for diagnostics.
6. Surface explicit errors when elevation or integrity boundaries block playback.

### Hotkey Architecture

1. Use RegisterHotKey in V1 for start, stop, playback, abort, and related global actions.
2. Keep hotkey behavior conservative and predictable.
3. Avoid complex hook-based hotkey parsing unless RegisterHotKey proves insufficient.
4. Make conflicts and registration failures visible to the user.
5. Keep hotkey chord parsing and action-boundary orchestration in shared services so the CLI and future UI reuse the same behavior instead of duplicating it.

### Session Boundary And Pressed-State Cleanup

1. Treat hotkey chords as control-plane input rather than user input that should appear in saved recordings.
2. For hotkeys that start recording or playback, trigger the action only after every key in the start chord has been released.
3. Do not rely on stop-hotkey suppression alone as the correctness guarantee, because a recorder may legitimately observe some or all of the stop chord before the control path stops the session.
4. Track pressed keyboard keys and mouse buttons centrally so recording finalization and playback cleanup can reconcile session state deterministically.
5. When a recording ends, reconcile any still-pressed keys or buttons by appending balancing release events at the session boundary so the canonical recording ends in a fully released state.
6. When playback ends, whether by normal completion, abort, cancellation, or failure, release all tracked pressed keys and buttons before returning control to the user.
7. Add focused tests around start-hotkey release barriers, stop-hotkey boundaries, long-held modifiers, and recordings that would otherwise end with keys or buttons still pressed.

### Prepared Playback Plans

1. Keep the canonical recording unchanged when handling hotkey artifacts, ghost release events, or session-end cleanup requirements.
2. Build a derived playback plan from the loaded recording and any future hotkey-control context rather than mutating the saved session text.
3. Prefer doing semantic cleanup in a preflight preparation pass before timed playback starts so the runtime dispatch loop stays focused on scheduling accuracy.
4. The preparation pass may ignore unmatched release events, append synthetic terminal releases for inputs still tracked as pressed at session end, and apply future hotkey-aware cleanup rules without rewriting the source recording.
5. Repeated key-down events should be preserved by default in the prepared playback plan unless explicit control-plane metadata proves they are artifacts, because held keys can legitimately generate repeated logical key-down messages that applications observe.
6. The runtime player should still track pressed state while dispatching the prepared plan so abort, failure, and partial-send cleanup remain deterministic.

### Import, Export, And Editing

1. Use pretty-printed, versioned JSON as the canonical text format.
2. Validate imported recordings strictly and fail clearly.
3. Preserve stable field names and schema migration rules.
4. Keep UI editing aligned with the text schema rather than inventing a separate hidden model.
5. Consider TOML import and export later if manual editing becomes common enough to justify the extra surface area.

### SDK And CLI Surface

The SDK should eventually cover:

1. Start recording.
2. Stop recording.
3. Play recording.
4. Abort playback.
5. Import recording.
6. Export recording.
7. Validate recording.
8. Set or query speed.
9. Select or inspect backend capabilities.
10. Query diagnostics such as elevation status and unsupported target conditions.
11. Select a recording strategy without changing the canonical recording format.

The CLI should remain a thin wrapper over the SDK and should not reimplement domain rules.

### UI Direction

The desktop UI uses Slint and should optimize for clarity and responsiveness.

UI expectations:

1. Functional over flashy.
2. Fast enough for large recordings and long event lists.
3. Keyboard-friendly where practical.
4. Clear status reporting for recording, playback, hotkeys, elevation state, and backend limitations.
5. Minimal animation.
6. Stable terminology that matches the SDK and CLI.

## Testing Strategy

### Testing Principles

1. Tests are part of the product documentation.
2. Prefer deterministic unit tests over slow environment-dependent tests.
3. Cover invalid inputs and cleanup paths, not only happy paths.
4. Do not rely on sleeps in unit tests when a fake clock or scheduler will prove the behavior more reliably.
5. Use integration tests for crate boundaries and selective end-to-end flows.
6. Keep manual testing for the small set of scenarios that truly need a live desktop environment.

### Coverage Priorities

Highest priority coverage:

1. Event ordering and sequencing.
2. Timestamp normalization and speed scaling.
3. Import and export round-trips.
4. Schema validation and migration.
5. Coordinate handling and normalization.
6. Cleanup behavior on abort and failure.
7. Capability checks and backend selection.
8. Error classification for malformed input and unsupported operations.
9. Session-boundary cleanup and pressed-state reconciliation for recording, playback, and hotkey-driven workflows.

Moderate priority coverage:

1. CLI command behavior.
2. Application service orchestration.
3. Slint view-model logic that contains business-relevant behavior.

Lower priority direct unit coverage:

1. Thin Win32 FFI shims that are better covered through wrapper tests and integration smoke tests.
2. Pure presentation details that do not affect workflow correctness.

### Recommended Test Types

| Test type | Purpose | Examples |
| --- | --- | --- |
| Unit tests | Document and verify business rules | speed scaling, ordering, validation, cleanup, coordinate conversion |
| Property or table-driven tests | Cover multiple cases compactly | schema validation, parser edge cases, speed multiplier behavior |
| Doctests or examples | Document public API usage | basic recording flow, import/export flow |
| Integration tests | Verify crate boundaries and CLI behavior | CLI record command wiring, import validation, SDK orchestration |
| Windows integration smoke tests | Verify live system behavior selectively | hotkey registration, SendInput smoke checks, Raw Input pipeline smoke checks |
| Manual acceptance tests | Final confidence in desktop behavior | tray workflow, multi-monitor replay, elevated target behavior |

### Tests As Documentation Rules

1. Use scenario-based test names that describe behavior.
2. Keep test setup short and visible.
3. Prefer one concept per test.
4. Use compact fixtures and helper builders instead of opaque setup blobs.
5. Add comments only when the scenario is not obvious from the setup and assertions.
6. When a public API should teach usage, add a doctest or example.

### Coverage Goals

1. Keep the core crate at very high coverage where practical.
2. Aim for exhaustive tests around serialization, scheduling, validation, and cleanup logic.
3. Do not chase 100 percent coverage if it forces poor abstractions or fake confidence.
4. If a critical behavior cannot be unit tested cleanly, treat that as a design smell and look for a better seam.

## Implementation Phases

### Phase 0 - Repository Setup

Deliverables:

1. Cargo workspace.
2. Core documentation and Copilot instructions.
3. Workspace settings and extension recommendations.
4. Baseline CI plan once code exists.

Exit criteria:

1. The workspace layout is committed.
2. The implementation plan and instructions reflect the current scope.

### Phase 1 - Domain Model And Schema

Deliverables:

1. Canonical event model.
2. Recording metadata model.
3. Schema versioning strategy.
4. Validation rules.
5. Initial test suite for ordering, timestamps, and schema behavior.

Exit criteria:

1. Recordings can be represented without any Windows-specific types.
2. Import, export, and validation contracts are stable enough for later backends.

### Phase 2 - SDK Surface And Import Export

Deliverables:

1. Public Rust SDK entry points.
2. Canonical JSON import and export.
3. Round-trip and negative-case tests.
4. Initial CLI skeleton.

Exit criteria:

1. A recording can be loaded, validated, transformed, and exported without touching live Windows APIs.
2. Public API naming is coherent enough to support CLI and UI wrappers.

### Phase 3 - Windows Recording And Hotkeys

Deliverables:

1. Message-loop thread.
2. Raw Input recording backend.
3. Hook fallback backends.
4. RegisterHotKey integration.
5. Diagnostics for registration failures.
6. CLI smoke path for manual global hotkey validation.

Exit criteria:

1. Live capture works for standard desktop apps.
2. Hotkeys work while the app is not focused and can be validated from the CLI.
3. High-frequency mouse input does not drop events in normal conditions.

### Phase 4 - Playback Engine

Deliverables:

1. SendInput playback backend.
2. High-resolution scheduler.
3. Speed scaling.
4. Completion, abort, and cleanup behavior.
5. Drift metrics.

Exit criteria:

1. Playback preserves event order.
2. Playback speed control behaves predictably.
3. Completion and abort paths do not leave stuck key or button state behind.

### Phase 5 - CLI And Integration Hardening

Deliverables:

1. Working CLI commands.
2. Diagnostics and doctor-style checks for environment, recording, playback, and control-loop configuration.
3. SDK and CLI integration tests that exercise the public CLI binary rather than private implementation helpers.
4. Explicit opt-in elevated-run support for playback-capable commands when targeting elevated apps.
5. Optional global-hotkey start and stop controls for CLI record and play workflows.

Current status:

1. The CLI has `doctor` commands for `environment`, `recording`, `playback`, and `control` with machine-readable JSON output and doctor-style non-zero exits when checks fail.
2. The CLI also has `diagnostics` commands for the same targets with deeper inspection output that does not treat failing checks as command failure by itself.
3. Environment diagnostics currently probe RegisterHotKey availability, session-directory state, and current-process elevation so permission guidance is more actionable.
4. The `play` and `control` commands now accept `--elevate` so they can relaunch themselves as administrator before playback begins or playback-capable hotkeys are armed.
5. CLI coverage now includes public-binary tests for diagnostics behavior plus focused unit coverage for `--elevate` parsing and Windows relaunch argument quoting.

Exit criteria:

1. The CLI can drive the core workflows without private hooks into the implementation.
2. Capability and permission failures are actionable.
3. Recording and playback can be started or stopped by configured global hotkeys when desired.
4. Playback-capable CLI workflows can relaunch elevated before dispatch begins when the operator requests it.

### Phase 6 - Slint Desktop UI

Deliverables:

1. Recording library management UI.
2. Playback controls.
3. Import and export UI.
4. Basic event editing.
5. Tray and minimized workflow.

Current status:

1. `regxorder-ui` now compiles into a Slint shell with a compact toolbar, left-side navigation, and dedicated editor, diagnostics, and settings views instead of a single text-heavy dashboard.
2. The editor view discovers validated session files from the shared `sessions/` directory, supports direct selection through a compact dropdown, surfaces invalid JSON files only when needed, filters large event lists in Rust before rendering, and hides pointer-move rows by default for responsiveness.
3. The controls flow can start a new recording, stop the active record or playback action, auto-save new captures into `sessions/`, play the selected session through the shared controller path, and delete individual visible events directly from the list with immediate persistence back to the same session file.
4. The settings view now exposes real UI-side configuration for recording strategy, playback speed selection, and a shared stop-action hotkey binding that is passed through to both recording and playback requests.
5. The Slint UI has started moving toward a modular structure with extracted shared components and updated Copilot instructions for splitting future UI work across feature-specific files instead of continuing to grow a single monolithic window file.
6. Import/export, richer event authoring beyond row deletion, tray/minimized workflow, and explicit UI-side elevation handling are still pending in Phase 6.

Exit criteria:

1. Core workflows are usable without the CLI.
2. Large recordings remain responsive.
3. UI terminology and behavior match the SDK and CLI.

### Phase 7 - Packaging, Reliability, And Release Readiness

Deliverables:

1. Logging and diagnostics.
2. Packaging and installer behavior.
3. Clear unsupported-scenarios documentation.
4. Manual acceptance checklist.

Exit criteria:

1. Elevated-target guidance is clear.
2. Unsupported scenarios are documented plainly.
3. The acceptance checklist is complete for the release candidate.

## Acceptance Checklist For V1

1. Recording works with precise relative timing for keyboard and mouse.
2. Playback preserves event order and respects speed scaling.
3. Global hotkeys work outside the focused app.
4. Recordings can be imported, exported, validated, and edited as JSON.
5. The SDK exposes the core workflows cleanly.
6. The CLI exercises the SDK rather than bypassing it.
7. The Slint UI covers the essential workflows without performance issues on normal recording sizes.
8. Cleanup logic prevents stuck modifier or button state after session end, abort, or failure.
9. Elevated targets are supported when regxorder itself is elevated.
10. Unsupported scenarios are documented clearly.

## Risks And Mitigations

| Risk | Why it matters | Mitigation |
| --- | --- | --- |
| Integrity boundaries and UIPI | Playback may fail silently or partially across privilege boundaries | Detect and surface elevation limitations explicitly, and provide an opt-in elevated relaunch path before playback begins |
| High-frequency mouse input | Event loss or jitter can compromise fidelity | Prefer Raw Input and test buffered handling under load |
| Hook callback overhead | Slow callbacks can drop hooks or degrade input handling | Keep callbacks minimal and offload work immediately |
| Coordinate portability | Exact coordinates are machine-specific | Store normalized coordinates and monitor metadata alongside exact values |
| Stuck input state | Recording or playback can end with keys or buttons still pressed | Track pressed state centrally, reconcile recordings to a fully released terminal state, and release playback state on completion, abort, or failure |
| Schema drift | External editing and API use will magnify breaking changes | Version the schema and add migration tests |
| Overcoupled UI logic | Business rules may become duplicated or harder to test | Keep UI thin and route behavior through shared services |

## Documentation Maintenance Rules

1. Update this file when architecture, scope, or testing expectations change.
2. Keep `.github/copilot-instructions.md` aligned with this document.
3. When a phase is completed or materially changed, update the relevant phase section and acceptance criteria.
4. When a decision is reversed, remove stale guidance rather than layering conflicting notes on top.

## Immediate Next Steps

1. Create the Cargo workspace and crate skeleton.
2. Implement the core event model, schema versioning, and validation rules first.
3. Add the first wave of unit tests around event ordering, timing, and JSON round-trips before integrating live Windows APIs.
4. Add the Windows message-loop infrastructure only after the core contracts are stable.
