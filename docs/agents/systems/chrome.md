# Chrome (headless PDF printing)

[ADR-0007](../../adr/0007-local-invoices-from-html-templates.md)

## What

`invoice::render::pdf` writes the rendered HTML to a temp file next to the template (so relative links such as a logo resolve) and runs a Chromium-family browser with `--headless --no-pdf-header-footer --print-to-pdf=<out>`.
CSS `@page` sets size and margins, and `@page` margin boxes (`@bottom-right { content: counter(page) }`) carry the footer.

Lookup order: `invoice.chrome` in config, `$JIMTIME_CHROME`, the macOS app bundles (Chrome, Chromium, Brave, Edge in `/Applications` or `~/Applications`), then common names on `PATH`.

## Gotchas

- **Do not pass `--user-data-dir`.**
  With one, Chrome (seen on macOS) writes the PDF and then never exits.
  Without it, headless mode already runs on its own temporary profile; verified that the user's real profile is untouched apart from shared crash-reporter state.
- **Do not wait on stdout/stderr pipes.**
  Chrome's helper processes inherit them and outlive the browser, so `output()` hangs.
  stderr goes to a temp file and we wait on the exit status, under a 90s timeout.
- `--virtual-time-budget` gives web fonts and images time to load before printing.
- To look at a PDF page by page when testing, rasterize it (on macOS a few lines of Swift with PDFKit), since Quick Look only renders page 1.
