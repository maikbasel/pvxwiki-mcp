# Admin Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An operator-only admin page on its own port that shows scraper status, live progress, run
history and pages needing attention, and starts a run by hand.

**Architecture:** Outside-in. Task 1 writes the acceptance tests against the admin listener over
HTTP; they stay red until Task 7. The store gains `runs` and `parse_failures`; the scraper gains a
`Runner` that owns every run (one at a time, schedule, manual trigger, progress, blocked state) and
replaces the bin's `scrape_forever`; a new `pvxwiki-admin` crate renders the page; the bin wires
both listeners.

**Tech Stack:** axum 0.8, maud 0.27 (feature `axum`), htmx 2.0.4 (0BSD), Pico CSS 2.0.6 (MIT),
rusqlite, tokio, wiremock, reqwest (dev, for acceptance tests).

**Spec:** `docs/superpowers/specs/2026-10-22-admin-page-design.md`

## Global Constraints

- Workflow (`testing.md`): outside-in. Acceptance tests in `crates/pvxwiki-mcp/tests/` first; inner
  red-green with one test at a time; watch every test fail before the code that passes it.
- No auth in the service. Admin listener on `PVXWIKI_ADMIN_LISTEN`, default `0.0.0.0:8081`;
  compose publishes it on `${PVXWIKI_ADMIN_PUBLISH_ADDR:-127.0.0.1}:8081`.
- `POST /runs` requires `HX-Request: true` and `Sec-Fetch-Site` equal to `same-origin` or absent;
  otherwise 403. Host header must be in `PVXWIKI_ALLOWED_HOSTS`, `localhost` or `127.0.0.1`
  (port ignored); otherwise 421.
- Manual runs refused with 409 while running, when off, when blocked. No resume-after-block.
- Named consts: `RUN_HISTORY_LIMIT = 100`, `ISSUE_LIST_LIMIT = 50`, `PROGRESS_POLL = "every 2s"`,
  admin `MAX_BODY_BYTES = 1024`, admin `REQUEST_TIMEOUT = 10 s`.
- Outcomes stored as text: `running`, `finished`, `failed`, `blocked`, `interrupted`.
  Triggers: `scheduled`, `manual`.
- Serious issue kinds (default filter): `timed_out`, `too_deeply_nested`.
- Wiki links: `https://gwpvx.fandom.com/wiki/<title, spaces to _, percent-encoded>` and
  `https://gwpvx.fandom.com/index.php?oldid=<revision>`. Never from page content.
- Vendored assets served from the binary (`include_bytes!`), no CDN, no Node.
- `unwrap`/`expect` only in tests; logging per `logging.md` (manual start at `info`, trigger on the
  run span).
- Commits: conventional with scope; end with the session's `Co-Authored-By` trailer.

## Review Focus

1. **A run that panics or errors leaves the runner stuck "running".** Expected: the state returns to
   idle (or blocked) and the `runs` row is closed as `failed`. Test in Task 4.
2. **Restart after a block.** Expected: a fresh process starts unblocked and runs again; the old
   `blocked` row stays in history. Covered by `Runner` being in-memory; test in Task 4.
3. **A title with `/`, `?`, `#`, `&` or `"` in the attention list.** Expected: escaped in HTML and
   percent-encoded in the link, link still points at the right page. Test in Task 6.
4. **Admin listener reached through a proxy with a different Host and a port suffix**
   (`admin.example.org:8443`). Expected: accepted when the bare host is allowed. Test in Task 6.
5. **History beyond `RUN_HISTORY_LIMIT`.** Expected: oldest rows pruned at `start_run`, never the
   running row. Test in Task 2.

---

### Task 1: Acceptance tests for the admin listener (stay red)

**Files:**
- Create: `crates/pvxwiki-mcp/tests/admin.rs`
- Modify: `crates/pvxwiki-mcp/Cargo.toml` (dev-dep `reqwest` with `json`)
- Modify: `Cargo.toml` (workspace `reqwest` gains nothing; dev use only)

**Interfaces:**
- Consumes (future, Task 7): `pvxwiki_mcp::run(config, listener, admin_listener, limits, schedule, shutdown)`;
  `Config.admin_listen`.
- Produces: the acceptance suite every later task drives towards.

Helpers in the file: `fake_wiki()` (copy of the one in `acceptance.rs`: Vaettir page in namespace
100, empty 104), `start(contact: Option<&str>, wiki: &MockServer) -> (SocketAddr /*admin*/, TempDir)`
binding both listeners on `127.0.0.1:0` with `Schedule { interval: 3600 s, pacing: 1 ms }`,
`post_run(addr, headers) -> reqwest::Response`, `page(addr) -> String` (GET `/`), and
`wait_for(addr, needle, 5 s)` polling `GET /` until the body contains `needle`.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn history_shows_the_startup_run_once_it_finishes()
// start(Some(contact)); wait_for(addr, "finished"); page contains "scheduled" and "1" checked.

#[tokio::test] async fn run_now_starts_a_manual_run()
// after the startup run finished: post_run with HX-Request: true, Sec-Fetch-Site: same-origin
// -> 202; wait_for(addr, "manual").

#[tokio::test] async fn run_now_while_running_is_refused()
// wiki listing for ns 100 answers with a 2 s delay (ResponseTemplate::set_delay);
// immediately post_run -> 409.

#[tokio::test] async fn cross_site_run_now_is_refused()
// post_run with Sec-Fetch-Site: cross-site and HX-Request: true -> 403;
// post_run without HX-Request -> 403.

#[tokio::test] async fn run_now_without_a_contact_is_refused()
// start(None); post_run -> 409; page contains "PVXWIKI_CONTACT".

#[tokio::test] async fn run_now_after_a_block_is_refused()
// wiki answers 403 to everything; wait_for(addr, "blocked"); post_run -> 409.

#[tokio::test] async fn unknown_host_is_refused()
// GET / with Host: evil.example -> 421.

#[tokio::test] async fn admin_is_not_served_on_the_mcp_port()
// GET http://<mcp addr>/ and POST /runs on the MCP listener -> 404.
```

- [ ] **Step 2: Run, verify they fail to compile for the expected reason**

Run: `cargo test -p pvxwiki-mcp --test admin`
Expected: compile error on the new `run` arity / `admin_listen` field. They stay red until Task 7.

- [ ] **Step 3: Commit**

```bash
git add crates/pvxwiki-mcp/tests/admin.rs crates/pvxwiki-mcp/Cargo.toml Cargo.lock
git commit -m "test(admin): acceptance tests for the admin listener"
```

Note: the workspace stays non-compiling for this one test target until Task 7. Run inner tasks with
`-p <crate>` so they are unaffected.

---

### Task 2: Store: run history

**Files:**
- Create: `crates/store/migrations/02-admin/up.sql` (`runs` and `parse_failures`)
- Create: `crates/store/src/runs.rs`
- Modify: `crates/store/src/lib.rs` (exports), `crates/store/src/sqlite.rs` (`fresh_store_is_empty_and_migrated` expects `user_version` 2)
- Test: `crates/store/tests/runs.rs`

**Interfaces:**
- Produces:
  - `pub enum Trigger { Scheduled, Manual }`, `pub enum Outcome { Running, Finished, Failed, Blocked, Interrupted }` (text as in Global Constraints, `as_str()`)
  - `pub struct RunCounts { pub checked: u64, pub updated: u64, pub kept: u64, pub removed: u64 }` (Default)
  - `pub struct RunRecord { pub id: i64, pub trigger: Trigger, pub outcome: Outcome, pub started_at: i64, pub finished_at: Option<i64>, pub counts: RunCounts, pub error: Option<String> }`
  - `pub const RUN_HISTORY_LIMIT: usize = 100;`
  - `SqliteStore::start_run(&self, trigger: Trigger, started_at: i64) -> Result<i64, StoreError>` (prunes to the newest `RUN_HISTORY_LIMIT` rows, never a `running` row)
  - `SqliteStore::finish_run(&self, id: i64, outcome: Outcome, finished_at: i64, counts: RunCounts, error: Option<&str>) -> Result<(), StoreError>`
  - `SqliteStore::interrupt_running_runs(&self) -> Result<usize, StoreError>`
  - `async SqliteStore::recent_runs(&self, limit: usize) -> Result<Vec<RunRecord>, StoreError>` (newest first, via the read pool)

Schema: `runs(id INTEGER PK, trigger TEXT NOT NULL, outcome TEXT NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER, checked, updated, kept, removed INTEGER NOT NULL DEFAULT 0, error TEXT) STRICT`;
`parse_failures(title TEXT PRIMARY KEY, failed_revision INTEGER NOT NULL, first_seen INTEGER NOT NULL) STRICT`.

- [ ] **Step 1:** test `started_run_reads_back_as_running` → `start_run(Manual, 10)`; `recent_runs(10)` == one record, outcome Running, finished_at None. Run `cargo test -p pvxwiki-store --test runs`, expect FAIL (missing items). Stub, see assertion fail, implement, pass.
- [ ] **Step 2:** test `finished_run_keeps_counts_outcome_and_error` → `finish_run(id, Failed, 20, RunCounts{checked:3,..}, Some("boom"))`; record matches. Red, green.
- [ ] **Step 3:** test `interrupt_marks_only_running_rows` → two runs, one finished; `interrupt_running_runs()` == 1; outcomes Finished and Interrupted. Red, green.
- [ ] **Step 4:** test `history_keeps_the_newest_runs_and_never_the_running_one` (Review Focus 5) → 101 finished runs, `start_run` once more; `recent_runs(200).len() == RUN_HISTORY_LIMIT`, newest is Running, oldest id 1 is gone. Red, green.
- [ ] **Step 5:** `cargo test -p pvxwiki-store` passes (including the updated `user_version` 2 assertion).
- [ ] **Step 6: Commit** `feat(store): run history table`

---

### Task 3: Store: pages needing attention and index counts

**Files:**
- Create: `crates/store/src/attention.rs`
- Modify: `crates/store/src/sqlite.rs` (`remove()` also deletes the title's `parse_failures` row), `crates/store/src/lib.rs`
- Test: `crates/store/tests/attention.rs`

**Interfaces:**
- Produces:
  - `pub struct ParseFailure { pub title: String, pub stored_revision: i64, pub failed_revision: i64, pub first_seen: i64 }`
  - `pub struct IssuePage { pub title: String, pub revision: i64 }`
  - `pub const ISSUE_LIST_LIMIT: usize = 50;`
  - `SqliteStore::record_parse_failure(&self, title: &str, failed_revision: i64, seen_at: i64) -> Result<(), StoreError>` (upsert; keeps `first_seen`, updates `failed_revision`)
  - `async SqliteStore::parse_failures(&self) -> Result<Vec<ParseFailure>, StoreError>` (joined with `pages` for `stored_revision`, ordered by title)
  - `async SqliteStore::namespace_counts(&self) -> Result<Vec<(i64, u64)>, StoreError>`
  - `async SqliteStore::issue_counts(&self) -> Result<Vec<(String, u64)>, StoreError>` (pages per issue kind, via `json_each(document, '$.issues')`)
  - `async SqliteStore::pages_with_issue(&self, kind: &str) -> Result<(Vec<IssuePage>, u64), StoreError>` (at most `ISSUE_LIST_LIMIT`, ordered by title; second value is the total)

- [ ] **Step 1:** test `recorded_failure_lists_stored_and_failed_revision` → write page A rev 1, `record_parse_failure("A", 2, 50)`; list == `[ParseFailure{ "A", 1, 2, 50 }]`. Red, green.
- [ ] **Step 2:** test `writing_or_deleting_the_page_clears_its_failure` → write A at rev 3 clears; record for B then `delete_page("B")` clears. Red, green.
- [ ] **Step 3:** test `namespace_counts_split_build_and_archive` → two pages ns 100, one ns 104 → `[(100, 2), (104, 1)]`. Red, green.
- [ ] **Step 4:** test `issue_counts_and_capped_issue_list` → 51 pages with `{{` × 300 (too_deeply_nested) and one with an unknown profession; `issue_counts` == `[("too_deeply_nested", 51), ("unknown_profession", 1)]` (sorted by kind); `pages_with_issue("too_deeply_nested")` returns 50 pages and total 51. Red, green.
- [ ] **Step 5:** `cargo test -p pvxwiki-store` passes. **Commit** `feat(store): parse failures and attention queries`

---

### Task 4: Scraper: `Runner`

**Files:**
- Create: `crates/scraper/src/runner.rs`
- Modify: `crates/scraper/src/run.rs` (progress, trigger on the span, records parse failures), `crates/scraper/src/lib.rs`
- Test: `crates/scraper/tests/runner.rs`; existing `crates/scraper/tests/run.rs` keeps passing

**Interfaces:**
- Consumes: Task 2 and 3 store methods.
- Produces:
  - `pub enum Phase { Listing, Fetching }`, `pub struct Progress { pub phase: Phase, pub done: usize, pub total: usize }`
  - `run(client: &mut ApiClient, store: &SqliteStore, progress: &dyn Fn(Progress)) -> Result<RunReport, ScrapeError>`; calls `store.record_parse_failure` for each kept page. Listing reports `done` = namespaces listed of 2; fetching reports pages fetched of `fetch.len()`.
  - `pub enum ScraperState { Off, Idle, Running { trigger: Trigger, started_at: i64, progress: Option<Progress> }, Blocked { since: i64 } }`
  - `pub struct RunnerStatus { pub state: ScraperState, pub next_run_at: Option<i64> }`
  - `pub enum RunRefused { Off, Running, Blocked }` (`thiserror`, messages name the reason; `Off` mentions `PVXWIKI_CONTACT`)
  - `#[derive(Clone)] pub struct Runner` with:
    - `Runner::new(store: Arc<SqliteStore>, client: Option<ApiClient>, interval: Duration) -> Runner`
    - `Runner::status(&self) -> RunnerStatus`
    - `Runner::run_now(&self) -> Result<(), RunRefused>` (checks and marks the request under one lock)
    - `Runner::run_forever(self)` async: calls `store.interrupt_running_runs()`, runs at once, then on every `interval` tick (`MissedTickBehavior::Skip`, so ticks during a run are skipped) or manual request; each run wrapped in `start_run`/`finish_run`; Blocked/NotJson → state Blocked, row `blocked`, loop keeps serving status but runs nothing; other errors → row `failed` with the error text, state Idle. Returns immediately when the client is `None` (state Off).
- Removes: `scrape_forever` from the bin (Task 7 switches to `Runner`). Move its 403 test here.

- [ ] **Step 1:** test `startup_run_is_recorded_as_scheduled_and_finished` (fake wiki, 1 page) → spawn `run_forever`; poll `recent_runs` until outcome Finished; trigger Scheduled, counts.checked 1. Red, green.
- [ ] **Step 2:** test `a_blocked_runner_stops_running_and_refuses_manual_runs` → 403 wiki; wait for Blocked; exactly 1 request received after 200 ms with a 10 ms interval; `run_now()` == `Err(RunRefused::Blocked)`. Red, green.
- [ ] **Step 3:** test `manual_run_while_running_is_refused` → listing delayed 500 ms; after startup run begins `run_now()` == `Err(Running)`; after it finishes `run_now()` is Ok and a Manual row appears. Red, green.
- [ ] **Step 4:** test `a_failed_run_is_recorded_and_the_runner_returns_to_idle` (Review Focus 1) → wiki 404 on listing (ScrapeError::Status); row Failed with error; state Idle. Red, green.
- [ ] **Step 5:** test `without_a_client_the_runner_is_off` → `run_now()` == `Err(Off)`; status Off. Red, green.
- [ ] **Step 6:** test `interrupted_rows_are_closed_at_startup` → `start_run` before spawning; after startup the old row is Interrupted. Red, green.
- [ ] **Step 7:** test in `run.rs` `a_kept_page_is_recorded_as_a_parse_failure` → extend the existing keep test; `parse_failures()` == `[("Build:A", 1, 2, _)]`. Red, green.
- [ ] **Step 8:** `cargo test -p pvxwiki-scraper` passes. **Commit** `feat(scraper): runner with history, progress and manual runs`

---

### Task 5: Admin crate skeleton: guards and assets

**Files:**
- Create: `crates/admin/Cargo.toml` (`pvxwiki-admin`; deps axum, maud `0.27` features `axum`, tower, tower-http, tokio, tracing, pvxwiki-store, pvxwiki-scraper; workspace lints), `crates/admin/src/lib.rs`, `crates/admin/src/guard.rs`
- Create: `crates/admin/assets/htmx.min.js` (2.0.4), `crates/admin/assets/pico.min.css` (2.0.6), `crates/admin/assets/LICENSES.md` (htmx 0BSD text, Pico MIT text, source URLs and versions)
- Modify: `Cargo.toml` (workspace member dep `pvxwiki-admin`, `maud = { version = "0.27", features = ["axum"] }`)
- Test: `#[cfg(test)] mod tests` in `guard.rs` and `lib.rs`, using `tower::ServiceExt::oneshot`

**Interfaces:**
- Produces:
  - `pub struct AdminState { pub runner: Runner, pub store: Arc<SqliteStore>, pub allowed_hosts: Vec<String> }` (Clone)
  - `pub fn router(state: AdminState) -> Router` with, so far, `GET /assets/htmx.min.js`, `GET /assets/pico.min.css` (correct content types, `cache-control: public, max-age=86400`), host guard on every route, `RequestBodyLimitLayer(MAX_BODY_BYTES)`, `TimeoutLayer(REQUEST_TIMEOUT)`, `TraceLayer`.
  - `fn host_allowed(host: &str, allowed: &[String]) -> bool` (strips `:port`; IPv6 `[::1]` form handled; `localhost`, `127.0.0.1` always allowed)
  - `fn same_origin_htmx(headers: &HeaderMap) -> bool`

- [ ] **Step 1:** unit tests for `host_allowed`: `"localhost:8081"` true; `"admin.example.org:8443"` with `["admin.example.org"]` true (Review Focus 4); `"evil.example"` false; `"127.0.0.1"` true. Red, green.
- [ ] **Step 2:** unit tests for `same_origin_htmx`: HX-Request true + same-origin true; HX-Request true + no Sec-Fetch-Site true; cross-site false; same-site false; missing HX-Request false. Red, green.
- [ ] **Step 3:** router tests: `GET /assets/htmx.min.js` 200 `text/javascript`; with `Host: evil.example` 421. Red, green.
- [ ] **Step 4:** `cargo test -p pvxwiki-admin`, `cargo deny check` pass. **Commit** `feat(admin): admin crate with host guard and vendored assets`

---

### Task 6: Admin views and endpoints

**Files:**
- Create: `crates/admin/src/views.rs` (maud), `crates/admin/src/handlers.rs`
- Modify: `crates/admin/src/lib.rs`
- Test: `#[cfg(test)] mod tests` in `views.rs` (pure render functions), router tests in `lib.rs`

**Interfaces:**
- Consumes: `Runner::status/run_now`, store `recent_runs`, `parse_failures`, `namespace_counts`, `issue_counts`, `pages_with_issue`.
- Produces routes:
  - `GET /` full page: `<head>` loads `/assets/pico.min.css` and `/assets/htmx.min.js`; sections Status (with the `#status` fragment), History, Pages needing attention.
  - `GET /status` fragment: state, reason when off or blocked, progress (`<progress value done max total>` and phase), next run time, namespace counts, the Run now button (`hx-post="/runs" hx-target="#status"`, `disabled` with the reason when refused). Polls itself with `hx-trigger` = `PROGRESS_POLL`.
  - `POST /runs`: 403 unless `same_origin_htmx`; then `run_now()`: Ok → 202 with the status fragment; `Err` → 409 with the status fragment. Logs `info!(trigger = "manual", "manual run requested")`.
  - `GET /issues?kind=<kind>` fragment: the capped list with "and N more" when total > `ISSUE_LIST_LIMIT`; unknown kind → empty list.
  - Render fns: `pub(crate) fn page(...) -> Markup`, `status(&RunnerStatus, &[(i64,u64)]) -> Markup`, `history(&[RunRecord]) -> Markup`, `attention(&[ParseFailure], &[(String,u64)], &str, &[IssuePage], u64) -> Markup`; `fn wiki_url(title: &str) -> String`, `fn revision_url(revision: i64) -> String`.

- [ ] **Step 1:** test `wiki_url_encodes_titles` (Review Focus 3): `wiki_url("Build:A/Me \"Q&A\" #1?")` == `"https://gwpvx.fandom.com/wiki/Build:A/Me_%22Q%26A%22_%231%3F"` (keep `/` and `:`). Red, green.
- [ ] **Step 2:** test `attention_escapes_titles` → title `<script>` renders as `&lt;script&gt;`. Red, green.
- [ ] **Step 3:** test `status_disables_run_now_with_the_reason` for Off (contains `PVXWIKI_CONTACT`), Running, Blocked; enabled for Idle. Red, green.
- [ ] **Step 4:** test `history_shows_outcome_trigger_counts_and_error`. Red, green.
- [ ] **Step 5:** test `issue_list_says_how_many_more`: total 51, 50 shown → contains "and 1 more". Red, green.
- [ ] **Step 6:** router tests: `POST /runs` cross-site → 403; Off runner → 409; `GET /` 200 `text/html` containing "Run history". Red, green.
- [ ] **Step 7:** `cargo test -p pvxwiki-admin` passes. **Commit** `feat(admin): status, history and attention views`

---

### Task 7: Bin wiring, deploy and docs (acceptance goes green)

**Files:**
- Modify: `crates/pvxwiki-mcp/src/config.rs` (`admin_listen: SocketAddr` from `PVXWIKI_ADMIN_LISTEN`, default `0.0.0.0:8081`), `crates/pvxwiki-mcp/src/lib.rs`, `crates/pvxwiki-mcp/src/main.rs`, `crates/pvxwiki-mcp/Cargo.toml`, `crates/pvxwiki-mcp/tests/acceptance.rs` (new `run` signature)
- Modify: `deploy/compose.yaml`, `deploy/Caddyfile`; create `deploy/caddy/mcp-on.caddy`, `deploy/caddy/mcp-off.caddy`, `deploy/caddy/admin-on.caddy`, `deploy/caddy/admin-off.caddy`
- Modify: `README.md`, `.claude/rules/security.md`, `deploy/.env.example` (Claude cannot read this file: hand the exact lines to the user to add)

**Interfaces:**
- Consumes: `Runner`, `pvxwiki_admin::{router, AdminState}`.
- Produces: `pub async fn run(config: Config, listener: TcpListener, admin_listener: TcpListener, limits: Limits, schedule: Schedule, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), RunError>`; both servers stop on the same shutdown (share it via `tokio::sync::watch` or `CancellationToken`); `scrape_forever` and its test removed (moved to Task 4).

- [ ] **Step 1:** test in `config.rs` tests: `PVXWIKI_ADMIN_LISTEN` unset → `0.0.0.0:8081`; invalid → `ConfigError::Invalid`. Red, green.
- [ ] **Step 2:** wire `run`: `Runner::new(store, client, schedule.interval)`, spawn `run_forever`, serve MCP router and admin router on their listeners with graceful shutdown. `main` binds `config.admin_listen`.
- [ ] **Step 3:** Run `cargo test -p pvxwiki-mcp --test admin` → all Task 1 tests PASS. Run `--test acceptance` → PASS. If an acceptance test fails, fix the code, not the test.
- [ ] **Step 4:** Deploy: compose publishes `"${PVXWIKI_ADMIN_PUBLISH_ADDR:-127.0.0.1}:8081:8081"`. `Caddyfile` becomes
  ```
  import caddy/mcp-{$PVXWIKI_CADDY_MCP:on}.caddy
  import caddy/admin-{$PVXWIKI_CADDY_ADMIN:off}.caddy
  ```
  `mcp-on.caddy` holds today's site; `admin-on.caddy` is `{$PVXWIKI_ADMIN_DOMAIN} { basic_auth { {$PVXWIKI_ADMIN_USER} {$PVXWIKI_ADMIN_PASSWORD_HASH} } reverse_proxy pvxwiki-mcp:8081 }`; the `-off` files are empty. Mount `./caddy` read-only and pass the new variables to the caddy service. Verify: `docker run --rm -v $PWD/deploy:/etc/caddy -e PVXWIKI_DOMAIN=a.example -e PVXWIKI_CADDY_ADMIN=on -e PVXWIKI_ADMIN_DOMAIN=b.example -e PVXWIKI_ADMIN_USER=u -e PVXWIKI_ADMIN_PASSWORD_HASH='$2a$14$...' caddy:2.10 caddy adapt --config /etc/caddy/Caddyfile` succeeds, and with defaults (admin off) adapts without admin variables; with `PVXWIKI_CADDY_MCP=off` and admin on it adapts without `PVXWIKI_DOMAIN`.
- [ ] **Step 5:** Docs: README "Admin page" section (what it shows; reach it at `127.0.0.1:8081`; protect it with `PVXWIKI_CADDY_ADMIN=on` and the `https` profile, or a second resource in your own proxy with SSO; list its host in `PVXWIKI_ALLOWED_HOSTS`), config table rows for `PVXWIKI_ADMIN_LISTEN`, `PVXWIKI_ADMIN_PUBLISH_ADDR`, `PVXWIKI_CADDY_MCP`, `PVXWIKI_CADDY_ADMIN`, `PVXWIKI_ADMIN_DOMAIN`, `PVXWIKI_ADMIN_USER`, `PVXWIKI_ADMIN_PASSWORD_HASH`; crate list gains `crates/admin`. `security.md` gains under Abuse resistance: "The admin page has its own listener, published on 127.0.0.1 by default; its protection lives in the reverse proxy (Caddy basic auth or the operator's SSO), never in the service." CLAUDE.md Stack line gains `crates/admin` (maud + htmx admin page).
- [ ] **Step 6: Full verification** `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build`, `cargo deny check`; show output.
- [ ] **Step 7: Commit** `feat(admin): serve the admin page on its own listener`; give the user the `.env.example` lines.
