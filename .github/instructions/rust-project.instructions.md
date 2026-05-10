---
description: "Use when writing Rust code, backend traits, public APIs, scheduling logic, serialization, or Win32 adapters for regxorder. Covers library-first architecture, FFI isolation, deterministic behavior, and test expectations."
name: "Regxorder Rust"
applyTo: "**/*.rs"
---
# Regxorder Rust Guidelines

- Keep `regxorder-core` platform-agnostic. No Win32 handles, message types, or unsafe code should leak into the core crate.
- Isolate unsafe Windows calls inside `regxorder-win32` behind thin safe wrappers that translate raw failures into typed domain errors.
- Treat recording, playback, hotkeys, and timing as explicit backends or services. Prefer trait-based seams and capability checks over ad hoc branching.
- Prefer verbose, unambiguous names over short or generic names. If a name could plausibly mean more than one thing, rename it to make the meaning explicit.
- Preserve deterministic semantics: stable event ordering, relative timestamps, explicit speed scaling, cleanup on abort, and drift reporting where timing matters.
- Prefer strong domain types for coordinates, timestamps, hotkeys, speed multipliers, and capability flags instead of passing raw primitives through the system.
- Keep the CLI and Slint UI thin. Shared behavior belongs in the SDK or shared application services.
- Avoid hidden global state, implicit singleton services, and long-lived background threads without explicit ownership and shutdown behavior.
- Add or update unit tests with behavior changes. Tests should document the behavior as clearly as the code does.
- When adding public APIs, consider doctests or examples if they improve discoverability without duplicating existing tests.
- Before committing Rust changes, run `cargo fmt --all` and stage the formatted result.
- If save-time or non-Rust formatters also rewrote staged files, restage those files before committing.
- Keep the repository pre-commit hook and any Copilot commit guard logic aligned so both enforce the same commit-readiness checks.
