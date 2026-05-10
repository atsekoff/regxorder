---
description: "Use when writing Slint UI files, view models, or UI interaction code for regxorder. Covers functional, performant desktop UI design for recording, playback, editing, and tray workflows."
name: "Regxorder Slint UI"
applyTo: "**/*.slint"
---
# Regxorder Slint UI Guidelines

- Favor clarity, responsiveness, and keyboard-friendly workflows over decorative UI.
- Keep layouts dense but readable. Recording lists, playback controls, hotkey settings, diagnostics, and event editing workflows should take priority.
- Prefer reusable components and straightforward data flow between Slint views and Rust view models.
- Design for large recordings. Lists, inspectors, and editors should avoid heavy per-row computation and chatty bindings.
- Keep animations minimal and purposeful. Nothing in the UI should risk sluggishness during recording or playback monitoring.
- Preserve tray and minimized workflows and make recording, playback, elevation state, and backend limitations obvious.
- Match UI terminology to the SDK and CLI so core actions have one meaning across the product.
