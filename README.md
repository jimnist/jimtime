# jimtime

[![CI](https://github.com/jimnist/jimtime/actions/workflows/ci.yml/badge.svg)](https://github.com/jimnist/jimtime/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/jimnist/jimtime?include_prereleases&sort=semver)](https://github.com/jimnist/jimtime/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Track billable time per git repo, review and approve it, then push the approved entries to [Harvest](https://www.getharvest.com/).

Run `jimtime` from inside any git repo.
It maps that repo to a Harvest client, project, and task, and appends the entry to a central, human-readable store.
Nothing reaches Harvest until you approve it.

```sh
$ cd ~/code/client/acme
$ jimtime add --hours 1.25 --notes "Implemented webhook retry handling"
$ jimtime review --week --pending
$ jimtime approve --week
$ jimtime harvest push --week
```

## Why

Timers get forgotten and web forms get skipped.
The repo you are working in already knows which client you are billing, so `jimtime` uses that as the key and keeps the friction down to one command at the end of a chunk of work.

Three properties make it safe to point at a real invoice:

- **The store is the source of truth.**
  Time lives as one JSON file per day, diffable and committable to a private git repo.
  Harvest is a downstream destination, not the record. [[ADR-0001](docs/adr/0001-structured-store-is-source-of-truth.md)]
- **Approval is an explicit, per-entry human gate.**
  Nothing is auto-approved, and only approved, billable, not-yet-imported entries are eligible to push. [[ADR-0004](docs/adr/0004-per-entry-approval.md)]
- **Pushes are deduplicated.**
  Each entry records the Harvest id it created, so re-running a push never double-bills.

## Install

Download a prebuilt binary from the [Releases page](https://github.com/jimnist/jimtime/releases), or build from source:

```sh
cargo install --git https://github.com/jimnist/jimtime
```

Requires Rust 1.85 or newer to build.
The binary runs in the caller's working directory, so no shell alias or wrapper is needed.

## Setup

### 1. Environment

`jimtime` is configured entirely through the environment.
No config file holds secrets, and no credentials are ever written to disk. [[ADR-0003](docs/adr/0003-credentials-from-environment-only.md)]

Add these to your shell profile:

```sh
# Where time data lives: the store plus the repo-to-Harvest mapping.
# If unset, falls back to the XDG data dir (~/.local/share/jimtime).
export JIMTIME_HOME="$HOME/time-tracking"

# Harvest credentials. Create a Personal Access Token at
# https://id.getharvest.com/developers
export HARVEST_ACCESS_TOKEN="..."
export HARVEST_ACCOUNT_ID="..."

# Optional. The timezone billing days are anchored to, as an IANA name.
# Defaults to America/Los_Angeles.
# export JIMTIME_TZ="Europe/Berlin"

# Optional. Harvest asks that API clients identify themselves.
# export HARVEST_USER_AGENT="jimtime (you@example.com)"
```

Pointing `JIMTIME_HOME` at a private git repo is the recommended setup: your billing record then has a full history.

### 2. Map your repos to Harvest

The mapping is non-secret and lives at `$JIMTIME_HOME/config/harvest-projects.json`.
Look up the real ids first:

```sh
jimtime harvest clients              # client ids
jimtime harvest projects             # project ids and their client
jimtime harvest tasks --project ID   # tasks assigned to a project
```

Then write the file:

```json
{
  "repos": [
    {
      "repo_path": "/Users/you/code/client/acme",
      "client_id": 123,
      "client_name": "Acme",
      "project_id": 234,
      "project_name": "Billing Portal",
      "default_task_id": 345,
      "default_task_name": "Development",
      "billable": true
    }
  ],
  "aliases": {
    "meetings": { "task_id": 346, "task_name": "Meetings" }
  }
}
```

`repo_path` is the repo's `git rev-parse --show-toplevel`, compared canonically.
One repo maps to one client and project.
`aliases` are reusable shorthands for non-default tasks, used as `jimtime add --task meetings`.

Confirm it resolved:

```
$ jimtime status
Current repo:     /Users/you/code/client/acme
Mapped client:    Acme
Mapped project:   Billing Portal
Default task:     Development
Billable default: yes
Billing timezone: America/Los_Angeles
Today's store:    /Users/you/time-tracking/entries/2026/08/2026-08-17.json
```

## The workflow

### Log

Run from the repo the work happened in:

```sh
jimtime add --hours 1.25 --notes "Implemented webhook retry handling"
jimtime add --from 14:00 --to 15:30 --notes "Reviewed the invoice sync PR" --needs-review
jimtime add --hours 0.5 --notes "Weekly sync" --task meetings
```

Use `--hours` for decimal hours or `--from`/`--to` for real clock times.
`--needs-review` marks an entry as an estimate, which holds it back from bulk approval until you look at it.
`--date YYYY-MM-DD` backfills an earlier day, and `--billable no` overrides the mapping's default.

### Review

```
$ jimtime review --today
Review: 2026-08-17

Acme - Billing Portal - Development
  2026-08-17-acme-billing-portal-development-001
    ●   1.25h  billable  Implemented webhook retry handling
  2026-08-17-acme-billing-portal-development-002
    ●   1.50h  billable  Reviewed the invoice sync PR  [needs review]
  Total: 2.75h · 2 unapproved · 1 needs-review · 0 eligible to push

Acme - Billing Portal - Meetings
  2026-08-17-acme-billing-portal-meetings-001
    ●   0.50h  billable  Weekly sync
  Total: 0.50h · 1 unapproved · 0 needs-review · 0 eligible to push

Totals: 3.25h (3.25h billable) · 0 eligible to push
```

`●` is unapproved and `○` is approved.
Add `--pending` to list only what is outstanding.
Each entry prints its stable id, which is what `approve --only` and `--except` take.

### Approve

Approval is the human gate, and it is per entry.
`approve` sweeps everything unapproved in range except entries flagged `needs-review`, which it holds and names:

```
$ jimtime approve --today
Approved 2 entries:
  2026-08-17  1.25h  Acme - Billing Portal - Development  (2026-08-17-acme-billing-portal-development-001)
  2026-08-17  0.50h  Acme - Billing Portal - Meetings  (2026-08-17-acme-billing-portal-meetings-001)

Held 1 entry flagged needs-review (approve with --include-needs-review, or --only <id>):
  2026-08-17  1.50h  Acme - Billing Portal - Development  (2026-08-17-acme-billing-portal-development-002)
```

```sh
jimtime approve --week --except <id>          # sweep, but hold specific entries
jimtime approve --week --include-needs-review # sweep the flagged ones too
jimtime approve --only <id>                   # approve just these, bypassing the hold
jimtime unapprove --only <id>                 # take one back
```

`unapprove` mirrors `approve`: it sweeps the range, honors `--except`, and takes `--only` to act on exactly the ids you name.
It skips entries already pushed to Harvest, since those cannot be un-billed from here.
Both commands also narrow by `--client`, `--project`, or `--repo`.

### Push

Always dry-run first.
It makes no API calls and needs no credentials:

```
$ jimtime harvest dry-run --today
Dry run: Harvest import - 2026-08-17

2026-08-17  1.25h  Acme - Billing Portal - Development
  id: 2026-08-17-acme-billing-portal-development-001
  notes: Implemented webhook retry handling
2026-08-17  0.50h  Acme - Billing Portal - Meetings
  id: 2026-08-17-acme-billing-portal-meetings-001
  notes: Weekly sync

Total eligible: 2 entries, 1.75h
No entries were pushed.
```

```sh
jimtime harvest push --week   # creates real entries in Harvest
```

Push sends approved, billable, not-yet-imported entries and saves each returned Harvest id back to the store immediately, so a re-run skips them.
Pass `--include-non-billable` to send the rest.
Hours are pushed exactly as stored, with no rounding.

### Report

`jimtime report --week` writes a markdown table you can paste into an invoice or a status update, grouped by client, project, and task, with subtotals and a grand total.
Add `--billable-only` to drop the rest.

### What is still unbilled

`jimtime harvest uninvoiced` asks Harvest what it is owed: one line per client, the tracked hours and their money value, largest balance first.

```
$ jimtime harvest uninvoiced
Uninvoiced - 2026-06-28 through 2026-08-28

CLIENT                                   HOURS            USD
Acme                                     42.25       6,337.50
Globex                                   12.00       1,800.00
-------------------------------------------------------------
Total (2 clients)                        54.25       8,137.50
```

The amounts are Harvest's own, computed from the rates set there, and cover time that is tracked but not yet on an invoice.
It defaults to the last two months, which covers the current and previous billing month; `--from` reaches further back, and since the Harvest report accepts at most a year per request, wider ranges are split across requests automatically.
Uninvoiced expenses are reported separately at the bottom, or folded into each client's total with `--with-expenses`.

### Date ranges

`review`, `approve`, `unapprove`, `report`, and `harvest` all take the same range flags:

| Flag | Range |
|---|---|
| `--today` | Today (the default) |
| `--week` | The current Monday to Sunday week |
| `--last-week` | The previous Monday to Sunday week |
| `--date YYYY-MM-DD` | A single day |
| `--from A --to B` | An inclusive range |

Days are anchored to one timezone regardless of the machine's clock, so "today" stays stable when travelling.
Set `JIMTIME_TZ` to any IANA name to choose it; it defaults to `America/Los_Angeles`.

## Commands

| Command | What it does |
|---|---|
| `status` | Show the current repo, its Harvest mapping, and today's store path |
| `map` | Show the Harvest mapping for the current repo |
| `add` | Add a time entry for the current repo |
| `today` | Print today's time log |
| `review` | List entries over a date range or a single day |
| `approve` | Approve unapproved entries, the human gate before pushing |
| `unapprove` | Set matching entries back to unapproved |
| `report` | Export a markdown time report over a date range or a single day |
| `harvest` | Query Harvest, show uninvoiced balances, and dry-run or push approved time entries |

Run `jimtime <command> --help` for the full flag list.

## How time is stored

One JSON file per day at `$JIMTIME_HOME/entries/YYYY/MM/YYYY-MM-DD.json`, the only persisted artifact. [[ADR-0002](docs/adr/0002-per-day-json-store.md)]

```json
{
  "date": "2026-08-17",
  "sections": [
    {
      "repo_path": "/Users/you/code/client/acme",
      "client_id": 123, "client_name": "Acme",
      "project_id": 234, "project_name": "Billing Portal",
      "task_id": 345, "task_name": "Development",
      "entries": [
        {
          "id": "2026-08-17-acme-billing-portal-development-001",
          "hours": 1.25, "billable": true, "approved": false, "needs_review": true,
          "notes": "Implemented webhook retry handling",
          "harvest_time_entry_id": null
        }
      ]
    }
  ]
}
```

A section groups entries that share a repo, client, project, and task.
It is a storage and display grouping only; approval lives on the entry.
Terminal output and markdown reports are rendered on demand and never persisted.

If an entry is wrong, fix it through the CLI or edit the day's JSON directly.
It is plain, stable, and meant to be read.

## The Claude Code skill

This repo ships a `/jimtime` skill at [`.claude/skills/jimtime/`](.claude/skills/jimtime/SKILL.md) that lets [Claude Code](https://claude.com/claude-code) drive the workflow.
It summarizes the work from your session into a conservative entry via `add --needs-review`, helps you review, and runs `approve` or `push` only on your explicit instruction.

It is marked `disable-model-invocation: true`, so it never fires on its own.
You invoke it by typing `/jimtime`.

To use it from any repo, symlink it onto your Claude skills path:

```sh
ln -s "$PWD/.claude/skills/jimtime" ~/.claude/skills/jimtime
```

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs all three on every push and pull request.
Tagging `vX.Y.Z` builds cross-platform binaries and opens a draft release with notes generated by [git-cliff](https://git-cliff.org/).

`.rustfmt.toml` sets a few nightly-only options.
Stable `cargo fmt` warns about them and ignores them, which is expected.

## Project docs

| File | What it covers |
|---|---|
| [`CONTEXT.md`](CONTEXT.md) | The domain glossary: entry, store, view, mapping, section, approval |
| [`docs/adr/`](docs/adr/) | The load-bearing decisions and why they were made |
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | The build spec and data model |
| [`AGENTS.md`](AGENTS.md), [`docs/agents/`](docs/agents/) | Durable project context for AI agents |

## License

MIT. See [LICENSE](LICENSE).
