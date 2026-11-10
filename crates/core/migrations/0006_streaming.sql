-- Which entry on an extension's source corresponds to an AniList show.
CREATE TABLE source_match (
    media_id      INTEGER NOT NULL,
    ext_id        TEXT    NOT NULL,
    source_id     TEXT    NOT NULL,
    source_title  TEXT    NOT NULL,
    auto          INTEGER NOT NULL DEFAULT 1,  -- 0 when the user picked it
    updated_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (media_id, ext_id)
);
