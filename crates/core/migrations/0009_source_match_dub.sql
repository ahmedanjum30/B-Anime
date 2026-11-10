-- Sub and dub versions of a show are separate entries on most sources: match them separately.
CREATE TABLE source_match_new (
    media_id      INTEGER NOT NULL,
    ext_id        TEXT    NOT NULL,
    dub           INTEGER NOT NULL DEFAULT 0,
    source_id     TEXT    NOT NULL,
    source_title  TEXT    NOT NULL,
    auto          INTEGER NOT NULL DEFAULT 1,
    updated_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (media_id, ext_id, dub)
);
INSERT INTO source_match_new (media_id, ext_id, dub, source_id, source_title, auto, updated_at)
    SELECT media_id, ext_id, 0, source_id, source_title, auto, updated_at FROM source_match;
DROP TABLE source_match;
ALTER TABLE source_match_new RENAME TO source_match;
