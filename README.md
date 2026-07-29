# jimtime

Track billable time per git repo, review and approve it, and push approved
billable entries to Harvest.

Run `jimtime` from any git repo; it maps the repo to a Harvest client/project
and stores time centrally. See `docs/HANDOFF.md` for the design and
`docs/adr/` for the decisions behind it.

## Configuration

jimtime is configured entirely through the environment - no config file holds
secrets, and none is committed with credentials. [ADR-0003]

Add these to your shell/dotfiles:

```sh
# Where time data lives (the store + the repo->Harvest mapping).
# If unset, falls back to the XDG data dir (~/.local/share/jimtime).
export JIMTIME_HOME="$HOME/code/jimnist/bigbrain/time_tracking"

# Harvest credentials (secrets - keep these out of any repo).
# Create a Personal Access Token at https://id.getharvest.com/developers
export HARVEST_ACCESS_TOKEN="..."
export HARVEST_ACCOUNT_ID="..."
# Optional; defaults to "jimtime (jim@jimnist.com)"
# export HARVEST_USER_AGENT="jimtime (you@example.com)"
```

The repo→Harvest mapping is non-secret and lives at
`$JIMTIME_HOME/config/harvest-projects.json` (committed). Build it from real
Harvest data:

```sh
jimtime harvest projects           # find the client_id + project_id
jimtime harvest tasks --project ID # find a default_task_id assigned to it
```

## Usage

```
$ jimtime --help
< TODO: insert help here >
```

## Async by default

This project is async: `main` is `#[tokio::main]` and ships with `tokio`,
`reqwest`, and `serde_json` for network/IO work.

### Going sync

If this is an offline CLI that never touches the network, strip async out:

1. In `Cargo.toml`, remove `tokio`, `reqwest`, `serde_json` (and `async-trait`
   if present).
2. In `src/main.rs`, drop the `#[tokio::main]` attribute and make `main`
   non-`async`: `fn main() -> Result<()>`.
3. If you have subcommands, in `src/commands/mod.rs` remove
   `#[async_trait::async_trait]` and make the trait method sync
   (`fn run(&self) -> Result<()>`); do the same in each command's `impl`, and
   change `cli.commands.run().await?` back to `cli.commands.run()?`.



## Agent tooling

This repo ships with agent context and skills pre-installed (from the
[`jimnist/rust-cli-template`](https://github.com/jimnist/rust-cli-template)
template):

- `AGENTS.md` + `docs/agents/` — durable, project-specific context for AI agents.
- `.claude/skills/` and `.agents/skills/` — the `grill-with-docs` skill (and its
  deps), plus `setup-matt-pocock-skills`. See `docs/agents/skills.md`.

Run `/grill-with-docs` before building anything non-trivial, and
`/setup-matt-pocock-skills` once to wire up issue tracking and doc locations.

## Install

You can either download the latest release builds from the [Releases page](https://github.com/jimnist/jimtime/releases) or install it using cargo install.
```
cargo install --git https://github.com/jimnist/jimtime
```
