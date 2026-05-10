---
description: "Use when updating architecture docs, plans, decision logs, or implementation notes for regxorder. Covers keeping project scope, testing expectations, and design decisions synchronized."
name: "Regxorder Docs"
applyTo: "docs/**/*.md"
---
# Regxorder Documentation Guidelines

- Keep `docs/implementation-plan.md` aligned with the actual architecture, scope, and phase ordering.
- When requirements or design decisions change, update the relevant documentation and instructions in the same change.
- Prefer concise headings, direct language, and checklists or tables when they improve scanning.
- Document V1 scope boundaries clearly, especially unsupported targets and determinism limits.
- Keep testing expectations visible in the docs because tests are treated as executable documentation in this project.
