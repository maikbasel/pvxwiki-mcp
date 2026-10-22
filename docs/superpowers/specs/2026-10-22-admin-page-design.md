# Admin page

A small web page for the operator of an instance: see what the scraper did and is doing, and start
a run by hand.

## Why

Today the only window into the scraper is `docker logs`. The operator cannot tell at a glance
whether the index is current, whether the last run failed, when the next run starts, or which pages
the parser struggled with. Starting a run means restarting the container.

## Who

Only the operator of an instance (the maintainer for the hosted instance, anyone who self-hosts for
theirs). Agents and the public never see it.

## Goals

The page shows:

- **Status:** scraper on or off (off when `PVXWIKI_CONTACT` is unset, with that reason), blocked
  (after a 403 or challenge page, until restart), running or idle, when the next scheduled run
  starts, and how many pages the index holds, split by namespace (Build 100, Archive 104).
- **Live progress** of a running run: phase (listing or fetching), pages done of total, elapsed
  time. Refreshes on its own while the page is open.
- **Run history:** start, duration, trigger (scheduled or manual), checked, updated, kept and removed
  counts, and the outcome: finished, failed (with the error message), or blocked.
- **Run now:** starts a run immediately. Disabled, with the reason shown, while a run is going, when
  the scraper is off, and when it is blocked.
- **Pages needing attention:**
  - pages whose newest revision failed to parse, so the stored copy is older than the wiki's (the
    runs' `kept` pages). Each shows the title, the stored revision and the failed revision, with an
    `?oldid=` link to each; the failed one is what needs fixing.
  - pages stored with parse issues (`ParseIssue`s), with the issue kinds and an `?oldid=` link to
    the stored revision.

### Calls made while writing this (not discussed with the maintainer)

- **Failed parses are state, not run history.** The store keeps a `parse_failures` table
  (title, failed revision, first seen). A run inserts or updates a row when it keeps a stored copy,
  and deletes it when that page is later written or removed. The page lists this table, so the list
  is the same whether the newest run finished or not. The scraper reports each kept page with its
  failed revision.
- **A run's row is written when it starts** with outcome `running`, and closed when it ends. At
  startup, any row still `running` becomes `interrupted` (the process was killed mid-run,
  `production-grade.md`). Outcomes: running, finished, failed, blocked, interrupted.
- **The parse-issue list is grouped and capped.** A count per issue kind on top; the list below is
  filtered to one kind at a time, serious kinds (`TimedOut`, `TooDeeplyNested`) selected by default,
  capped at a named const (e.g. 50) with "and N more". Filtering may use SQLite's `json_each` over
  the stored document; an indexed column is not needed at this size.

## Non-goals

- A "resume after block" button. After a block a person investigates and restarts the container;
  a button would put retrying through a block one click away (`.claude/rules/scraping.md`).
- Editing configuration, managing users, viewing logs. `.env`, the proxy and `docker logs` own those.
- Authentication inside the service.
- Anything for agents or the public. The MCP tools and their schemas do not change.

## Anti-goals (a failure even if it works)

- The admin page, or its run button, reachable without protection on a default install.
- A manual run that requests from Fandom faster than the scheduled pacing, or two runs at once.
- A second toolchain (Node, a JS bundler) in the build or the image.

## Decisions made

- **Protection lives in the reverse proxy, not the service.** `security.md` stays: no auth in the
  service. Caddy in compose protects the admin page with a login and is optional. Operators with
  their own SSO proxy (the maintainer uses Pangolin) leave Caddy off and protect it there.
- **Separate listener.** The admin page is served on its own port (8081 inside the container), not
  under the MCP port, so a proxy that forwards the public MCP port never exposes it by accident.
- **Published on 127.0.0.1 by default**, through its own variable `PVXWIKI_ADMIN_PUBLISH_ADDR`
  (default `127.0.0.1`), independent of `PVXWIKI_PUBLISH_ADDR`. Opening the MCP port to the world
  must not open the admin port.
- **Caddy login is basic auth** with a username and a bcrypt hash from `.env`
  (`PVXWIKI_ADMIN_USER`, `PVXWIKI_ADMIN_PASSWORD_HASH`; the hash from `caddy hash-password`).
- **Technology:** HTML rendered by the existing Rust binary with `maud` (markup written in Rust,
  returned straight from axum handlers, no template files); `htmx` for the progress refresh and the
  run button; Pico CSS (MIT) for styling, which makes semantic HTML look finished without utility
  classes and follows the browser's light or dark mode. The htmx and Pico files are vendored and
  embedded in the binary, no CDN. No Node, no build step, same distroless image. Chosen over the
  more common Askama + Tailwind: three small views do not justify template files and a CSS build
  stage.
- **Project structure follows the existing crates:**
  - `crates/admin` (new, `pvxwiki-admin`): the admin axum router, maud views, and the vendored
    assets under `crates/admin/assets/` with their licence files. Mirrors `crates/server`, which
    owns the MCP router.
  - `crates/scraper`: owns the runner, the one place that starts runs: one run at a time, the
    schedule, manual triggers, live progress and the blocked state. The bin and the admin crate
    hold a handle to it.
  - `crates/store`: the `runs` and `parse_failures` tables (one migration) and their queries.
  - `crates/pvxwiki-mcp`: wiring only. Opens the store, builds the runner, binds both listeners,
    shuts both down on the same signal.
- **Run history is stored** in a new `runs` table (a store migration), since runs only reach the
  logs today. Bounded to the newest `RUN_HISTORY_LIMIT` rows (named const, e.g. 100).

## Constraints

- **CSRF and DNS rebinding.** Basic auth credentials are sent by the browser automatically, so the
  run endpoint must reject cross-site requests: accept `POST` only with `Sec-Fetch-Site: same-origin`
  (or absent, for non-browser clients) and the htmx `HX-Request: true` header, which a cross-site
  form cannot set. The admin listener checks the `Host` header like `/mcp` does: allowed are
  `PVXWIKI_ALLOWED_HOSTS` plus `localhost` and `127.0.0.1`. An operator serving the admin page on its
  own host name (for example `admin.example.org` in Pangolin) lists it in `PVXWIKI_ALLOWED_HOSTS`.
- **One run at a time.** Scheduled and manual runs share one runner. A manual request while a run is
  going is refused (HTTP 409, button disabled). A manual run does not move the schedule; a scheduled
  tick that falls inside a running run is skipped.
- **Pacing is unchanged.** A manual run uses the same client and `Pacing::FANDOM`.
- **Wiki text is untrusted.** Titles and issue text are HTML-escaped (maud does this by default).
  Wiki links are built from the title with URL encoding, never from raw page content.
- **Rate limit and size limits** on the admin listener like the MCP port: body size limit, request
  timeout. The page needs no per-client rate limit beyond that, being behind auth; the run endpoint
  is idempotent while a run is going.
- **Logging** (`logging.md`): a run's span records its trigger; a manual start logs at `info`.
- **Tests** (`testing.md`): acceptance tests drive the admin listener over HTTP against a temp
  store and a wiremock wiki (history appears after a run, run now starts one, a second run now
  while running is refused, cross-site POST is refused, run now when off or blocked is refused).
  Store tests cover the `runs` table and its bound. No browser tests.
- **Docs:** README gets an "Admin page" section (what it shows, how to protect it with the Caddy
  profile or your own proxy). `.env.example` gets the new variables. `security.md` gets one line
  about the admin listener and where its protection lives.

## Left to the builder

- Page layout and styling (a single page with minimal CSS is enough), poll interval of the progress
  refresh, exact table shape and the JSON or column layout of kept titles per run.
- How the scraper reports progress to the runner (a callback, a `watch` channel, shared state).
- Whether the Caddy admin site is a path on `PVXWIKI_DOMAIN` or its own host name, and how the
  compose profile is named or split, as long as the MCP-only HTTPS setup still works without admin
  credentials and an operator can run Caddy for the admin page alone.
- `maud`, `htmx` and Pico versions, within the licence policy (`cargo deny`); htmx is 0BSD.
