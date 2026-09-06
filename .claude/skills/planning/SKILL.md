---
name: planning
description: Standards for implementation plans in GRID Craft Launcher. Use when creating, reviewing, or executing a plan for a feature, fix, or refactor.
---

## When to plan

Plan when any of these is true: more than one file changes, the owning module is not obvious,
a new module or trait is added, or launch or download behavior changes.

Skip planning for one-file bug fixes, test-only changes, and copy edits.

## Read before planning

1. `ARCHITECTURE.md` for the owning module.
2. `docs/SPEC.md` for feature work. Cite requirement ids.
3. The domain skill for the area.
4. `docs/research/` when the skill marks something VERIFY.

## Plan format

### Summary
Two to four sentences. State the outcome.

### Files
| File | Change | Reason |
|---|---|---|

### Steps
Numbered. Each step names the agent (coder, slint-designer, doc-research), the files it may
touch, the test it adds or runs, and the command that proves it done.

### Verification
Which verifier runs at the end: api-verifier, e2e-runner, or both, per CLAUDE.md rules.

### Risks
What could break and how the plan checks it.

## Rules

- Steps that touch the same file are sequential.
- Every step has a test or a command with expected output.
- No step says "handle errors" or "add tests" without saying which.
