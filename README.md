# pvxwiki-mcp

[![CI](https://github.com/maikbasel/pvxwiki-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/maikbasel/pvxwiki-mcp/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/code-MIT-blue.svg)](LICENSE)
[![Content: CC BY-NC-SA 2.5](https://img.shields.io/badge/content-CC%20BY--NC--SA%202.5-lightgrey.svg)](https://creativecommons.org/licenses/by-nc-sa/2.5/)

An [MCP](https://modelcontextprotocol.io) server that lets AI agents search Guild Wars 1 builds
from the [PvX wiki](https://gwpvx.fandom.com).

A scheduled job copies build pages through the wiki's MediaWiki API into a local SQLite database:
once at startup, then every 6 hours, fetching only pages whose revision changed.
The server answers only from that database and never calls the wiki while serving a request. Every
answer credits the PvX wiki contributors and links the page revision it came from.

## Tools

| Tool | What it does |
|------|--------------|
| `search_builds` | Full-text search over build pages, filtered by skills, professions, gameplay types and ratings. Returns short results with matching snippets and a cursor for the next page. |
| `get_build` | One build page in full: vetting, every section as wikitext, and every skill bar with attributes, skills and variants. |

<details>
<summary><code>search_builds</code> parameters</summary>

| Parameter | Type | Description |
|-----------|------|-------------|
| `query` | string | Words to find on a build page (max 200 characters). Every word must appear in the same section. |
| `skills` | string[] | Skill names. A build matches when one of its skill bars has all of them (max 8). |
| `professions` | string[] | Primary profession of any skill bar on the page. |
| `gameplay_types` | string[] | For example `farming`, `HA`, `GvG`. The schema lists the allowed values. |
| `ratings` | string[] | Default: every rating except archived. |
| `include_archived` | boolean | Include archived (outdated) builds. Default `false`. |
| `limit` | integer | Results per page, 1 to 25. Default 10. |
| `cursor` | string | `next_cursor` from a previous response. |

</details>

<details>
<summary><code>get_build</code> parameters</summary>

| Parameter | Type | Description |
|-----------|------|-------------|
| `title` | string | Exact page title (`Build:A/Me Vaettir Farm`) or its gwpvx.fandom.com URL. |

</details>

Each result carries an `attribution` object with the credit, the licence name and the licence URL,
next to the page `url` and `revision`.

## Connect a client

The server speaks MCP over streamable HTTP at `/mcp`. There is no public instance yet, so use the
URL of one you run: `https://<your domain>/mcp` behind a proxy, or `http://localhost:8080/mcp` for
one on the same machine ([Run it locally](#run-it-locally)).

**Claude Code**

```bash
claude mcp add --transport http pvxwiki https://pvxwiki.example.org/mcp
```

**Clients with a JSON config** (Claude Desktop, Cursor, VS Code and others; the key names vary
slightly between clients)

```json
{
  "mcpServers": {
    "pvxwiki": {
      "type": "http",
      "url": "https://pvxwiki.example.org/mcp"
    }
  }
}
```

## Self-hosting

> [!IMPORTANT]
> Fandom's Terms of Use forbid automated access without written permission. This project's
> permission covers only its own instance. If you run your own, ask Fandom support for permission
> first, and keep the scraper's request delay as it is or slower.

You need Docker with the compose plugin.

```bash
git clone https://github.com/maikbasel/pvxwiki-mcp.git
cd pvxwiki-mcp/deploy
cp .env.example .env    # then edit .env
docker compose up -d
```

The first scrape starts right away. It sends one request every 2 seconds, so a fresh instance takes
a few minutes before it has builds; until then the tools answer "index not built yet". If Fandom
answers with a block (HTTP 403 or a challenge page), the scraper stops until you restart the
container. Check the logs before restarting.

The server listens on `127.0.0.1:8080`. To serve it publicly over HTTPS with automatic
certificates, set `PVXWIKI_DOMAIN` in `.env` and start the bundled Caddy:

```bash
docker compose --profile https up -d
```

Already running a reverse proxy (Traefik, Pangolin, nginx)? Skip the profile, point the proxy at
port 8080 and put its address in `PVXWIKI_TRUSTED_PROXIES`.

### Admin page

The admin page shows the scraper's state and live progress, when the next run starts, the run
history, and pages that need attention: pages whose newest revision failed to parse (the stored copy
stays) and pages stored with parse issues. A **Run now** button starts a run at the same pace as a
scheduled one. It is disabled while a run is going, when `PVXWIKI_CONTACT` is unset, and after the
wiki blocked the scraper (restart the container once you know why).

It listens on port 8081, published on `127.0.0.1` only. The service has no login of its own, so put
one in front before you expose it:

- **Bundled Caddy:** set `PVXWIKI_CADDY_ADMIN=on`, `PVXWIKI_ADMIN_DOMAIN`, `PVXWIKI_ADMIN_USER` and
  `PVXWIKI_ADMIN_PASSWORD_HASH` (create it with `docker run --rm caddy:2.10 caddy hash-password`),
  then start the `https` profile. Put the hash in single quotes in `.env`
  (`PVXWIKI_ADMIN_PASSWORD_HASH='$2a$14$...'`): compose reads `$` in unquoted or double-quoted
  values as a variable, and every login then fails. Set `PVXWIKI_CADDY_MCP=off` if Caddy should serve only the admin
  page.
- **Your own proxy** (Pangolin, Authelia, Cloudflare Access): add a second resource for port 8081
  behind your SSO.

Either way, list the admin host name in `PVXWIKI_ALLOWED_HOSTS`; other host names get HTTP 421.

The service has no authentication. Its abuse limits are a per-client rate limit, a concurrency cap,
a request size limit and request timeouts. For a private instance, put authentication in your
reverse proxy.

### Configuration

All settings are environment variables. [`deploy/.env.example`](deploy/.env.example) explains each
one.

| Variable | Default | Description |
|----------|---------|-------------|
| `PVXWIKI_ALLOWED_HOSTS` | required | Comma-separated host names clients use to reach the server (the `Host` header). |
| `PVXWIKI_CONTACT` | | Contact for the scraper's User-Agent: an email address Fandom can reach you at. Without it the scraper is off and the server only serves what the database already holds. |
| `PVXWIKI_TRUSTED_PROXIES` | empty | IPs or CIDR ranges whose `X-Forwarded-For` header is trusted. |
| `PVXWIKI_PUBLISH_ADDR` | `127.0.0.1` | Interface port 8080 is published on. |
| `PVXWIKI_DOMAIN` | | Public domain for the `https` profile. Also list it in `PVXWIKI_ALLOWED_HOSTS`. |
| `PVXWIKI_DATA_DIR` | `/data` | Where the SQLite database lives. |
| `PVXWIKI_LISTEN` | `0.0.0.0:8080` | Address the server binds inside the container. |
| `PVXWIKI_ADMIN_LISTEN` | `0.0.0.0:8081` | Address the admin page binds inside the container. Outside a container it defaults to `127.0.0.1:8081`. |
| `PVXWIKI_ADMIN_PUBLISH_ADDR` | `127.0.0.1` | Interface port 8081 is published on. Keep it on loopback unless a proxy with a login is in front. |
| `PVXWIKI_CADDY_MCP` | `on` | `off` stops the bundled Caddy from serving the MCP site. |
| `PVXWIKI_CADDY_ADMIN` | `off` | `on` makes the bundled Caddy serve the admin page with basic auth. |
| `PVXWIKI_ADMIN_DOMAIN` | | Public domain of the admin page for the bundled Caddy. Also list it in `PVXWIKI_ALLOWED_HOSTS`. |
| `PVXWIKI_ADMIN_USER` | | Basic-auth user for the admin page. |
| `PVXWIKI_ADMIN_PASSWORD_HASH` | | Bcrypt hash of that user's password, from `caddy hash-password`. |
| `PVXWIKI_LOG_FORMAT` | `json` | `json` or `pretty`. |
| `RUST_LOG` | `info,rmcp=warn` | Log filter. |

## Development

Requires Rust 1.96 or newer.

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo deny check
```

### Run it locally

Run the same container you would deploy, built from your checkout:

```bash
cd deploy
cp .env.example .env    # then edit: PVXWIKI_ALLOWED_HOSTS must include localhost
docker compose up -d --build
```

The MCP server is at `http://localhost:8080/mcp` and the admin page at `http://localhost:8081`.
Without `PVXWIKI_CONTACT` the scraper stays off and nothing calls the wiki: the admin page says so,
and the tools answer "index not built yet". Setting `PVXWIKI_CONTACT` starts a real run against
Fandom at once, so only do that from a machine your permission covers (see
[Self-hosting](#self-hosting)).

For a quicker edit-and-run loop without Docker, `cargo run -p pvxwiki-mcp` works too. It reads
the environment, not `.env`, so set at least `PVXWIKI_ALLOWED_HOSTS=localhost` and
`PVXWIKI_DATA_DIR=./data` (an existing directory).

The workspace has six crates:

- `crates/parser`: wikitext to typed builds, no I/O.
- `crates/store`: SQLite with FTS5 search.
- `crates/server`: the MCP tools and the HTTP layer.
- `crates/admin`: the admin page (maud, htmx and Pico CSS, served from the binary).
- `crates/scraper`: copies changed build pages from the wiki's API into the store, paced and
  retrying.
- `crates/pvxwiki-mcp`: the binary.

Tests never call the wiki. They run against recorded fixtures and a fake `api.php`.

## License

The source code is licensed under the [MIT License](LICENSE).

Build content comes from the [PvX wiki](https://gwpvx.fandom.com), written by the PvX wiki
contributors and licensed under
[CC BY-NC-SA 2.5](https://creativecommons.org/licenses/by-nc-sa/2.5/). Content this server serves,
including parsed fields and search snippets, stays under that licence. NonCommercial applies: no
ads, no paid access and no paid tier, for this instance or yours.

This project is not affiliated with or endorsed by ArenaNet, NCSOFT, Fandom or the PvX wiki. Guild
Wars is a trademark of ArenaNet, LLC.
