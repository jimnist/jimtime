# AGENTS.md

`jimtime` is a Rust command-line application. Track billable time per git repo and push approved entries to Harvest

Before making changes, read the relevant docs under `docs/agents/`.

## Agent docs

Start here:

- `docs/agents/README.md` - what lives where
- `docs/agents/systems.md` - external systems this app talks to
- `docs/agents/skills.md` - the `/jimtime` Claude Code skill this repo ships
- `docs/HANDOFF.md` - the build spec; `CONTEXT.md` - the glossary; `docs/adr/` - decisions

## Working rules

- Align before building anything non-trivial: settle the design, then write the
  decisions down (ADRs in `docs/adr/`, terms in `CONTEXT.md`).
- Prefer small, reviewable changes. Keep the CLI surface stable unless a change
  is deliberate and documented.
- This app is **async by default** (`tokio`) because it talks to the Harvest API.
- Never commit secrets, tokens, or credentials to code or docs. [ADR-0003]
