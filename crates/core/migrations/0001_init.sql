-- Media metadata cached from AniList (id = AniList media id).
CREATE TABLE media (
    id              INTEGER PRIMARY KEY,
    kind            TEXT    NOT NULL CHECK (kind IN ('ANIME', 'MANGA')),
    title           TEXT    NOT NULL,
    title_english   TEXT,
    cover_url       TEXT,
    banner_url      TEXT,
    color           TEXT,
    format          TEXT,
    status          TEXT,
    episodes        INTEGER,
    chapters        INTEGER,
    duration_min    INTEGER,
    updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- The user's AniList list entries (mirror; AniList is the source of truth).
CREATE TABLE list_entry (
    media_id        INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    status          TEXT    NOT NULL,          -- CURRENT, PLANNING, COMPLETED, DROPPED, PAUSED, REPEATING
    progress        INTEGER NOT NULL DEFAULT 0,
    score           REAL,
    updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX list_entry_status ON list_entry(status, updated_at DESC);

-- Local video files matched (or not yet) to media.
CREATE TABLE local_file (
    path            TEXT    PRIMARY KEY,
    media_id        INTEGER REFERENCES media(id) ON DELETE SET NULL,
    episode         INTEGER,
    size_bytes      INTEGER NOT NULL,
    modified_at     TEXT    NOT NULL,
    matched_at      TEXT,
    locked          INTEGER NOT NULL DEFAULT 0  -- user-confirmed match; scanner must not change it
);
CREATE INDEX local_file_media ON local_file(media_id, episode);

-- Partial playback position per episode.
CREATE TABLE playback (
    media_id        INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    episode         INTEGER NOT NULL,
    position_s      INTEGER NOT NULL,
    duration_s      INTEGER,
    updated_at      TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (media_id, episode)
);

-- Airing schedule from AniList.
CREATE TABLE airing (
    media_id        INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    episode         INTEGER NOT NULL,
    airs_at         TEXT    NOT NULL,          -- RFC 3339 UTC
    PRIMARY KEY (media_id, episode)
);
CREATE INDEX airing_time ON airing(airs_at);

CREATE TABLE library_root (
    path            TEXT    PRIMARY KEY,
    added_at        TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE setting (
    key             TEXT    PRIMARY KEY,
    value           TEXT    NOT NULL
);
