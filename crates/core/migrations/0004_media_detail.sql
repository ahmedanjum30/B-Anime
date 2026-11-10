-- Last AniList response for a show page, so pages open instantly and work offline.
CREATE TABLE media_detail (
    media_id    INTEGER PRIMARY KEY,
    json        TEXT    NOT NULL,
    fetched_at  TEXT    NOT NULL
);
