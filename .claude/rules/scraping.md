# Scraping the wiki

Always loaded. The goal is to work within Fandom's rules, not around its bot protection. Getting
this wrong gets the server's IP blocked and ends the project.

## Required

- Use the MediaWiki Action API only: `https://gwpvx.fandom.com/api.php`, with `format=json` and
  `formatversion=2`. Do not scrape rendered `/wiki/` HTML.
- Send an honest, identifying User-Agent: tool name, version, repository URL and a contact address
  taken from config. Never a browser User-Agent. The contact is the GitHub repository URL once the
  repo exists; until then, the maintainer's email.
- One request at a time, with a fixed minimum delay between requests (a named const).
- Send `maxlag=5`. On a `maxlag` error, 429 or 503, honour `Retry-After`, back off exponentially
  with a cap, and abort the run after a bounded number of retries. The next scheduled run tries again.
- Batch: request up to 50 titles per call (the non-bot limit).
- Refetch only what changed: compare revision IDs (`prop=info` / `list=recentchanges`) and skip
  pages whose revision is already stored.
- Respect `robots.txt` and Fandom's terms for the paths we call. Checked by hand 2026-10-09: the
  `*` group allows `/api.php?action=` and `/api.php?*&action=`, has no `Crawl-delay`, and disallows
  only `/wiki/` HTML paths. Named AI crawlers (GPTBot, ClaudeBot, CCBot, xAI bots) are disallowed
  entirely; never send their User-Agents. The server cannot fetch `robots.txt` itself (Cloudflare
  challenge), so recheck it in a browser before calling any new path.

## Forbidden

- Anything that evades bot protection: rotating IPs or User-Agents, proxies, headless browsers to pass
  challenges, solving CAPTCHAs, retrying through a 403.
- Calling the wiki from the MCP request path, from tests, or from CI.

If the API answers with 403 or a challenge page, stop the run, log it, and tell the user. Do not try
to work around it.
