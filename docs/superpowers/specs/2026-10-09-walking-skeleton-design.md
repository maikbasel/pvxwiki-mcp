# Walking skeleton: store, MCP server, bin, container

Status: draft for review. Date: 2026-10-09.

## Goal

The thinnest real end-to-end path: an MCP client calls `search_builds` and `get_build` over
streamable HTTP and gets answers from a real SQLite database, with attribution, input bounds and
abuse limits in place. The service runs as one container that anyone can self-host with docker
compose. The scraper is not part of this spec; until it exists, tests fill the database through the
store's write function and a fresh server answers "index not built yet".

## Decisions this spec rests on

- One long-running service per host: scheduled scraper (later spec) plus MCP server plus embedded
  SQLite. Self-hostable by others; shipped as a container image; deployed with docker compose.
- Plugins (later spec) connect to a hosted instance URL. No stdio transport.
- No auth in the service. The public instance is authless with abuse limits; private instances use
  their reverse proxy's auth (Pangolin, Caddy, Cloudflare Access).
- SQLite (not a separate database server, not the Turso rewrite): data is a few MB, rebuildable from
  the wiki, one writer.
- Two tools: `search_builds` returns short results with snippets; `get_build` returns one full page.
  Section prose is returned as raw wikitext. Default search covers every page except archived ones.
- Template codes are out of scope.

## Crates

```
crates/
  parser/        done (docs/superpowers/specs/2026-10-09-parser-design.md)
  store/         pvxwiki-store: migrations, write_page, search, get_page, status
  server/        pvxwiki-server: MCP tools, HTTP router and limits, /healthz. No scraper dependency.
  pvxwiki-mcp/   bin: env config, tracing subscriber, wiring, `serve` and `healthcheck`
deploy/          Dockerfile, compose.yaml, .env.example, Caddyfile
```

Stack (checked 2026-10-09; versions are what the plan pins):

| Purpose | Crate |
|---|---|
| MCP | `rmcp` `=3.5.1` (features `server`, `macros`, `transport-streamable-http-server`), `schemars` 1 |
| HTTP | `axum` 0.8, `tower` 0.5 (`limit`, `load-shed`, `timeout`), `tower-http` 0.7 (`limit`, `timeout`, `trace`) |
| Rate limit | `governor` 0.10 in our own middleware (not `tower_governor`: ISC dependency, spoofable IP extractor) |
| Storage | `rusqlite` 0.40 (`bundled`, FTS5 included), `deadpool-sqlite` 0.14, `rusqlite_migration` 2.6 (`from-directory`) |
| Runtime | `tokio` 1, `serde`, `serde_json`, `thiserror`, `tracing`, `tracing-subscriber` |

`rmcp` is pinned exactly: its 3.x line has frequent breaking changes.

## Tools

### `search_builds`

Input (every field optional; bounds are named consts and part of the JSON schema):

| Field | Type | Bound |
|---|---|---|
| `query` | string, full text | max 200 chars |
| `skills` | list of skill names; a page matches if any bar has all of them | max 8 entries, 64 chars each |
| `professions` | list of enum (10 professions); primary profession of any bar | enum |
| `gameplay_types` | list of enum (AB, FA, JQ, GvG, HA, RA, PvP team, general, farming, running, hero, SC, PvE team, quest) | enum |
| `ratings` | list of enum (great, good, creative, testing, trial, trash, abandoned) | enum |
| `include_archived` | bool, default false | |
| `limit` | int, default 10 | 1..=25 |
| `cursor` | opaque string from a previous response | parsed and range-checked |

Output:

```jsonc
{ "results": [{
    "title": "...", "url": "...", "revision": 1266398,
    "rating": "great", "status": "meta", "gameplay_types": ["farming"],
    "bars": [{ "name": null, "variant": null, "primary": "Assassin", "secondary": "Dervish" }],
    "matches": [{ "section": "Equipment", "snippet": "... Zealous Dagger ..." }],
    "attribution": { "credit": "PvX wiki contributors", "licence": "CC BY-NC-SA 2.5",
                     "licence_url": "https://creativecommons.org/licenses/by-nc-sa/2.5/" } }],
  "next_cursor": "...",
  "index": { "pages": 795, "updated_at": "2026-10-09T06:00:00Z" } }
```

Ranking: with `query`, FTS5 BM25 with column weights title 10, heading 5, body 1; ties by rating
(great, good, creative, then the rest), then title. Without `query`, by rating then title.
`matches` is empty when there is no `query`.

### `get_build`

Input: `title` (exact page title, or a `https://gwpvx.fandom.com/wiki/...` URL), max 300 chars.

Output: title, url, revision, vetting (rating, status, gameplay types, archive date/category/
reasons), every section in page order (heading, level, exact `wikitext`), every skill bar fully
structured (section index, name, variant name/position, professions, attributes with canonical and
raw name and value, slots), and attribution.

### Errors

| Situation | Returned as |
|---|---|
| Index empty | Tool error: index not built yet, first scrape in progress |
| `get_build` title not found | Tool error naming the title, suggesting `search_builds` |
| Bad input (bound, enum, cursor) | Tool error naming the field and what is allowed |
| Query over its time budget, or database busy | Tool error: try again or narrow the search |
| Bug or I/O failure | Generic tool error with a request ID; detail only in the `error` log |

Wiki text appears only in the fields `snippet` and `wikitext`, never in server messages
(`security.md`, prompt injection).

## Storage

One SQLite file in `PVXWIKI_DATA_DIR`, WAL mode. Migrations in `crates/store/migrations/`, one
folder per migration named `<n>-<name>` with `up.sql` (consecutive numbers, no dates), embedded
with `include_dir` and applied with `to_latest` on open. The version lives in SQLite's
`user_version`.

`01-initial_schema/up.sql`:

```sql
pages        (id PK, title UNIQUE, namespace, revision, fetched_at,
              vetting, rating, status, document)   -- document: JSON get_build payload minus attribution
page_types   (page_id, gameplay_type)              -- lowercase, indexed
page_skills  (page_id, bar, skill)                 -- lowercase, every slot skill of every bar, indexed
page_profs   (page_id, profession)                 -- primary profession of every bar, indexed
sections_fts USING fts5(title, heading, body, page_id UNINDEXED, section UNINDEXED,
                        tokenize = 'porter unicode61 remove_diacritics 2')
```

- `write_page(title, namespace, revision, fetched_at, &ParsedPage)` replaces a page's rows in one
  transaction. Readers see the old or the new copy, never a mix. Deciding not to replace a good
  copy with a worse parse is the scraper's job.
- The store owns serializable types mapped from the parser's output; the parser gains no `serde`.
- Status: page count and newest `fetched_at`.
- Categories and scrape-run history arrive with the scraper (migration `02`).

### Query safety

- SQL text is built only from fixed fragments chosen by which filters are set; every value is a
  bound parameter. Placeholder count is bounded by the input bounds.
- FTS5 `MATCH` input is escaped: each word wrapped in `"..."` with inner `"` doubled, so `AND`,
  `OR`, `NOT`, `NEAR`, `*`, `^` and `col:` are literal.
- The server's pool opens the file read-only (`SQLITE_OPEN_READ_ONLY`, `PRAGMA query_only`).
  Writes come only from the separate writer connection.
- A progress handler aborts any query over a named time budget.
- The cursor decodes to a number and is range-checked; it is never put into SQL text.

### Port

`BuildStore` trait: `search`, `get_page`, `status`. Implementations: the SQLite store and an
in-memory fake for server unit tests. One generic contract suite runs against both and covers what
both can honour: filters, archived exclusion, not found, filter-only ordering, empty index. BM25
ranking, snippets and porter stemming are tested on the SQLite store only.

## Server

- `rmcp` streamable HTTP at `/mcp`, stateless (legacy session mode off, JSON responses).
- Allowed hosts from `PVXWIKI_ALLOWED_HOSTS` (rmcp's default accepts only localhost).
- Tools via `#[tool]`, inputs `Parameters<T>`, outputs `Json<T>` (structured content and output
  schema).

Layers, outermost first, every value a named const:

1. Per-client rate limit (`governor`, keyed by client IP). `X-Forwarded-For` is read only when the
   peer is in `PVXWIKI_TRUSTED_PROXIES`, taking the rightmost hop not in that list. Over the limit:
   429 with `Retry-After`. Stale keys are dropped periodically.
2. Global concurrency cap with load shedding: 503, no queue.
3. Request timeout and request body deadline.
4. Request body size limit.
5. Response size: result limit plus a per-page byte budget.

`/healthz` returns 200 when the database opens; not rate-limited. No auth code anywhere.

## Bin

- `pvxwiki-mcp serve`: read config, install the tracing subscriber, open the store (applying
  migrations), start the server.
- `pvxwiki-mcp healthcheck`: GET `/healthz` on the configured listen address; exit 0 or 1. Exists
  because the distroless image has no shell or curl.

Config, parsed once with `std::env` into a typed struct; a missing or invalid required value stops
startup with one log line naming the variable:

| Variable | Required | Default |
|---|---|---|
| `PVXWIKI_ALLOWED_HOSTS` | yes | none |
| `PVXWIKI_CONTACT` | yes once the scraper exists | none |
| `PVXWIKI_TRUSTED_PROXIES` | no | empty |
| `PVXWIKI_DATA_DIR` | no | `/data` |
| `PVXWIKI_LISTEN` | no | `0.0.0.0:8080` |
| `PVXWIKI_LOG_FORMAT` | no | `json` |
| `RUST_LOG` | no | `info` |

Rate limits, timeouts and size caps are consts, not config.

## Logging

Per `.claude/rules/logging.md`. Subscriber on stderr, JSON or pretty, default filter `info`. Each
MCP request is a span with request ID, tool name and a hashed client IP; spans log their duration
on close. Rate-limit rejections are `warn`. Startup logs the effective config at `info`.

## Deploy

- `deploy/Dockerfile`: multi-stage with `cargo-chef`; build on `rust:1.96.1-slim-trixie`, run on
  `gcr.io/distroless/cc-debian13:nonroot`; amd64 and arm64.
- `deploy/compose.yaml`: the service, named volume `pvxwiki-data` at `/data`, `env_file: .env`,
  healthcheck via `healthcheck`, `restart: unless-stopped`. Caddy behind the `https` profile.
- `deploy/.env.example`: every variable above, commented.
- `deploy/Caddyfile`: reverse proxy to the service with automatic HTTPS for `PVXWIKI_DOMAIN`.
- CI builds the image on every push (no publish until the GitHub repo exists) and runs it under
  compose until the healthcheck passes.

## Testing

Outside-in per `.claude/rules/testing.md`.

- Acceptance tests (bin crate): temp database filled through `write_page` from the parser
  fixtures, real server on a random port, tools called over streamable HTTP with `rmcp`'s client.
  Cases: the brief's three example queries, `get_build` with attribution, empty index, not found,
  each input bound, rate limit returning 429.
- Contract suite for `BuildStore` against the SQLite store and the fake.
- Unit tests: FTS5 escaping (operators and quotes matched literally), client IP extraction with
  trusted and untrusted peers, cursor decoding, config parsing.

## Project file changes

- `CLAUDE.md` Stack: record the store/server/bin crates and the chosen crates.
- `.claude/rules/security.md`: replace "If token auth is added" with "No auth in the service;
  private instances use their reverse proxy's auth".
- `.claude/rules/scraping.md`: the contact comes from `PVXWIKI_CONTACT`.
- `deny.toml`: add licences only if `cargo deny` reports a new one; each addition named in the plan.

## Later specs

Scraper (MediaWiki client, schedule, keep-last-good, categories); GitHub repo and image
publishing to GHCR; plugins (Claude Code marketplace, Claude Desktop extension, client snippets);
README with self-hosting, attribution and licence.
