# AGENTS.md

`jimtime` is a Rust command-line application. Track billable time per git repo and push approved entries to Harvest

Before making changes, read the relevant docs under `docs/agents/`.

## Agent docs

Start here:

- `docs/agents/README.md` — what lives where
- `docs/agents/systems.md` — external systems this app talks to
- `docs/agents/skills.md` — installed skills and how to add more

## Working rules

- Run `/grill-with-docs` before building anything non-trivial — align first,
  then write the decisions down (ADRs in `docs/adr/`, terms in `CONTEXT.md`).
- Prefer small, reviewable changes. Keep the CLI surface stable unless a change
  is deliberate and documented.
- This app is **async by default** (`tokio`). If it never touches the network,
  see "Going sync" in the README before adding sync-only assumptions.
- Never commit secrets, tokens, or credentials to code or docs.
