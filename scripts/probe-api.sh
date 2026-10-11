#!/usr/bin/env bash
# One-off probe: can this host use the PvX wiki's MediaWiki API politely?
# Run it on the server that will host the scraper, never from CI or tests.
#   CONTACT=you@example.org scripts/probe-api.sh
# Stops at the first 403, non-JSON answer, or API error (.claude/rules/scraping.md).
set -euo pipefail

: "${CONTACT:?set CONTACT to an email or URL Fandom can reach you at}"
API=https://gwpvx.fandom.com/api.php
UA="pvxwiki-mcp-probe/0.1 (https://github.com/maikbasel/pvxwiki-mcp; ${CONTACT})"
DELAY_SECS=2       # one request at a time, at least 2 s apart
MAX_LIST_PAGES=20  # caps the title count at 20 x 500
BATCH=50           # non-bot limit for titles per query
OUT=$(mktemp -d -t pvx-probe.XXXX)
requests=0

# api NAME PARAM=VALUE...: GET with maxlag, saves $OUT/NAME.{json,headers}, stops on any failure.
api() {
  local name=$1 args=() status
  shift
  for p in format=json formatversion=2 maxlag=5 "$@"; do args+=(--data-urlencode "$p"); done
  sleep "$DELAY_SECS"
  requests=$((requests + 1))
  status=$(curl -sS -G -A "$UA" -D "$OUT/$name.headers" -o "$OUT/$name.json" \
    -w '%{http_code}' "${args[@]}" "$API")
  echo "$name: HTTP $status"
  if [[ $status != 200 ]]; then
    grep -i '^retry-after' "$OUT/$name.headers" || true
    echo "STOP: non-200. Do not retry through it. Output in $OUT"
    exit 1
  fi
  if ! jq -e . "$OUT/$name.json" >/dev/null 2>&1; then
    echo "STOP: not JSON (challenge page?):"
    head -c 300 "$OUT/$name.json"
    exit 1
  fi
  if jq -e .error "$OUT/$name.json" >/dev/null; then
    jq .error "$OUT/$name.json"
    grep -i '^retry-after' "$OUT/$name.headers" || true
    echo "STOP: API error. Output in $OUT"
    exit 1
  fi
  jq -c '.warnings // empty' "$OUT/$name.json"
}

echo "== robots.txt"
curl -sS -A "$UA" -o "$OUT/robots.txt" https://gwpvx.fandom.com/robots.txt
requests=$((requests + 1))
if grep -qi '<html' "$OUT/robots.txt"; then
  echo "robots.txt answered with an HTML (challenge) page, not rules. Read it in a browser."
else
  grep -niE '^user-agent|api\.php' "$OUT/robots.txt" || echo "(no api.php rules)"
fi

echo "== siteinfo"
api siteinfo action=query meta=siteinfo 'siprop=general|namespaces|rightsinfo|statistics'
jq -r '.query.general | "\(.sitename), \(.generator)"' "$OUT/siteinfo.json"
jq -r '.query.rightsinfo | "licence: \(.text) <\(.url)>"' "$OUT/siteinfo.json"
ns=$(jq -r '[.query.namespaces[] | select(.canonical == "Build" or .name == "Build") | .id][0] // empty' \
  "$OUT/siteinfo.json")
if [[ -z $ns ]]; then
  echo "STOP: no Build namespace. Namespaces:"
  jq -r '.query.namespaces[] | "\(.id) \(.name)"' "$OUT/siteinfo.json"
  exit 1
fi
echo "Build namespace id: $ns"

echo "== build page titles"
: >"$OUT/titles.txt"
cont=()
for ((i = 1; i <= MAX_LIST_PAGES; i++)); do
  api "allpages-$i" action=query list=allpages "apnamespace=$ns" aplimit=max \
    apfilterredir=nonredirects "${cont[@]}"
  jq -r '.query.allpages[].title' "$OUT/allpages-$i.json" >>"$OUT/titles.txt"
  next=$(jq -r '.continue.apcontinue // empty' "$OUT/allpages-$i.json")
  [[ -z $next ]] && break
  cont=("apcontinue=$next")
done
total=$(wc -l <"$OUT/titles.txt")
echo "build pages: $total$([[ -n $next ]] && echo " (capped, more exist)")"

echo "== wikitext for $BATCH titles in one request"
titles=$(head -n "$BATCH" "$OUT/titles.txt" | paste -sd '|')
api revisions action=query prop=revisions 'rvprop=ids|timestamp|content' rvslots=main "titles=$titles"
jq -r '[.query.pages[] | select(.revisions[0].slots.main.content)] | "pages with wikitext: \(length)"' \
  "$OUT/revisions.json"
echo "response size: $(wc -c <"$OUT/revisions.json") bytes"

echo "== recent changes in Build namespace"
api recentchanges action=query list=recentchanges "rcnamespace=$ns" rclimit=50 \
  'rcprop=title|ids|timestamp' 'rctype=edit|new'
jq -r '.query.recentchanges | "changes returned: \(length), oldest: \(.[-1].timestamp // "none")"' \
  "$OUT/recentchanges.json"

echo "== summary"
echo "requests made: $requests"
echo "full crawl estimate: $(((total + 499) / 500)) list + $(((total + BATCH - 1) / BATCH)) wikitext requests"
echo "raw responses: $OUT"
