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

> [!NOTE]
> **Work in progress.** There is no public instance yet.

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

The server speaks MCP over streamable HTTP at `/mcp`. Replace the URL with your instance.

**Claude Code**

```bash
claude mcp add --transport http pvxwiki http://localhost:8080/mcp
```

**Clients with a JSON config** (Claude Desktop, Cursor, VS Code and others; the key names vary
slightly between clients)

```json
{
  "mcpServers": {
    "pvxwiki": {
      "type": "http",
      "url": "http://localhost:8080/mcp"
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

The workspace has five crates:

- `crates/parser`: wikitext to typed builds, no I/O.
- `crates/store`: SQLite with FTS5 search.
- `crates/server`: the MCP tools and the HTTP layer.
- `crates/scraper`: the paced, retrying client for the wiki's API.
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
