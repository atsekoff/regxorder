---
description: "Use when writing Rust files in crates/regxorder-ui. Covers Slint view-model organization, callback handlers, state sync, filtering, and hotkey/settings validation."
name: "Regxorder UI Rust"
applyTo: "crates/regxorder-ui/src/**/*.rs"
---
# Regxorder UI Rust Guidelines

- Keep `regxorder-ui` Rust code modular. Favor feature modules or handler/state helpers over continuing to grow `lib.rs` as a single orchestration file.
- Keep business logic, validation, filtering, list shaping, and hotkey parsing in Rust. Slint files should stay declarative and focus on layout, styling, and callback emission.
- Prefer a clear state flow: domain or controller state -> UI display state -> `apply_to()` or equivalent window sync.
- Use callbacks as the boundary from Slint into Rust. Avoid implicit two-way coupling for complex state.
- Treat display models as UI-specific. Format labels, summaries, and row text in dedicated Rust helpers rather than encoding domain rules in `.slint`.
- For large lists, compute filtered or consolidated row models in Rust and keep per-row Slint bindings cheap.
- Settings and hotkey editors should validate in Rust and return clear user-facing messages instead of relying on ad hoc UI-side parsing.
- Add or update focused unit tests for state transitions, callback handlers, filtering, and validation whenever UI behavior changes.
