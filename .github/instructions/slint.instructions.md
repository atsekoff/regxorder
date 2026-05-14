---
description: "Use when writing Slint UI files, view models, or UI interaction code for regxorder. Covers functional, performant desktop UI design for recording, playback, editing, and tray workflows."
name: "Regxorder Slint UI"
applyTo: "**/*.slint"
---
# Regxorder Slint UI Guidelines

- Favor clarity, responsiveness, and keyboard-friendly workflows over decorative UI.
- Keep layouts dense but readable. Recording lists, playback controls, hotkey settings, diagnostics, and event editing workflows should take priority.
- Prefer reusable components and straightforward data flow between Slint views and Rust view models.
- Split large UI surfaces into feature view files and shared components. Avoid growing one monolithic `.slint` file when a view or control can be imported instead.
- Keep one component or one closely related view group per file when practical, and use relative imports to compose the final window.
- Design for large recordings. Lists, inspectors, and editors should avoid heavy per-row computation and chatty bindings.
- Use Slint for presentation and interaction wiring, not domain logic. Validation, hotkey parsing, filtering, sorting, and summary generation should stay in Rust.
- Prefer shared style tokens or theme files for repeated colors, spacing, and component chrome instead of duplicating values across many views.
- Settings and form-style surfaces should keep validation feedback compact and route edits through Rust callbacks.
- Keep animations minimal and purposeful. Nothing in the UI should risk sluggishness during recording or playback monitoring.
- Preserve tray and minimized workflows and make recording, playback, elevation state, and backend limitations obvious.
- Match UI terminology to the SDK and CLI so core actions have one meaning across the product.
