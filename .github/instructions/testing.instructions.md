---
description: "Use when writing or updating unit tests, integration tests, doctests, fixtures, or coverage plans for regxorder. Covers tests-as-documentation, deterministic test design, and Windows/backend testing boundaries."
name: "Regxorder Testing"
---
# Regxorder Testing Guidelines

- Treat tests as executable documentation. A reader should learn expected behavior from the test names, fixtures, and assertions.
- Prefer focused unit tests close to the logic they describe. Use integration tests for crate boundaries, CLI behavior, and selective live Windows flows.
- Keep the core domain heavily covered. Ordering, timing, validation, import/export, cleanup, and coordinate handling should all have direct tests.
- Avoid sleeps and live OS dependencies in unit tests. Use fake clocks, fake backends, and deterministic fixtures whenever possible.
- For Win32 interop, wrap raw APIs in small safe adapters and test behavior around those adapters. Reserve live Windows tests for smoke and integration coverage.
- Add negative-case tests for malformed recordings, unsupported capabilities, UIPI and elevation failures, interrupted playback, and stuck-key cleanup.
- Prefer table-driven tests or compact fixtures when a single rule has multiple cases.
- When adding a public API, consider a doctest or example if it improves discoverability and stays aligned with the implementation.
