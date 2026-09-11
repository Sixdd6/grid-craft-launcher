# Agents follow these rules

- Read `docs/SPEC.md` only when implementing a feature or changing user-facing behavior. Cite the requirement id in the commit message.
- Read `ARCHITECTURE.md` to find which module owns a behavior before you add code. Update its module table when you add a module.
- Read the skill for the domain before touching it: `.claude/skills/<name>/SKILL.md`. The skill names its domain in its description.
- API facts come from `docs/research/` and the skills. Items marked VERIFY need a live check by the api-verifier agent before a parser depends on them.
- Unit tests never call the network. Use fixtures under `tests/fixtures/` with wiremock.
- Never print or log `CURSEFORGE_API_KEY`, `GCL_CURSEFORGE_API_KEY`, `GCL_MSA_CLIENT_ID`, tokens, or refresh tokens.
- Run `just check` before reporting a task done. Paste the last lines of its output in the report.
- Use `thiserror` enums in `gcl-core`, `anyhow` only in binaries. No `unwrap` or `expect` outside tests and build scripts.
- Prose in docs and skills: short sentences, active voice, one word per thing.
