![jimtime](img/jimtime.jpg)

[![CI](https://github.com/jimnist/jimtime/actions/workflows/ci.yml/badge.svg)](https://github.com/jimnist/jimtime/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/jimnist/jimtime?include_prereleases&sort=semver)](https://github.com/jimnist/jimtime/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Track billable time per git repo, review and approve it, then invoice it: a PDF from your own HTML template, emailed to the client after you have looked at it.
Pushing to [Harvest](https://www.getharvest.com/) is there too, optional and off by default.

Run `jimtime` from inside any git repo.
It maps that repo to a client and project, and appends the entry to a central, human-readable store that is its own private git repo, committed and pushed on every change.
Nothing is billed until you approve it, and no invoice is sent until you have seen it.

```sh
$ cd ~/code/client/acme
$ jimtime add --hours 1.25 --notes "Implemented webhook retry handling"
$ jimtime review --month --pending
$ jimtime approve --month
$ jimtime invoice draft --client acme --last-month
$ jimtime invoice finalize --client acme --last-month
```

## Why

Timers get forgotten and web forms get skipped.
The repo you are working in already knows which client you are billing, so `jimtime` uses that as the key and keeps the friction down to one command at the end of a chunk of work.

A few properties make it safe to point at a real invoice:

- **The store is the source of truth.**
  Time lives as one JSON file per day, diffable, in a private git repo that every change is committed and pushed to. [[ADR-0001](docs/adr/0001-structured-store-is-source-of-truth.md), [ADR-0009](docs/adr/0009-data-home-is-an-auto-synced-git-repo.md)]
- **Approval is an explicit, per-entry human gate.**
  Nothing is auto-approved, and only approved, billable time can be invoiced or pushed. [[ADR-0004](docs/adr/0004-per-entry-approval.md)]
- **What you approve is what is sent.**
  An invoice is drafted, looked at, and only then finalized; a fingerprint makes sure nothing changed in between. [[ADR-0008](docs/adr/0008-invoice-lifecycle.md)]
- **Nothing is billed twice.**
  An invoiced entry is locked to its invoice number, and a pushed one records its Harvest id, so re-running either can never double-bill.

## Install

Build from source. It works on any platform Rust supports:

```sh
cargo install --git https://github.com/jimnist/jimtime
```

Requires Rust 1.85 or newer.
There is nothing to install beyond that: TLS uses rustls, so there is no OpenSSL or other system library to hunt down first.
The binary runs in the caller's working directory, so no shell alias or wrapper is needed.
If you later change the code, the installed binary does not update itself.
See [Running your changes](#running-your-changes).

Rendering invoice PDFs needs Chrome, Chromium, Brave or Edge installed; jimtime prints with it headlessly. [[ADR-0007](docs/adr/0007-local-invoices-from-html-templates.md)]

Prebuilt binaries on the [Releases page](https://github.com/jimnist/jimtime/releases) are **macOS only** (Apple Silicon and Intel).
This is a personal tool and that is the platform it is used on, so building binaries for targets nobody downloads is not worth the CI time.
On any other platform, `cargo install` above is the supported path.

This tool is also opinionated about a workflow that happens to be mine.
If yours differs, the intended use is to fork it and make it yours: the domain language is written down in [`CONTEXT.md`](CONTEXT.md), and the decisions worth arguing with are in [`docs/adr/`](docs/adr/).
Adding your own platform back to the release matrix is a few lines in [`.github/workflows/releases.yml`](.github/workflows/releases.yml).

## Setup

### 1. Environment

Secrets come only from the environment (or, for cloud logins, the OS keychain), never from a file. [[ADR-0003](docs/adr/0003-credentials-from-environment-only.md)]
Add what you use to your shell profile:

```sh
# Where your data lives: the store, the config, and invoices.
# If unset, falls back to the XDG data dir (~/.local/share/jimtime).
export JIMTIME_HOME="$HOME/code/jimtime-data"

# Emailing invoices: your SMTP (app) password.
export JIMTIME_SMTP_PASSWORD="..."

# Optional. The timezone billing days are anchored to, as an IANA name.
# Defaults to America/Los_Angeles.
# export JIMTIME_TZ="Europe/Berlin"

# Only when using Harvest (harvest.toml). Create a token at https://id.getharvest.com/developers
# export HARVEST_ACCESS_TOKEN="..."
# export HARVEST_ACCOUNT_ID="..."

# Only with Google Drive uploads. See docs/agents/systems/google-drive.md
# export JIMTIME_GDRIVE_CLIENT_SECRET="..."
```

### 2. The data repo

Make `JIMTIME_HOME` its own private git repo and jimtime keeps it committed and pushed:

```sh
# create an empty private repo (e.g. jimtime-data) on your git host, then:
jimtime data init --remote git@github.com:you/jimtime-data.git
```

On an empty `JIMTIME_HOME` this clones the repo; on one with data it makes it a repo and pushes.
From then on every command that writes pulls first and commits and pushes after, so a second machine is just `data init --remote` on an empty directory.
Offline is fine: the commit stays local and the next command pushes it.
If two machines change the same day, a merge driver combines them entry by entry, and stops rather than guesses on a real conflict. [[ADR-0009](docs/adr/0009-data-home-is-an-auto-synced-git-repo.md)]
`jimtime data status` shows where it stands.

jimtime will not sync a data home that sits inside some other repo; `data status` prints the `git subtree split` recipe for moving it out with its history.

### 3. Config

Everything else lives in `$JIMTIME_HOME/config/jimtime.toml`.
`jimtime config init` writes a commented starter:

```toml
[business]
name = "Your Name"
email = "you@example.com"
address = """
123 Main St
Portland, OR 97201
"""
payment_instructions = "ACH to ..., or pay at https://..."

[tasks.development]
name = "Development"

[tasks.meetings]
name = "Meetings"

[clients.acme]
name = "Acme Corp"
currency = "USD"
email_to = ["billing@acme.example"]
email_cc = ["controller@acme.example"]   # optional

[clients.acme.projects.website]
name = "Website"
rate = 150.0
task_rates = { meetings = 100.0 }
default_task = "development"

[[repos]]
path = "~/code/acme/website"
client = "acme"
project = "website"

[email]
host = "smtp.fastmail.com"
port = 465               # 587 with security = "starttls"
username = "you@example.com"
from = "Your Name <you@example.com>"
cc = ["books@example.com"]         # optional: Cc on every invoice
bcc = ["you@example.com"]
```

Clients, projects and tasks are identified by their keys (`acme`, `website`, `meetings`). [[ADR-0006](docs/adr/0006-harvest-is-optional.md)]
`repos[].path` is the repo's `git rev-parse --show-toplevel`, compared canonically.
One repo maps to one client and project; `--task <key>` picks a non-default task.
Rates are hourly, per project, with optional per-task overrides, in the client's currency.
`jimtime config check` validates it and summarizes what it found, and a typo'd key or a mapping to an unknown project fails at load, not mid-invoice.

Upgrading from the Harvest-only version?
`jimtime config migrate` converts `harvest-projects.json` into `jimtime.toml`, keeps your task aliases as keys and your Harvest ids, and rewrites the day files to match, in one commit.
Everything Harvest goes to its own `config/harvest.toml`: pushing (left **off**), numbering, and the Harvest ids of your clients, projects and tasks.
Add `rate`s and `[business]` to invoice, or set `enabled = true` in harvest.toml to keep pushing.
Run it again on a jimtime.toml from before harvest.toml existed and it moves the Harvest settings out, editing the file in place so your comments and edits stay.

Confirm a repo resolves:

```
$ jimtime map
Repo:             /Users/you/code/acme/website
Client:           Acme Corp (acme)
Project:          Website (website)
Default task:     Development (development)
Billable default: yes
Rate:             150 USD/h
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
`--date YYYY-MM-DD` backfills an earlier day, and `--billable no` overrides the project's default.

### Review

```
$ jimtime review --today
Review: 2026-08-17

Acme Corp - Website - Development
  2026-08-17-acme-corp-website-development-001
    ●   1.25h  billable  Implemented webhook retry handling
  2026-08-17-acme-corp-website-development-002
    ●   1.50h  billable  Reviewed the invoice sync PR  [needs review]
  Total: 2.75h · 2 unapproved · 1 needs-review · 0 ready to invoice

Acme Corp - Website - Meetings
  2026-08-17-acme-corp-website-meetings-001
    ●   0.50h  billable  Weekly sync
  Total: 0.50h · 1 unapproved · 0 needs-review · 0 ready to invoice

Totals: 3.25h (3.25h billable) · 0 ready to invoice
```

`●` is unapproved and `○` is approved; `[invoice 2026-004]` and `[imported]` mark entries already billed or pushed.
Add `--pending` to list only what is outstanding.
Each entry prints its stable id, which is what `approve --only` and `--except` take.

### Approve

Approval is the human gate, and it is per entry.
`approve` sweeps everything unapproved in range except entries flagged `needs-review`, which it holds and names:

```
$ jimtime approve --today
Approved 2 entries:
  2026-08-17  1.25h  Acme Corp - Website - Development  (2026-08-17-acme-corp-website-development-001)
  2026-08-17  0.50h  Acme Corp - Website - Meetings  (2026-08-17-acme-corp-website-meetings-001)

Held 1 entry flagged needs-review (approve with --include-needs-review, or --only <id>):
  2026-08-17  1.50h  Acme Corp - Website - Development  (2026-08-17-acme-corp-website-development-002)
```

```sh
jimtime approve --week --except <id>          # sweep, but hold specific entries
jimtime approve --week --include-needs-review # sweep the flagged ones too
jimtime approve --only <id>                   # approve just these, bypassing the hold
jimtime unapprove --only <id>                 # take one back
```

`unapprove` mirrors `approve`: it sweeps the range, honors `--except`, and takes `--only` to act on exactly the ids you name.
It skips entries already on an invoice (void the invoice first) or pushed to Harvest (`harvest unpush` first).
Both commands also narrow by `--client`, `--project`, or `--repo`.

### Invoice

An invoice bills one client's approved, billable, not-yet-invoiced time over a period.
Draft it first. It renders the PDF, opens it, and saves nothing:

```
$ jimtime invoice draft --client acme --last-month
Invoice DRAFT for Acme Corp (acme)
  Period:      2026-08-01 to 2026-08-31
  Issued/due:  2026-09-01 / 2026-10-01
  Lines:       12 entries, 18.25h
  Total:       $2,662.50 USD
  Email to:    billing@acme.example
  Bcc:         you@example.com
  Fingerprint: 161ad9998023

Preview: .../invoices/.drafts/acme-draft.pdf

Nothing was saved. To finalize exactly this invoice:
  jimtime invoice finalize --client acme --from 2026-08-01 --to 2026-08-31 --confirm 161ad9998023
```

It also says what it left out: unapproved or needs-review time in the period, and any entries also pushed to Harvest, where invoicing from both would double-bill.

Then finalize.
In a terminal it shows the numbered PDF and asks; from a script or Claude Code it needs the draft's `--confirm <fingerprint>`, and refuses if what would be billed has changed since:

```
$ jimtime invoice finalize --client acme --last-month
...
Finalize invoice 2026-004 and email it? [y/N] y

Finalized invoice 2026-004: .../invoices/2026/2026-004.pdf
Emailed to billing@acme.example
Uploaded to dropbox: /Invoices/Invoice 2026-004.pdf
```

On approval it assigns the next number, writes the invoice record (`invoices/YYYY/<number>.json`, a full snapshot) and the PDF, locks each entry to that number, commits and pushes, emails the PDF, and uploads it to any configured cloud folder.
If the email or an upload fails, the invoice stays issued and the error names the retry: `invoice send <number>` or `cloud upload <number>`.
`--no-send` finalizes without emailing.
An invoice goes to the client's `email_to`, Cc'd to its `email_cc` and to `[email] cc`, and Bcc'd to `[email] bcc`; add a one-off Cc with `--cc <address>` on `draft` and `finalize` (it is part of the fingerprint, so what you approved is who gets it).
An address listed in more than one place gets the email once, in the most visible field.

```sh
jimtime invoice list                 # number, date, client, total, paid/open/overdue
jimtime invoice open 2026-004        # the PDF
jimtime invoice send 2026-004        # email it (again); --to / --cc add recipients
jimtime invoice paid 2026-004        # record the payment (--date, or --undo)
jimtime invoice void 2026-004        # unlock its entries; the number stays used
```

```
$ jimtime invoice list
NUMBER       ISSUED      CLIENT                            TOTAL  STATUS
035          2026-08-16  Magic Mind                 2,062.50 USD  paid 2026-09-17 (from Harvest)
036          2026-09-17  Magic Mind                 3,150.00 USD  open, due 2026-10-17 (from Harvest)

Outstanding: 3,150.00 USD
```

#### Bringing your Harvest history over

If you invoiced from Harvest before, import that history before you switch Harvest off: [[ADR-0011](docs/adr/0011-harvest-history-and-payments.md)]

```sh
jimtime invoice import-harvest --dry-run
jimtime invoice import-harvest
```

It only reads Harvest.
Every Harvest invoice becomes a local record with its PDF and paid date, every entry Harvest billed is locked to that invoice so it can never be billed again, and time that only ever lived in Harvest is added to your store.
Imported records keep Harvest's own line items, discounts and amount, since what the client was billed is not always the tracked hours times the rate.
It needs the `HARVEST_*` credentials and the Harvest ids in harvest.toml (which `config migrate` writes), and it can be re-run to pick up changes, such as an invoice getting paid.

Numbers default to `{year}-{seq:03}` (`2026-004`), restarting each year; set `invoice.number_format` and `invoice.start_seq` to continue an existing sequence.
Finalizing requires a successful pull when the data repo has a remote, so the next number is always the real next number.

Coming from Harvest invoicing, carry its numbers on instead:

```toml
# jimtime.toml
[invoice]
number_format = "{seq:03}"   # match Harvest's: 036 -> 037

# harvest.toml
numbering = true
```

`draft` and `finalize` then read your Harvest invoice numbers (read-only, with the `HARVEST_*` credentials, whether or not pushing is enabled) and continue past the highest of Harvest's and jimtime's, so an invoice issued in Harvest during the switch can never be duplicated.
The draft prints the number finalize will use, and finalize refuses to run if it cannot read Harvest.

#### Templates

The built-in template is a clean US Letter layout: a summary by project and task, the amount due and payment instructions, then the time detail with a page footer.
To make it yours, copy [`src/invoice/default.html`](src/invoice/default.html) into `$JIMTIME_HOME/config/templates/`, and set `invoice.template = "templates/invoice.html"` (or `template` on one client).
It is plain HTML and CSS with [MiniJinja](https://docs.rs/minijinja) (Jinja2) tags; page size and margins come from CSS `@page`, and relative links such as a logo resolve next to the template.
The context is `invoice`, `business`, `client`, `lines` (one per entry), `groups` (per project/task/rate), `currency_symbol` and `draft`, with filters `money`, `hours` and `css_string`.
The email subject and body under `[email]` are templates with the same context.

### Push to Harvest (optional)

Harvest's settings live in `config/harvest.toml`, beside jimtime.toml:

```toml
enabled = true      # push time to Harvest (off by default)
numbering = false   # continue Harvest's invoice numbers

# Tasks are based on Harvest's tasks. jimtime knows the union of the tasks
# here and in jimtime.toml.
[tasks.development]
id = 345
name = "Development"

[clients.acme]
id = 123

[clients.acme.projects.website]
id = 234
```

Keys are jimtime's (`acme`, `website`, `development`); the ids are Harvest's.
`jimtime harvest clients | projects | tasks --project ID` looks them up.

Always dry-run first.
It makes no API calls and needs no credentials:

```
$ jimtime harvest dry-run --today
Dry run: Harvest import - 2026-08-17

2026-08-17  1.25h  Acme Corp - Website - Development
  id: 2026-08-17-acme-corp-website-development-001
  notes: Implemented webhook retry handling

Total eligible: 1 entry, 1.25h
No entries were pushed.
```

```sh
jimtime harvest push --week      # creates real entries in Harvest
jimtime approve --today --push   # approve and push in one step
jimtime harvest unpush --today   # delete from Harvest, unlink locally
```

Push sends approved, billable, not-yet-imported entries and saves each returned Harvest id back immediately, so a re-run skips them.
Hours are pushed exactly as stored, with no rounding.
`approve --push` is opt-in because approving is local and reversible and pushing is not. [[ADR-0005](docs/adr/0005-approve-and-push-stay-separate.md)]
`unpush` deletes the Harvest entry and clears the link; Harvest refuses to delete an invoiced or locked entry, and those stay linked.
`jimtime harvest uninvoiced` shows what Harvest says each client owes, computed from the rates set there.

### Cloud copies of invoices (optional)

Git holds and syncs everything; Google Drive and Dropbox only get a copy of each finalized invoice PDF, in a folder you can browse or share. [[ADR-0010](docs/adr/0010-invoice-pdfs-to-cloud-folders.md)]

```toml
[cloud.dropbox]
app_key = "..."          # your own Dropbox app
folder = "/Invoices"

[cloud.google_drive]
client_id = "....apps.googleusercontent.com"
folder = "Business/Invoices"
```

```sh
jimtime cloud login dropbox        # one-time browser login
jimtime cloud login google-drive
jimtime cloud status
```

The login stores a refresh token in the OS keychain, never on disk.
Creating the Dropbox app and the Google OAuth client is a one-time step, described in [`docs/agents/systems/`](docs/agents/systems/).

### Report

`jimtime report --week` writes a markdown table you can paste into a status update, grouped by client, project, and task, with subtotals and a grand total.
Add `--billable-only` to drop the rest.

### Date ranges

`review`, `approve`, `unapprove`, `report`, `invoice` and `harvest` all take the same range flags:

| Flag | Range |
|---|---|
| `--today` | Today (the default, except for `invoice`, which always needs a range) |
| `--week` | The current Monday to Sunday week |
| `--last-week` | The previous Monday to Sunday week |
| `--month` | The current calendar month |
| `--last-month` | The previous calendar month |
| `--date YYYY-MM-DD` | A single day |
| `--from A --to B` | An inclusive range |

Days are anchored to one timezone regardless of the machine's clock, so "today" stays stable when travelling.
Set `JIMTIME_TZ` to any IANA name to choose it; it defaults to `America/Los_Angeles`.

## Commands

| Command | What it does |
|---|---|
| `status` | Show the current repo, its mapping, today's store path, and sync state |
| `map` | Show the client/project mapping and rate for the current repo |
| `add` | Add a time entry for the current repo |
| `today` | Print today's time log |
| `review` | List entries over a date range or a single day |
| `approve` | Approve unapproved entries, the human gate before billing |
| `unapprove` | Set matching entries back to unapproved |
| `report` | Export a markdown time report |
| `invoice` | Draft, finalize, send, list, open and void invoices |
| `harvest` | Optional: query Harvest, dry-run or push approved entries |
| `cloud` | Optional: log in to Dropbox/Google Drive and upload invoice PDFs |
| `config` | Create, migrate and check the config |
| `data` | Set up and inspect the auto-synced data repo |

Run `jimtime <command> --help` for the full flag list.

## How data is stored

```
$JIMTIME_HOME/
  config/jimtime.toml               the config
  config/templates/                 your invoice templates (optional)
  entries/YYYY/MM/YYYY-MM-DD.json   the time, one file per day
  invoices/YYYY/<number>.json       each invoice's record
  invoices/YYYY/<number>.pdf        ...and its PDF
```

A day file looks like this: [[ADR-0002](docs/adr/0002-per-day-json-store.md)]

```json
{
  "date": "2026-08-17",
  "sections": [
    {
      "repo_path": "/Users/you/code/acme/website",
      "client": "acme",
      "client_name": "Acme Corp",
      "project": "website",
      "project_name": "Website",
      "task": "development",
      "task_name": "Development",
      "entries": [
        {
          "id": "2026-08-17-acme-corp-website-development-001",
          "hours": 1.25,
          "billable": true,
          "approved": true,
          "needs_review": false,
          "notes": "Implemented webhook retry handling",
          "invoice": "2026-004"
        }
      ]
    }
  ]
}
```

A section groups entries that share a repo, client, project, and task.
It is a storage and display grouping only; approval lives on the entry.
Harvest ids (`harvest_*_id`, `harvest_time_entry_id`) appear only when Harvest is used.
Terminal output and markdown reports are rendered on demand and never persisted.

If an entry is wrong, fix it through the CLI or edit the day's JSON directly.
It is plain, stable, and meant to be read.
jimtime commits only the files it writes itself, so record a hand edit with `jimtime data sync`, which checks every day file and the config still parse before committing and pushing.

## The Claude Code skill

This repo ships a `/jimtime` skill at [`.claude/skills/jimtime/`](.claude/skills/jimtime/SKILL.md) that lets [Claude Code](https://claude.com/claude-code) drive the workflow.
It summarizes the work from your session into a conservative entry via `add --needs-review`, helps you review, and runs `approve`, `invoice finalize` or `harvest push` only on your explicit instruction.

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
`cargo test keychain -- --ignored` also checks, against the real OS keychain, that cloud logins persist.
Tagging `vX.Y.Z` builds the macOS binaries and opens a draft release with notes generated by [git-cliff](https://git-cliff.org/).
It lands as a draft, so nothing publishes without a review.

`.rustfmt.toml` sets a few nightly-only options.
Stable `cargo fmt` warns about them and ignores them, which is expected.

### Running your changes

Building does not update the `jimtime` on your `PATH`.
`cargo install` put a *copy* there, so `cargo build` only refreshes `./target/`, and every other shell keeps running whatever you installed last.
Reinstall to make a change live:

```sh
cargo install --path . --force --locked
```

Nothing does this for you, and the stale binary gives no hint that it is stale, so it is easy to test a change in this repo and then use the old one everywhere else.
`--locked` builds against `Cargo.lock`, the same dependency set CI and the release binaries use.

To confirm which build is on your `PATH`, compare it against a fresh one:

```sh
cargo build --release
shasum -a256 "$(command -v jimtime)" target/release/jimtime
```

Matching hashes mean the installed binary is current.

## Project docs

| File | What it covers |
|---|---|
| [`CONTEXT.md`](CONTEXT.md) | The domain glossary: entry, store, config, key, invoice, fingerprint, data repo |
| [`docs/adr/`](docs/adr/) | The load-bearing decisions and why they were made |
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | The build spec and data model |
| [`AGENTS.md`](AGENTS.md), [`docs/agents/`](docs/agents/) | Durable project context for AI agents, including external system setup |

## License

MIT. See [LICENSE](LICENSE).
