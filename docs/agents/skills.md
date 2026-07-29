# Skills

This repo ships with a few agent skills pre-installed from
[`jimnist/skills`](https://github.com/jimnist/skills) (a fork of Matt Pocock's
["Skills For Real Engineers"](https://github.com/mattpocock/skills)), via the
[`jimnist/rust-cli-template`](https://github.com/jimnist/rust-cli-template)
template.

Skills are installed into two locations so different coding agents can find
them:

- `.claude/skills/` — read by Claude Code
- `.agents/skills/` — the agent-agnostic location used by other agents (Codex,
  etc.)

`skills-lock.json` records what's installed and from where.

## Installed

| Skill | Purpose |
|---|---|
| `grill-with-docs` | The headline skill: a relentless interview that sharpens a plan/design **and** writes docs (ADRs + glossary) as you go. Run it before building anything non-trivial. |
| `grilling` | The interview engine `grill-with-docs` drives — asks one hard question at a time until you reach shared understanding. |
| `domain-modeling` | The doc-writing engine `grill-with-docs` uses — builds the glossary (`CONTEXT.md`) and ADRs (`docs/adr/`). |
| `setup-matt-pocock-skills` | "The instructions." Wires up the skills for a repo: picks an issue tracker, triage labels, and where docs live. Run once. |

`grill-with-docs` depends on both `grilling` and `domain-modeling`, so all three
are installed together — installing `grill-with-docs` alone would half-work.

## Adding more skills

Use the official installer (interactive — it lets you pick skills and target
agents):

```bash
npx skills@latest add jimnist/skills
```

Browse what's available in the [`jimnist/skills`](https://github.com/jimnist/skills)
README.

> Note: the entries in `skills-lock.json` here use a plain SHA-256 of each
> `SKILL.md`. The official `skills.sh` tool computes its hash differently, so
> the first time you run `npx skills@latest add jimnist/skills` it may rewrite
> `skills-lock.json` with its own hashes and/or re-sync the skill files. That's
> expected — let it own the lock from then on.
