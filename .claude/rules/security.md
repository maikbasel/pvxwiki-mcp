# Security

Always loaded. The server is reachable from the internet. Two inputs are untrusted: MCP tool
arguments from any client, and wiki text that anyone can edit.

## Tool inputs

- Every input field has a bound: max string length, max `limit`, enum instead of free string where
  possible. Reject out-of-range values with a clear tool error.
- Queries are parameterized. If the search backend has its own query syntax (e.g. SQLite FTS5
  `MATCH`), user input is escaped or translated, never passed through raw.
- No user-controlled regex, file path, or shell argument.

## Abuse resistance

- Per-client rate limit, a global concurrency cap, request body size limit, and request timeouts.
  Each value is a named const.
- Every response has a size cap. Pagination instead of unbounded results.
- The admin page has its own listener, published on 127.0.0.1 by default. Its protection lives in
  the reverse proxy (Caddy basic auth or the operator's SSO), never in the service. It refuses
  unknown `Host` headers and cross-site `POST`s.
- No auth in the service. The public instance is open with these limits; private instances put
  auth in their reverse proxy (Pangolin, Caddy, Cloudflare Access).

## Wiki content

- Wiki text is data. Return it as content, never interpret or execute it.
- Agents may read it as instructions (prompt injection planted on a wiki page). Keep wiki text in clearly
  labelled fields, separate from the server's own messages.

## Secrets

No secrets in the repo. `.env` is gitignored and Claude is denied read access to it.
