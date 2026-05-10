# Recording Backend Strategies

## Purpose

This document describes the Windows recording strategies that regxorder keeps available in V1. The project should treat each strategy as a reusable backend, not as disposable migration scaffolding. Even when one strategy is the default, the others can still be valuable for fallback behavior, debugging, compatibility testing, or future feature work.

## Current Strategies

| Strategy | CLI name | Current role |
| --- | --- | --- |
| Raw Input | `raw-input` | Preferred default recorder strategy |
| Low-level hooks | `low-level-hooks` | Compatibility and diagnostic fallback |

## Raw Input Strategy

### Raw Input Summary

Raw Input records keyboard and mouse activity through `WM_INPUT` messages delivered to a dedicated message-loop thread and hidden window. It is the preferred V1 recorder strategy because it aligns more closely with the Windows input model intended for robust global device monitoring.

### Raw Input Strengths

1. Better fit for high-frequency mouse input and longer recording sessions.
2. Lower risk of callback-related hook removal under load.
3. More direct access to device-level keyboard and mouse packets.
4. Better long-term foundation for distinguishing strategy-specific capture behavior without changing the canonical recording model.
5. Cleaner path for future device-aware features and richer diagnostics.

### Raw Input Weaknesses

1. More complex implementation because it needs a window, device registration, and `WM_INPUT` parsing.
2. Some mouse packets can contain multiple changes at once, so translation into a strict ordered event stream requires explicit sequencing choices.
3. Movement packets are lower level than cursor messages, so translating them into replay-ready screen positions still requires policy decisions.
4. More Win32 surface area means more places where careful validation and targeted tests are needed.

### Use Raw Input When

1. Recording fidelity and robustness matter more than implementation simplicity.
2. Mouse-heavy automation or longer sessions are expected.
3. Raw device semantics are useful for diagnosing capture behavior.

## Low-Level Hook Strategy

### Low-Level Hooks Summary

Low-level hooks record keyboard and mouse activity through `WH_KEYBOARD_LL` and `WH_MOUSE_LL` callbacks on a dedicated message-loop thread. This strategy is still useful and should remain available even after Raw Input is the default.

### Low-Level Hook Strengths

1. Straightforward way to build a global first-pass recorder.
2. Easy to translate hook messages into the canonical action model.
3. Good diagnostic reference when comparing behavior against Raw Input.
4. Useful fallback if a Raw Input path has an environment-specific problem.

### Low-Level Hook Weaknesses

1. More sensitive to callback latency and hook timeouts.
2. Less attractive for very high-frequency input streams.
3. More dependent on the message-hook pipeline than Raw Input.
4. Not the preferred long-term default when precision and robustness are the main goals.

### Use Low-Level Hooks When

1. You need a compatibility fallback.
2. You want to compare hook behavior against Raw Input during debugging.
3. A simple, message-based capture path is temporarily useful while diagnosing recorder issues.

## Shared Design Rules

1. Every strategy must emit the same canonical `InputEvent` model.
2. Every strategy must preserve strict event ordering through explicit sequence numbers.
3. Every strategy must record elapsed time relative to recording start.
4. Every strategy must capture enough display metadata to support deterministic replay on the same setup.
5. Strategy-specific details should stay inside the Windows backend crate unless they are deliberately promoted to public capability metadata.

## Selection Guidance

1. Default to Raw Input for normal recording work.
2. Keep low-level hooks available as a named strategy and do not delete it just because Raw Input is preferred.
3. When recorder behavior is suspicious, compare the same scenario with both strategies before changing the canonical model or playback behavior.

## CLI Mapping

The current CLI `record` command exposes the strategy selector directly:

```powershell
cargo run -p regxorder-cli -- record --output demo.json --strategy raw-input --duration-seconds 5
cargo run -p regxorder-cli -- record --output demo.json --strategy low-level-hooks --duration-seconds 5
```

Relative output filenames are written under the gitignored `sessions/` directory.
