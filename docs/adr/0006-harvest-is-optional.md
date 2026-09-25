# Harvest is optional and off by default; jimtime owns client/project/task identity

Until now every Section carried Harvest's numeric `client_id` / `project_id` / `task_id`, and the only config file was `harvest-projects.json`.
Harvest was the identity system, so jimtime could not be used without it.

jimtime now invoices on its own (ADR-0007), so Harvest becomes one optional destination for approved time rather than the thing that names clients and projects.

## Decision

- Clients, projects and tasks are defined in `$JIMTIME_HOME/config/jimtime.toml`, keyed by short slugs chosen by the user (`magic-mind`, `automations`, `programming`).
  These **keys** are the identity, and the display names are just labels.
- A Harvest id is an optional attribute on each (`harvest_id`), used only by the Harvest integration.
- `[harvest] enabled = false` is the default.
  While it is disabled, every `harvest` subcommand and `approve --push` fails with a message pointing at the setting, and no Harvest credential is looked up.
  The one exception is its own opt-in: `invoice.harvest_numbering` reads Harvest's invoice numbers so a local sequence can continue Harvest's (ADR-0008). It writes nothing to Harvest, so it does not need pushing turned on.
- A Section stores the keys (`client`, `project`, `task`) alongside the display names.
  The Harvest ids it used to require become optional `harvest_*_id` fields.
  At push time the ids come from the section first, then from config, so ids added to config later apply to existing time.
- The config moved from JSON to TOML because it is now hand-edited prose (addresses, email bodies, payment instructions) that benefits from comments and multi-line strings.
  The store stays JSON (ADR-0002): it is machine-written, and TOML buys nothing there.
- `jimtime config migrate` converts `harvest-projects.json` into `jimtime.toml` (keys are slugified names, Harvest ids preserved) and rewrites every day file into the new Section shape in one pass, so the change lands as a single reviewable commit.
  Legacy day files are still read: a Section missing its keys gets them by slugifying its names, which is exactly how migrate derives config keys.
- `add --task-id` / `--task-name` are removed.
  An ad-hoc Harvest task id has no local identity or rate, so it cannot be invoiced; a task is now always a key in `[tasks]`.

## Consequences

- A fresh install needs no Harvest account at all.
- Migrating an existing Harvest setup leaves Harvest **disabled** (the default the user asked for); turning it back on is one line.
- Harvest ids never appear in a Section's identity, so renaming a Harvest project cannot split a day into two sections.
