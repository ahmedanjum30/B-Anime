-- Installed extensions. The script itself lives in <data dir>/extensions/<id>.js.
CREATE TABLE extension (
    id            TEXT    PRIMARY KEY,
    manifest      TEXT    NOT NULL,
    source_url    TEXT    NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    installed_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT    NOT NULL DEFAULT (datetime('now'))
);
