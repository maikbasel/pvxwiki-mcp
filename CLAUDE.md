# pvxwiki-mcp

MCP server that lets AI agents search Guild Wars 1 builds from the PvX wiki. A scheduled job copies
build pages through the wiki's MediaWiki API into a local database; the MCP server answers only from
that database. Self-hosted in a homelab, reachable from the internet.

The original brief is `docs/handover.md`. Read it before proposing features or data shapes.

Rules in `.claude/rules/`. Always loaded: `production-grade.md`, `scraping.md`, `licensing.md`,
`security.md`, `data-fidelity.md`. Path-scoped: `logging.md`, `testing.md`, `documentation.md`.

## Stack

Rust (stable, edition 2024), Cargo workspace: `crates/parser` (pure wikitext parser on
`parse-wiki-text-2`), `crates/store` (SQLite with FTS5 via `rusqlite`, `deadpool-sqlite`,
`rusqlite_migration`), `crates/server` (MCP tools on `rmcp` streamable HTTP, axum, tower,
`governor`), `crates/admin` (operator admin page: maud, htmx, Pico CSS), `crates/scraper`
(paced MediaWiki API client on `reqwest`, plus the runner that fills the store on a schedule or on
request) and `crates/pvxwiki-mcp` (the bin: wires the store, the runner and both listeners). Deployed as a container with docker compose (`deploy/`).

## Commands

```bash
prek install         # once per clone: git hooks from .pre-commit-config.yaml
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo deny check     # advisories, licences, sources (CI runs it too)
cargo fmt            # also runs automatically after every edit (hook in .claude/settings.json)
```

## Domain invariants

- **The MCP request path never calls the wiki.** Agents are served from the local database only.
  Only the scheduled scraper talks to Fandom.
- **A failed or partial scrape never destroys good data.** The previous copy of a page stays until
  a newer one parses successfully.
- **Every response carrying wiki content carries its attribution.** See `.claude/rules/licensing.md`.
- **Page nuance survives.** The main bar, its optional slots and each variant bar are data;
  everything else stays as section text, never dropped. See `.claude/rules/data-fidelity.md`.

## Out of scope

A general GW1 skill database. Template codes: not extracted, decoded, generated or stored.

## Definition of done

`cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo build` pass, and you have shown
their output. New behaviour has a test.

## Repository etiquette

- Conventional commits with a scope: `feat(scraper): ...`, `fix(search): ...`.
- Do not commit, push, or open a PR unless asked.

## Never modify without asking

- The User-Agent string and the request pacing constants (`.claude/rules/scraping.md`).
- The attribution text and licence constants (`.claude/rules/licensing.md`).
- Public MCP tool names and their input/output schemas, once the server has been deployed.
