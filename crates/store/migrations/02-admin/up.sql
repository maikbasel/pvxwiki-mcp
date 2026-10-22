-- One row per scrape run, newest RUN_HISTORY_LIMIT kept. `outcome` is running until the run ends.
CREATE TABLE runs (
    id          INTEGER PRIMARY KEY,
    trigger     TEXT    NOT NULL,   -- scheduled | manual
    outcome     TEXT    NOT NULL,   -- running | finished | failed | blocked | interrupted
    started_at  INTEGER NOT NULL,   -- unix seconds
    finished_at INTEGER,
    checked     INTEGER NOT NULL DEFAULT 0,
    updated     INTEGER NOT NULL DEFAULT 0,
    kept        INTEGER NOT NULL DEFAULT 0,
    removed     INTEGER NOT NULL DEFAULT 0,
    error       TEXT
) STRICT;

-- Pages whose newest revision failed to parse; the last good copy stays served.
CREATE TABLE parse_failures (
    title           TEXT    PRIMARY KEY,
    failed_revision INTEGER NOT NULL,
    first_seen      INTEGER NOT NULL
) STRICT;
