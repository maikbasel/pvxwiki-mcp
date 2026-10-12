-- One row per build page. `document` is the get_build payload (JSON) without attribution.
CREATE TABLE pages (
    id         INTEGER PRIMARY KEY,
    title      TEXT    NOT NULL UNIQUE,
    namespace  INTEGER NOT NULL,
    revision   INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,   -- unix seconds
    vetting    TEXT    NOT NULL,   -- vetted | stub | missing
    rating     TEXT,               -- lowercase
    status     TEXT,               -- lowercase
    archived   INTEGER NOT NULL,   -- 0 or 1
    document   TEXT    NOT NULL
) STRICT;
CREATE INDEX pages_rating ON pages (rating);

CREATE TABLE page_types (
    page_id       INTEGER NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    gameplay_type TEXT    NOT NULL  -- lowercase
) STRICT;
CREATE INDEX page_types_type ON page_types (gameplay_type, page_id);

CREATE TABLE page_skills (
    page_id INTEGER NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    bar     INTEGER NOT NULL,       -- index into the document's skill_bars
    skill   TEXT    NOT NULL        -- lowercase
) STRICT;
CREATE INDEX page_skills_skill ON page_skills (skill, page_id, bar);

CREATE TABLE page_profs (
    page_id    INTEGER NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    profession TEXT    NOT NULL     -- lowercase primary profession of a bar
) STRICT;
CREATE INDEX page_profs_profession ON page_profs (profession, page_id);

-- One row per section. FTS5 tables cannot cascade: write_page deletes them by page_id.
CREATE VIRTUAL TABLE sections_fts USING fts5 (
    title, heading, body,
    page_id UNINDEXED, section UNINDEXED,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
