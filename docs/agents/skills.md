# Skills

This repo ships one Claude Code skill: **`/jimtime`**, at
`.claude/skills/jimtime/SKILL.md`.

It is a thin wrapper around the CLI - it teaches Claude Code to summarize the
user's work into a conservative `jimtime add`, help with `review`, and run
`approve` / `harvest push` only on an explicit instruction. The CLI owns all the
real logic (store, mapping, approval, dedup, Harvest push); the skill just drives
it.

It is `disable-model-invocation: true`, so Claude never triggers it on its own -
the user invokes it with `/jimtime`.

## Installing it globally

The skill lives in this repo so it ships and versions with the tool. To make it
invokable from any repo, symlink it onto your Claude skills path:

```sh
ln -s "$PWD/.claude/skills/jimtime" ~/.claude/skills/jimtime
```

(The template originally installed a set of generic Matt Pocock authoring skills
here; those were removed - they belong in your global agent config, not shipped
with this tool.)
