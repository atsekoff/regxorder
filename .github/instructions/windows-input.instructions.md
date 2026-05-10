---
description: "Use when implementing Raw Input, SendInput, RegisterHotKey, low-level hooks, message loops, Windows timing, or recorder/player backend logic for regxorder. Covers backend priorities, cleanup rules, and V1 support boundaries."
name: "Regxorder Windows Input"
---
# Regxorder Windows Input Guidelines

- V1 backend priority is Raw Input for recording, low-level hooks as fallback, SendInput for playback, and RegisterHotKey for global hotkeys.
- Keep recording and playback strategies reusable. A lower-priority backend should remain available as a named strategy when it is still useful for diagnostics, compatibility, or fallback behavior.
- Do not introduce anti-cheat support, stealth behavior, or kernel-driver work into V1 planning or implementation.
- Keep input capture and hotkey registration on a dedicated message-loop thread and move expensive work off callback paths immediately.
- Record both exact coordinates and normalized coordinates, plus enough monitor and DPI metadata to validate replay assumptions.
- Track pressed keys and buttons centrally so abort and failure paths can release them reliably.
- Measure or log expected versus actual playback timing where scheduler precision matters.
- Handle integrity-level and UIPI failures explicitly and return actionable errors rather than silent best-effort behavior.
- Favor small, testable wrappers around Win32 APIs so most behavior can be exercised without a live desktop environment.
- Document the strengths and weaknesses of each backend strategy where the project or architecture docs discuss them.
