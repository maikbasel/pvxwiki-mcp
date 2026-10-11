# Production-grade

Always loaded. The consumers are AI agents. A swallowed error or an unstable response shape becomes
a silent failure inside the calling agent, which is far harder to debug than the original bug.

## Failure cases that are part of the feature, not v2

- Fandom returns 429, 5xx, a `maxlag` error, or a block page instead of JSON.
- A build page was renamed, deleted, or moved to an archive namespace since the last run.
- A page has a missing section, an unknown template, or malformed wikitext.
- First boot: the database is empty. Tools say the index is not built yet. They never return an empty
  result that reads as "no builds match".
- The scraper is killed mid-run. The next run resumes or restarts cleanly, and readers never see a
  half-written page.

## Forbidden

- `unwrap()` / `expect()` outside tests and outside provably infallible cases with a comment.
- `TODO` / `FIXME` comments. Do the work, or open a GitHub issue and leave the code clean.
- Speculative abstractions: a trait with one implementation, config for a value that never changes.
- Unnamed tuning values. Scrape interval, request delay, rate limits and result caps are named consts.

## Required

- Each failure mode maps to a typed error (`thiserror`) and, at the MCP boundary, to a tool error
  message the agent can act on.
- MCP tool output shapes are a contract. Changing one is a breaking change; say so before doing it.
