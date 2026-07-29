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
Track billable time per git repo and push approved entries to Harvest

Commands:
  status     Show the current repo, its Harvest mapping, and today's store path
  map        Show the Harvest mapping for the current repo
  add        Add a time entry for the current repo
  today      Print today's time log
  review     List entries over a date range or a single day
  approve    Approve unapproved entries (the human gate before pushing)
  unapprove  Set matching entries back to unapproved
  report     Export a markdown time report over a date range or a single day
  harvest    Query Harvest, and dry-run or push approved time entries
```

### The workflow

```sh
# From the repo you did the work in:
jimtime add --hours 1.25 --notes "Implemented invoice sync" [--needs-review]

# See what's outstanding (lists each entry with its ID + status):
jimtime review --week --pending

# Approve — per entry; sweeps everything not yet approved…
jimtime approve --week                 # …except needs-review entries (held)
jimtime approve --week --except <id>   # …and hold specific ones
jimtime approve --week --include-needs-review
jimtime approve --only <id>            # …or approve just these (bypasses the hold)

# Push approved, billable, not-yet-imported entries to Harvest:
jimtime harvest dry-run --week         # preview the exact payload
jimtime harvest push --week            # creates the entries; skips ones already imported

# Export a markdown report to paste/share:
jimtime report --week
```

Ranges are shared across `review`, `approve`, `unapprove`, `report`, and
`harvest`: `--today`, `--week`, `--last-week`, `--date YYYY-MM-DD`, or
`--from … --to …` (default: today). Approval is per entry - `review` shows
`●` unapproved / `○` approved; if an entry is wrong, edit the day's JSON under
`$JIMTIME_HOME/entries/` directly. Push defaults to billable rows only; pass
`--include-non-billable` to include the rest.

## The Claude Code skill

The repo ships a user-invoked `/jimtime` skill at `.claude/skills/jimtime/` that
lets Claude Code drive the workflow: it summarizes your work into a conservative
entry (`add --needs-review`), helps you `review`, and runs `approve` / `push`
only on your explicit say-so. It is `disable-model-invocation: true`, so it never
fires on its own - you invoke it with `/jimtime`.

To use it from any repo, symlink it onto your Claude skills path, e.g.:

```sh
ln -s "$PWD/.claude/skills/jimtime" ~/.claude/skills/jimtime
```

## Agent context

`AGENTS.md` and `docs/agents/` hold durable, project-specific context for AI
agents; `CONTEXT.md` is the domain glossary and `docs/adr/` the load-bearing
decisions.

## Install

You can either download the latest release builds from the [Releases page](https://github.com/jimnist/jimtime/releases) or install it using cargo install.
```
cargo install --git https://github.com/jimnist/jimtime
```
