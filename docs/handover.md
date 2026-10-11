# Handover: PvX Build MCP Server

Original brief, kept verbatim. Decisions made since are recorded in `CLAUDE.md` and `.claude/rules/`.

## What I want to build
A service that lets AI agents look up Guild Wars 1 builds from the PvX wiki (https://gwpvx.fandom.com/wiki/PvX_wiki). Agents should be able to get a build's full details, including its template code and a link back to the wiki.

**Everything on a build page should be searchable, not just the main skill bar.** Pages vary, and whatever sections and boxes a page has should be captured, not just a fixed set. Examples of what appears on pages:
- the rating (e.g. Great) and what the build is designed for (e.g. PvE general, team, hero)
- the description
- attributes and skills
- all variants, including off-meta or niche variants, each with its own skill bar
- optional skills
- equipment (armor, runes, insignias, weapons)
- usage
- ally hero / party synergy
- counters
- notes
- for team builds, the team composition and the alterations based on the player's build

So an agent can ask things like:
- "a PvE hero team that works with a dagger player"
- "builds that use Death Blossom with a Zealous dagger"
- "which builds require, or are improved by, an 'of the Profession' weapon mod"

Answers should keep the page's nuance, e.g. whether something is **required** or just **recommended / an improvement**, rather than flattening everything into simple keyword matches.

## Why
The PvX wiki is the main collection of community-vetted GW1 builds, but agents can't use it well today. The pages are raw wiki markup, Fandom limits automated access, and generic wiki tools return whole pages instead of usable build data. A dedicated service turns the wiki into clean, structured, searchable build information.

## How, at a high level
- Copy the wiki's build pages into my own small database on a regular schedule, instead of querying the wiki live. This keeps answers fast, avoids hammering Fandom, and keeps the service working if the wiki is slow or blocks requests.
- Expose that database to agents as an MCP server.
- Host it myself, running permanently in my homelab and reachable from outside.
- Make it robust so nobody can overwhelm the server.
- Credit the PvX wiki as the source, since its content is CC-BY-SA licensed.

## Out of scope
A general GW1 skill database, and decoding or validating template codes. The build pages already contain what this service needs. If an agent needs skill details or code decoding, it can use a separate tool for that (e.g. the existing gw1-mcp).

## Open questions
- **How to get the data in a proper way.** The site appears to have bot protection: an earlier automated test was rejected, and parts of the site disallow bots. Dumps, exports and asking the wiki admins are not options. The plan is to use the wiki's official API, used politely and within Fandom's terms. The first task, before building anything else, is to confirm this works reliably from the server the service will run on.

  The goal is to work with the site's rules, not to get around its bot protection. How well this works also decides how often the data can be refreshed.
- Should the service be open to anyone or require a token?
