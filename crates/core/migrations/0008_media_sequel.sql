-- Each show's direct TV sequel (NULL = none), for episodes numbered past a season's end.
CREATE TABLE media_sequel (
    media_id    INTEGER PRIMARY KEY,
    sequel_id   INTEGER,
    checked_at  TEXT NOT NULL
);
