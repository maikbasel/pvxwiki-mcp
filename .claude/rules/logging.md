---
paths:
  - "crates/*/src/**/*.rs"
---

# Logging

Every crate logs through `tracing`. The logs must let the operator see what the service is doing
and, when something goes wrong, find the failing page, request or line of code without
reproducing the problem first.

## Setup

- Library crates emit spans and events only. The bin installs the subscriber, once.
- The subscriber writes to stderr. Stdout is reserved for the MCP stdio transport if it is ever
  used, so nothing else writes there. No `println!` / `eprintln!`.
- Level filter from `RUST_LOG` (`EnvFilter`). The default is a named const, `info,rmcp=warn` (rmcp logs several `info` lines per request); set
  `RUST_LOG=pvxwiki=debug` (prefix match covers every `pvxwiki_*` crate) to get per-page detail.
  Output format is a named config value: JSON in production, human-readable in development.
- Spans record their duration on close (`FmtSpan::CLOSE`), so slow runs, pages and requests show
  up without extra code.

## Context comes from spans

- A scrape run is a span with a run ID. Each page inside it is a span with title and revision ID.
  Each MCP request is a span with request ID and tool name.
- Code inside a span does not repeat the span's fields. The parser does not know the page title;
  the scraper's page span supplies it.
- Public functions doing non-trivial work get `#[tracing::instrument(skip_all, fields(...))]` with
  the identifying fields (IDs, sizes), never whole inputs.

## Levels

- `error`: a bug in this service. Never an upstream failure.
- `warn`: everything the environment does to us. Fandom 429/5xx, `maxlag`, a page that failed to
  parse, a rate-limited client.
- `info`: scrape run started and finished (pages checked, updated, failed), server started.
- `debug`: per-request, per-batch and per-page detail, including each problem found in a page.
- `trace`: individual decisions (a lookup that missed, a tag skipped), for chasing one bug.

## Each event

- Structured fields, not interpolated text: `warn!(status, retry_after_s, "fandom rate limited us")`.
  The message is a constant string, so logs can be grouped by it.
- A `warn` or `error` says what happened, to which item (IDs), and what happens next (retrying at,
  page skipped, previous copy kept).
- Errors are logged with their full source chain, once, where they are handled. A function that
  returns a problem as a value or `Err` does not also log it at `warn`; it may log `debug` detail
  the caller cannot see (byte offset, a short snippet).
- Text from outside (wiki snippets, tool arguments) is logged with `?` (Debug), which escapes
  newlines and control characters, and is cut to a named maximum length.

## Never log

- Tokens or `Authorization` headers.
- Raw client IPs. Log a truncated or hashed form if rate-limit diagnostics need it.
- Full page wikitext. Log title, revision ID, length, and at most a short snippet around a problem.

## Tests

Tests never assert on log output (`testing.md`). Logging must not change behaviour.
