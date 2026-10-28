# Invoices are generated locally from HTML templates and rendered to PDF by headless Chrome

jimtime used to leave money to Harvest: "jimtime never applies rates itself".
Invoicing without Harvest means jimtime has to know rates and produce a document a client will pay from.

## Decision

- **Rates live in config**: an hourly `rate` on each project, with optional per-task overrides in `task_rates`, in the client's `currency`.
  Each invoice line is one Entry: `amount = round2(hours × rate)`, and the total is the sum of the rounded lines, so the PDF always adds up.
  Hours are never rounded (the existing billing rule).
- **Templates are HTML + CSS**, rendered with MiniJinja (Jinja2 syntax).
  A default template is built into the binary; `invoice.template` in config, or `template` on a client, points at a user template under `config/`.
  Page size and margins come from CSS `@page`, so the template owns the whole layout.
- **PDF rendering shells out to an installed Chromium-family browser** (`--headless --print-to-pdf`), found via `invoice.chrome`, `$JIMTIME_CHROME`, the standard macOS app paths, then `PATH`.
  Headless mode runs on its own temporary profile, so it never touches the user's browser profile and works while Chrome is open (verified: only Chrome's shared crash-reporter state is written).
  An explicit `--user-data-dir` is deliberately not passed: with one, Chrome writes the PDF and then never exits.
  We verified the CLI honors CSS `@page` size and margins.
  We rejected a CDP client crate (large dependency for the same output), wkhtmltopdf (unmaintained, old WebKit), and a pure-Rust renderer (none renders real-world HTML/CSS faithfully).

## Consequences

- Invoices look exactly like the HTML the user writes, previewable in any browser.
- Rendering needs Chrome, Chromium, Brave or Edge installed.
  That is a runtime dependency, not a build one, and the error names the setting to fix.
- Harvest's own invoicing and jimtime's are independent.
  Pushing an entry to Harvest does not invoice it here, and invoicing here does not touch Harvest.
